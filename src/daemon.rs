// SPDX-License-Identifier: GPL-3.0-or-later

//! `t2l daemon`: owns the camera. One worker thread does all camera access (so frames from
//! every client are serialised and sequence numbers stay unique), executes requests with the
//! CLI's own argument parser and polls the camera to notice changes it makes itself.

use crate::cli::{Cli, Cmd, execute};
use crate::proto::{Hub, Request, Response, socket_path};
use clap::Parser;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::os::fd::FromRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};
use t2l::camera::{CAPABILITIES, Camera};
use t2l::error::{Error, Result};
use t2l::state::{self, State};

const TICK: Duration = Duration::from_secs(1);
/// The 0x0104 / zoom queries and the image controls are read at least this often.
const FULL_EVERY: Duration = Duration::from_secs(5);

struct Job {
    args: Vec<String>,
    reply: Sender<std::result::Result<(Value, String), String>>,
}

/// Parses and runs a request with the CLI code, like a direct call would.
fn run_request(args: &[String]) -> std::result::Result<(Value, String), String> {
    let cli = Cli::try_parse_from(std::iter::once("t2l".to_string()).chain(args.iter().cloned()))
        .map_err(|e| e.to_string())?;
    if matches!(cli.command, Cmd::Daemon | Cmd::Watch) {
        return Err("daemon and watch cannot be run through the daemon".into());
    }
    execute(cli.command).map_err(|e| e.to_string())
}

fn unavailable() -> Value {
    json!({"available": false, "capabilities": CAPABILITIES})
}

/// Differences in these auto-adjusted controls are not a change worth a broadcast.
fn stable(state: &Value) -> Value {
    let mut s = state.clone();
    if let Some(image) = s.get_mut("image").and_then(Value::as_object_mut) {
        for k in ["gain", "white_balance_temperature", "exposure_time"] {
            image.remove(k);
        }
    }
    s
}

struct Worker {
    hub: Arc<Hub>,
    /// Last full state seen from the camera (None while it is unavailable).
    last: Option<Value>,
    last_full: Instant,
    dirty: bool,
    /// Whether the last poll found the camera; None before the first one.
    up: Option<bool>,
}

impl Worker {
    fn read_full(&self) -> Result<Value> {
        let full = Camera::open()?.full_status()?;
        serde_json::to_value(full).map_err(|e| Error::Invalid(e.to_string()))
    }

    /// Reads everything and broadcasts when something changed (or `force`).
    fn publish_full(&mut self, force: bool) {
        self.last_full = Instant::now();
        self.dirty = false;
        match self.read_full() {
            Ok(state) => {
                // coming back from "unavailable" is always news
                let force = force || self.up != Some(true);
                self.up = Some(true);
                let changed = self.last.as_ref().map(stable) != Some(stable(&state));
                if force || changed {
                    self.hub.broadcast(&state);
                }
                self.last = Some(state);
            }
            Err(_) => self.mark_unavailable(),
        }
    }

    /// Broadcasts "available: false" once when the camera goes away.
    fn mark_unavailable(&mut self) {
        self.last = None;
        if self.up != Some(false) {
            self.up = Some(false);
            self.hub.broadcast(&unavailable());
        }
    }

    /// After a change: broadcast at once with the fresh status block and state file merged
    /// into the last full state; the queries follow in the next full read.
    fn publish_quick(&mut self) {
        let fresh = Camera::open().and_then(|c| c.status());
        match (fresh, self.last.as_mut()) {
            (Ok(status), Some(last)) => {
                last["status"] = serde_json::to_value(status).unwrap_or(Value::Null);
                last["state"] = serde_json::to_value(State::load()).unwrap_or(Value::Null);
                self.hub.broadcast(last);
                self.dirty = true;
            }
            _ => self.publish_full(true),
        }
    }

