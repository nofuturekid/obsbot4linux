// SPDX-License-Identifier: GPL-3.0-or-later

//! `obsbot4linux daemon`: owns the camera. One worker thread does all camera access (so frames from
//! every client are serialised and sequence numbers stay unique), executes requests with the
//! CLI's own argument parser and polls the camera to notice changes it makes itself.

use crate::cli::{Cli, Cmd, PresetCmd, execute};
use crate::proto::{Hub, Request, Response, socket_path};
use clap::Parser;
use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use nix::sys::stat::{Mode, umask};
use obsbot4linux::camera::{CAPABILITIES, Camera, ZoneInfo};
use obsbot4linux::commands::AiMode;
use obsbot4linux::error::{Error, Result};
use obsbot4linux::state::{self, State};
use obsbot4linux::streaming;
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::os::fd::FromRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

const TICK: Duration = Duration::from_secs(1);
/// The 0x0104 / zoom queries and the image controls are read at least this often.
const FULL_EVERY: Duration = Duration::from_secs(5);
/// The zone, hand and start pose readbacks (nine queries) change only by a command, so they are asked
/// again after one of ours and otherwise this often, to notice changes made with OBSBOT Center.
const ZONE_EVERY: Duration = Duration::from_secs(60);

struct Job {
    args: Vec<String>,
    reply: Sender<std::result::Result<(Value, String), String>>,
}

/// Parses a request with the CLI's own argument parser, like a direct call would.
fn parse_request(args: &[String]) -> std::result::Result<Cmd, String> {
    let cli = Cli::try_parse_from(
        std::iter::once("obsbot4linux".to_string()).chain(args.iter().cloned()),
    )
    .map_err(|e| e.to_string())?;
    if matches!(cli.command, Cmd::Daemon | Cmd::Watch | Cmd::Detect) {
        return Err("daemon, watch and detect cannot be run through the daemon".into());
    }
    Ok(cli.command)
}

/// Commands that only read: they neither drop the zone cache nor cause a re-read and an event.
fn read_only(cmd: &Cmd) -> bool {
    matches!(cmd, Cmd::Status | Cmd::Preset(PresetCmd::List))
}

fn unavailable() -> Value {
    json!({
        "available": false, "model": null, "device": null,
        "streaming": false, "streaming_apps": [], "target": null, "capabilities": CAPABILITIES
    })
}

/// Differences in these auto-adjusted controls are not a change worth a broadcast.
fn stable(state: &Value) -> Value {
    let mut s = state.clone();
    // the box follows the person all the time; only `detected` is news
    if let Some(target) = s.get_mut("target").and_then(Value::as_object_mut) {
        target.remove("box");
    }
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
    /// Zone, hand and start pose readbacks with the time they were read; dropped after a command.
    zone: Option<(ZoneInfo, Instant)>,
    dirty: bool,
    /// Whether the last poll found the camera; None before the first one.
    up: Option<bool>,
}

impl Worker {
    fn read_full(&mut self, cam: Option<&Camera>) -> Result<Value> {
        let known = self
            .zone
            .as_ref()
            .filter(|(_, at)| at.elapsed() < ZONE_EVERY)
            .map(|(info, _)| info.clone());
        let reused = known.is_some();
        let opened;
        let cam = match cam {
            Some(cam) => cam,
            None => {
                opened = Camera::open()?;
                &opened
            }
        };
        let full = cam.full_status_with(known)?;
        if !reused {
            let info = ZoneInfo {
                zone: full.zone.clone(),
                hand: full.hand.clone(),
                startup: full.state.startup,
            };
            self.zone = Some((info, Instant::now()));
        }
        serde_json::to_value(full).map_err(|e| Error::Invalid(e.to_string()))
    }

