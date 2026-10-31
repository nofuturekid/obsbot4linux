// SPDX-License-Identifier: GPL-3.0-or-later

//! `obsbot4linux daemon`: owns the camera. One worker thread does all camera access (so frames from
//! every client are serialised and sequence numbers stay unique), executes requests with the
//! CLI's own argument parser and polls the camera to notice changes it makes itself.

use crate::cli::{Cli, Cmd, GimbalCmd, PresetCmd, ZoomCmd, execute_with};
use crate::proto::{Hub, Request, Response, socket_path};
use clap::Parser;
use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use nix::sys::stat::{Mode, umask};
use obsbot4linux::camera::{CAPABILITIES, Camera, Identity, ZoneInfo};
use obsbot4linux::commands::AiMode;
use obsbot4linux::error::{Error, Result};
use obsbot4linux::state::{self, State};
use obsbot4linux::streaming;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::os::fd::FromRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
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

/// Stops change no setting: the zone readbacks stay, so a stop is not queued behind a long full
/// read.
fn keeps_zone_cache(cmd: &Cmd) -> bool {
    matches!(
        cmd,
        Cmd::Gimbal {
            action: Some(GimbalCmd::Stop),
            ..
        } | Cmd::Zoom {
            stop: Some(ZoomCmd::Stop),
            ..
        }
    )
}

/// Which connections want the live pose. It belongs to the connection, not to the camera: a
/// widget that crashes or is closed without saying "off" must not keep the polling running, so a
/// closing connection is removed here. A set of connection ids rather than a counter, so a
/// repeated "on" from one connection is not counted twice and one "off" still turns it off.
#[derive(Default)]
struct PoseDemand(Mutex<HashSet<u64>>);

impl PoseDemand {
    fn set(&self, connection: u64, wanted: bool) {
        let mut wanting = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if wanted {
            wanting.insert(connection);
        } else {
            wanting.remove(&connection);
        }
    }