    fn tick(&mut self) {
        let status = Camera::open().and_then(|c| c.status());
        match status {
            Err(_) => self.mark_unavailable(),
            Ok(status) => {
                let status = serde_json::to_value(status).unwrap_or(Value::Null);
                let status_changed = self.last.as_ref().map(|l| &l["status"]) != Some(&status);
                if status_changed || self.last_full.elapsed() >= FULL_EVERY {
                    self.publish_full(false);
                }
            }
        }
    }

    fn run(mut self, jobs: Receiver<Job>) {
        self.publish_full(true);
        let mut next_tick = Instant::now() + TICK;
        loop {
            let wait = if self.dirty {
                Duration::ZERO
            } else {
                next_tick.saturating_duration_since(Instant::now())
            };
            match jobs.recv_timeout(wait) {
                Ok(job) => {
                    let is_status = job.args.first().map(String::as_str) == Some("status");
                    let result = run_request(&job.args);
                    let changed = result.is_ok() && !is_status;
                    let _ = job.reply.send(result);
                    if changed {
                        self.publish_quick();
                    }
                }
                Err(RecvTimeoutError::Timeout) if self.dirty => self.publish_full(false),
                Err(RecvTimeoutError::Timeout) => {
                    self.tick();
                    next_tick = Instant::now() + TICK;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn serve_client(stream: UnixStream, hub: Arc<Hub>, jobs: Sender<Job>) {
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let Ok((id, writer)) = hub.add(stream) else {
        return;
    };
    for line in BufReader::new(reader).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Err(e) => Response {
                id: Value::Null,
                ok: false,
                result: None,
                text: None,
                error: Some(format!("bad request: {e}")),
            },
            Ok(req) => {
                let (tx, rx) = mpsc::channel();
                let outcome = if jobs
                    .send(Job {
                        args: req.args,
                        reply: tx,
                    })
                    .is_ok()
                {
                    rx.recv()
                        .unwrap_or_else(|_| Err("daemon is shutting down".into()))
                } else {
                    Err("daemon is shutting down".into())
                };
                match outcome {
                    Ok((result, text)) => Response {
                        id: req.id,
                        ok: true,
                        result: Some(result),
                        text: Some(text),
                        error: None,
                    },
                    Err(error) => Response {
                        id: req.id,
                        ok: false,
                        result: None,
                        text: None,
                        error: Some(error),
                    },
                }
            }
        };
        if Hub::respond(&writer, &response).is_err() {
            break;
        }
    }
    hub.remove(id);
}

/// The socket systemd passed (LISTEN_FDS), if any.
fn inherited_listener() -> Option<UnixListener> {
    let pid: u32 = std::env::var("LISTEN_PID").ok()?.parse().ok()?;
    let fds: i32 = std::env::var("LISTEN_FDS").ok()?.parse().ok()?;
    (pid == std::process::id() && fds >= 1).then(|| unsafe { UnixListener::from_raw_fd(3) })
}

fn bind() -> Result<UnixListener> {
    if let Some(l) = inherited_listener() {
        return Ok(l);
    }
    let path = socket_path()
        .ok_or_else(|| Error::Invalid("neither XDG_RUNTIME_DIR nor T2L_SOCKET is set".into()))?;
    if UnixStream::connect(&path).is_ok() {
        return Err(Error::Invalid(format!(
            "a daemon is already listening on {}",
            path.display()
        )));
    }
    let _ = std::fs::remove_file(&path); // stale socket of a dead daemon
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

pub fn run() -> Result<()> {
    state::enable_cache();
    let listener = bind()?;
    let hub = Arc::new(Hub::default());
    let (tx, rx) = mpsc::channel();
    let worker = Worker {
        hub: hub.clone(),
        last: None,
        last_full: Instant::now(),
        dirty: false,
        up: None,
    };
    std::thread::spawn(move || worker.run(rx));
    for stream in listener.incoming().flatten() {
        let (hub, tx) = (hub.clone(), tx.clone());
        std::thread::spawn(move || serve_client(stream, hub, tx));
    }
    Ok(())
}