    /// Reads everything (with `cam`, else a camera opened for it) and broadcasts when something
    /// changed (or `force`).
    fn publish_full(&mut self, cam: Option<&Camera>, force: bool) {
        self.last_full = Instant::now();
        self.dirty = false;
        match self.read_full(cam) {
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
        let cam = Camera::open().ok();
        let fresh = cam.as_ref().map(Camera::status);
        match (fresh, self.last.as_mut()) {
            (Some(Ok(status)), Some(last)) => {
                if status.ai_mode == Some(AiMode::Static) {
                    last["target"] = Value::Null;
                }
                last["status"] = serde_json::to_value(status).unwrap_or(Value::Null);
                // the remembered values only; zone_follow and startup come from the camera
                if let Ok(Value::Object(fresh)) = serde_json::to_value(State::load()) {
                    for (k, v) in fresh {
                        last["state"][k] = v;
                    }
                }
                self.hub.broadcast(last);
                self.dirty = true;
            }
            _ => self.publish_full(cam.as_ref(), true),
        }
    }

    /// The gimbal pose, the tracking target and the streaming applications every tick, so a
    /// pose moved by AI tracking or a started video call shows within a second. One broadcast
    /// for all of them.
    fn publish_live(&mut self, cam: &Camera) {
        let Some(last) = self.last.as_mut() else {
            return;
        };
        let mut changed = false;
        if let Ok(pose) = cam.live_pose() {
            let pose = serde_json::to_value(pose).unwrap_or(Value::Null);
            if last["pose"] != pose {
                last["pose"] = pose;
                changed = true;
            }
        }
        // box jitter alone is no event: it is kept in `last` for the next broadcast
        let tracking = last["status"]["ai_mode"]
            .as_str()
            .is_some_and(|m| m != "static");
        let target = if tracking {
            cam.target()
                .ok()
                .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
        } else {
            Some(Value::Null)
        };
        if let Some(target) = target {
            if last["target"]["detected"] != target["detected"] {
                changed = true;
            }
            last["target"] = target;
        }
        if let Some(device) = last["device"].as_str() {
            let streaming = json!(streaming::is_streaming(Path::new(device)));
            let apps = streaming::apps_using(Path::new(device));
            let apps = json!(apps);
            if last["streaming"] != streaming || last["streaming_apps"] != apps {
                last["streaming"] = streaming;
                last["streaming_apps"] = apps;
                changed = true;
            }
        }
        if changed {
            self.hub.broadcast(last);
        }
    }

    /// One camera handle for the whole tick: each open enumerates `/dev/v4l/by-id` again.
    fn tick(&mut self) {
        let Ok(cam) = Camera::open() else {
            return self.mark_unavailable();
        };
        match cam.status() {
            Err(_) => self.mark_unavailable(),
            Ok(status) => {
                let status = serde_json::to_value(status).unwrap_or(Value::Null);
                let status_changed = self.last.as_ref().map(|l| &l["status"]) != Some(&status);
                if status_changed || self.last_full.elapsed() >= FULL_EVERY {
                    self.publish_full(Some(&cam), false);
                } else {
                    self.publish_live(&cam);
                }
            }
        }
    }

    fn run(mut self, jobs: Receiver<Job>) {
        self.publish_full(None, true);
        let mut next_tick = Instant::now() + TICK;
        loop {
            let wait = if self.dirty {
                Duration::ZERO
            } else {
                next_tick.saturating_duration_since(Instant::now())
            };
            match jobs.recv_timeout(wait) {
                Ok(job) => {
                    // from the parsed command: argv can start with a global flag (--json status)
                    let (result, changes) = match parse_request(&job.args) {
                        Ok(cmd) => {
                            let changes = !read_only(&cmd);
                            (execute(cmd).map_err(|e| e.to_string()), changes)
                        }
                        Err(e) => (Err(e), false),
                    };
                    let changed = changes && result.is_ok();
                    let _ = job.reply.send(result);
                    if changed {
                        self.zone = None;
                        self.publish_quick();
                    }
                }
                Err(RecvTimeoutError::Timeout) if self.dirty => self.publish_full(None, false),
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

/// The listener and, when this process bound it, the lock that keeps a second daemon from
/// replacing the socket (held as long as the daemon runs).
fn bind() -> Result<(UnixListener, Option<Flock<File>>)> {
    if let Some(l) = inherited_listener() {
        return Ok((l, None));
    }
    let path = socket_path().ok_or_else(|| {
        Error::Invalid("neither XDG_RUNTIME_DIR nor OBSBOT4LINUX_SOCKET is set".into())
    })?;
    let (listener, lock) = bind_at(&path)?;
    Ok((listener, Some(lock)))
}

/// Takes `<socket>.lock`, then replaces a stale socket. Two daemons started at once (two
/// `watch` at login) would otherwise both find no listener and the second unlink the first's.
fn bind_at(path: &Path) -> Result<(UnixListener, Flock<File>)> {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)?;
    let lock = match Flock::lock(lock, FlockArg::LockExclusiveNonblock) {
        Ok(lock) => lock,
        Err((_, Errno::EWOULDBLOCK)) => {
            return Err(Error::Invalid(format!(
                "another daemon holds {}",
                Path::new(&lock_path).display()
            )));
        }
        Err((_, e)) => return Err(std::io::Error::from(e).into()),
    };
    if UnixStream::connect(path).is_ok() {
        return Err(Error::Invalid(format!(
            "a daemon is already listening on {}",
            path.display()
        )));
    }
    let _ = std::fs::remove_file(path); // stale socket of a dead daemon
    // no window with a socket others can connect to; still single-threaded here
    let old = umask(Mode::from_bits_truncate(0o077));
    let listener = UnixListener::bind(path);
    umask(old);
    let listener = listener?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, lock))
}

pub fn run() -> Result<()> {
    state::enable_cache();
    let (listener, _lock) = bind()?;
    let hub = Arc::new(Hub::default());
    let (tx, rx) = mpsc::channel();
    let worker = Worker {
        hub: hub.clone(),
        last: None,
        last_full: Instant::now(),
        zone: None,
        dirty: false,
        up: None,
    };
    let worker = std::thread::spawn(move || worker.run(rx));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let (hub, tx) = (hub.clone(), tx.clone());
                    std::thread::spawn(move || serve_client(stream, hub, tx));
                }
                // e.g. EMFILE: retrying at once would spin
                Err(e) => {
                    eprintln!("obsbot4linux: accept: {e}");
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
    // without the worker nothing answers; exit so that systemd (Restart=on-failure) restarts us
    let how = if worker.join().is_err() {
        "panicked"
    } else {
        "stopped"
    };
    Err(Error::Invalid(format!("the camera worker {how}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_state_has_null_model_and_device() {
        let v = unavailable();
        assert_eq!(v["available"], false);
        assert!(v["model"].is_null() && v["device"].is_null());
        assert!(v.as_object().unwrap().contains_key("model"));
        assert!(v.as_object().unwrap().contains_key("device"));
        assert_eq!(v["streaming"], false);
        assert_eq!(v["streaming_apps"], json!([]));
    }

    #[test]
    fn target_box_jitter_is_not_a_change_but_detected_is() {
        let at = |detected, x| json!({"target": {"detected": detected, "box": [x, 0, 1, 1]}});
        assert_eq!(stable(&at(true, 0.1)), stable(&at(true, 0.2)));
        assert_ne!(stable(&at(true, 0.1)), stable(&at(false, 0.1)));
        assert_ne!(stable(&at(true, 0.1)), stable(&json!({"target": null})));
        assert_eq!(unavailable()["target"], Value::Null);
    }

    fn parsed(line: &str) -> Cmd {
        let args: Vec<String> = line.split(' ').map(String::from).collect();
        parse_request(&args).unwrap()
    }

    #[test]
    fn reads_are_told_from_the_parsed_command_not_from_argv() {
        // scripts call `--json status`; the global flags may stand anywhere
        for line in [
            "status",
            "--json status",
            "status --json",
            "preset list",
            "--json preset list",
            "preset --json list",
        ] {
            assert!(read_only(&parsed(line)), "{line}");
        }
        for line in [
            "led 2",
            "--json led 2",
            "preset recall 1",
            "--json gimbal --pan 3",
        ] {
            assert!(!read_only(&parsed(line)), "{line}");
        }
        assert!(parse_request(&["--json".into(), "watch".into()]).is_err());
    }

    fn socket_in(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("o4l-sock-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("obsbot4linux.sock")
    }

    #[test]
    fn a_second_daemon_does_not_take_over_the_socket() {
        let path = socket_in("second");
        // a daemon that holds the lock but has not bound yet: no listener to find
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let file = File::create(&lock_path).unwrap();
        let starting = Flock::lock(file, FlockArg::LockExclusiveNonblock).unwrap();
        let err = bind_at(&path).unwrap_err().to_string();
        assert!(err.contains("another daemon holds"), "{err}");
        assert!(!path.exists(), "the loser creates no socket");
        drop(starting);
        let (listener, _lock) = bind_at(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        UnixStream::connect(&path).unwrap();
        listener.accept().unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_stale_socket_is_replaced() {
        let path = socket_in("stale");
        drop(bind_at(&path).unwrap()); // a dead daemon leaves its socket and lock file
        assert!(path.exists() && UnixStream::connect(&path).is_err());
        let (listener, _lock) = bind_at(&path).unwrap();
        UnixStream::connect(&path).unwrap();
        listener.accept().unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