    fn wanted(&self) -> bool {
        !self.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
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
    /// Whether any client wants the live pose; without one it is only read with the full status.
    pose: Arc<PoseDemand>,
    /// Last full state seen from the camera (None while it is unavailable).
    last: Option<Value>,
    last_full: Instant,
    /// Zone, hand and start pose readbacks with the time they were read; dropped after a command.
    zone: Option<(ZoneInfo, Instant)>,
    /// Firmware and model codes, read once per connect: dropped when the camera goes away.
    identity: Option<Identity>,
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
        let full = cam.full_status_with(known, self.identity.clone())?;
        self.remember_identity(Identity {
            firmware: full.firmware.clone(),
            model_info: full.model_info,
        });
        if !reused {
            let info = ZoneInfo {
                zone: full.zone.clone(),
                hand: full.hand.clone(),
                startup: full.state.startup,
                framing: full.framing.clone(),
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

    /// Keeps the identity once the camera has answered at least one of the two queries; an
    /// all-empty read is tried again with the next full read.
    fn remember_identity(&mut self, read: Identity) {
        if self.identity.is_none() && (read.firmware.is_some() || read.model_info.is_some()) {
            self.identity = Some(read);
        }
    }

    /// Broadcasts "available: false" once when the camera goes away.
    fn mark_unavailable(&mut self) {
        self.last = None;
        self.identity = None;
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

    /// The gimbal pose (only while a client wants it), the tracking target and the streaming
    /// applications every tick, so a pose moved by AI tracking or a started video call shows
    /// within a second. One broadcast for all of them. Each pose query holds the camera lock,
    /// so it is left out when nobody looks at it (#68); `pose` then changes with the full read.
    fn publish_live(&mut self, cam: &Camera) {
        let Some(last) = self.last.as_mut() else {
            return;
        };
        let mut changed = false;
        if let Some(Ok(pose)) = self.pose.wanted().then(|| cam.live_pose()) {
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
                // an AI mode change (a gesture, say) overwrites the framing options
                if self.last.as_ref().map(|l| &l["status"]["ai_mode"]) != Some(&status["ai_mode"]) {
                    self.zone = None;
                }
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
                    let (result, changes, keep_zone) = match parse_request(&job.args) {
                        Ok(cmd) => {
                            let changes = !read_only(&cmd);
                            let keep_zone = keeps_zone_cache(&cmd);
                            (
                                execute_with(cmd, self.identity.clone()).map_err(|e| e.to_string()),
                                changes,
                                keep_zone,
                            )
                        }
                        Err(e) => (Err(e), false, false),
                    };
                    let changed = changes && result.is_ok();
                    let _ = job.reply.send(result);
                    if changed {
                        if !keep_zone {
                            self.zone = None;
                        }
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

fn serve_client(stream: UnixStream, hub: Arc<Hub>, pose: Arc<PoseDemand>, jobs: Sender<Job>) {
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
            // per connection, so it is answered here and not by the camera worker
            Ok(req) => {
                let (tx, rx) = mpsc::channel();
                let outcome = if let Ok(Cmd::LivePose { state }) = parse_request(&req.args) {
                    pose.set(id, state.get());
                    Ok((
                        json!({"live_pose": state.get()}),
                        format!("live-pose: {}", if state.get() { "on" } else { "off" }),
                    ))
                } else if jobs
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
    pose.set(id, false);
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
    let pose = Arc::new(PoseDemand::default());
    let (tx, rx) = mpsc::channel();
    let worker = Worker {
        hub: hub.clone(),
        pose: pose.clone(),
        last: None,
        last_full: Instant::now(),
        zone: None,
        identity: None,
        dirty: false,
        up: None,
    };
    let worker = std::thread::spawn(move || worker.run(rx));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let (hub, pose, tx) = (hub.clone(), pose.clone(), tx.clone());
                    std::thread::spawn(move || serve_client(stream, hub, pose, tx));
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
    use crate::cli::OnOff;

    #[test]
    fn a_lost_camera_is_announced_once_with_null_model_and_device() {
        let hub = Arc::new(Hub::default());
        let (srv, client) = UnixStream::pair().unwrap();
        hub.add(srv).unwrap();
        let mut worker = Worker {
            hub: hub.clone(),
            pose: Arc::default(),
            last: Some(json!({"available": true, "model": "Tiny 2 Lite"})),
            last_full: Instant::now(),
            zone: None,
            identity: None,
            dirty: false,
            up: Some(true),
        };
        // every failed poll lands here; only the first one is news
        worker.mark_unavailable();
        worker.mark_unavailable();
        assert!(worker.last.is_none());
        drop((worker, hub));
        let events: Vec<Value> = BufReader::new(client)
            .lines()
            .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
            .collect();
        assert_eq!(events.len(), 1);
        let state = &events[0]["state"];
        assert_eq!(state["available"], false);
        // the widget tells "no camera" by these keys being null, not missing
        let keys = state.as_object().unwrap();
        assert!(keys["model"].is_null() && keys["device"].is_null());
        assert_eq!(state["streaming"], false);
        assert_eq!(state["streaming_apps"], json!([]));
    }

    #[test]
    fn identity_is_kept_once_answered_and_dropped_with_the_camera() {
        let mut worker = Worker {
            hub: Arc::default(),
            pose: Arc::default(),
            last: None,
            last_full: Instant::now(),
            zone: None,
            identity: None,
            dirty: false,
            up: Some(true),
        };
        let none = Identity {
            firmware: None,
            model_info: None,
        };
        let known = Identity {
            firmware: Some("6.2.8.1".into()),
            model_info: None,
        };
        worker.remember_identity(none.clone());
        assert_eq!(worker.identity, None, "a silent camera is asked again");
        worker.remember_identity(known.clone());
        worker.remember_identity(none);
        assert_eq!(worker.identity, Some(known), "once per connect");
        worker.mark_unavailable();
        assert_eq!(
            worker.identity, None,
            "a replugged camera may have new firmware"
        );
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
            "gimbal stop",
            "gimbal center 0.3 0.4",
            "zoom 2 --speed 30",
            "framing size close-up",
            "speed-mode auto",
        ] {
            assert!(!read_only(&parsed(line)), "{line}");
        }
        assert!(parse_request(&["--json".into(), "watch".into()]).is_err());
    }

    #[test]
    fn stops_keep_the_zone_cache_other_moves_drop_it() {
        for line in ["gimbal stop", "zoom stop", "--json gimbal stop"] {
            let cmd = parsed(line);
            assert!(keeps_zone_cache(&cmd) && !read_only(&cmd), "{line}");
        }
        for line in ["gimbal --pan 1", "gimbal rate --pan 1", "zoom 2", "led 1"] {
            assert!(!keeps_zone_cache(&parsed(line)), "{line}");
        }
    }

    #[test]
    fn the_pose_is_polled_while_any_connection_wants_it() {
        let demand = PoseDemand::default();
        assert!(!demand.wanted(), "a new daemon polls nothing");
        demand.set(1, true);
        demand.set(2, true);
        // a widget closing must not switch the pose off for the other one
        demand.set(1, false);
        assert!(demand.wanted());
        demand.set(2, false);
        assert!(!demand.wanted());
    }

    #[test]
    fn repeated_on_from_one_connection_needs_only_one_off() {
        // a reconnecting widget may say "on" again; a counter would never get back to zero
        let demand = PoseDemand::default();
        demand.set(7, true);
        demand.set(7, true);
        demand.set(7, false);
        assert!(!demand.wanted());
        demand.set(7, false); // "off" without "on" is harmless
        assert!(!demand.wanted());
    }

    #[test]
    fn live_pose_is_a_request_for_the_connection_not_a_camera_command() {
        assert!(matches!(
            parsed("live-pose on"),
            Cmd::LivePose { state: OnOff::On }
        ));
        assert!(matches!(
            parsed("--json live-pose off"),
            Cmd::LivePose { state: OnOff::Off }
        ));
        assert!(parse_request(&["live-pose".into(), "maybe".into()]).is_err());
        assert!(CAPABILITIES.contains(&"live_pose_on_demand"));
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
