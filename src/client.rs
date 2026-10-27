// SPDX-License-Identifier: GPL-3.0-or-later

//! Client side of the socket: forwarding CLI commands to a running daemon, and `obsbot4linux watch`.

use crate::cli::Cli;
use crate::proto::{Response, socket_path};
use obsbot4linux::error::{Error, Result};
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Sends the command line to the daemon and prints its answer like a direct call.
/// Returns None when no daemon is listening (the caller then runs the command itself).
pub fn forward(cli: &Cli, args: Vec<String>) -> Option<i32> {
    let stream = UnixStream::connect(socket_path()?).ok()?;
    let finish = |msg: String| -> Option<i32> { Some(crate::cli::print_error(cli.json, &msg)) };
    stream.set_read_timeout(Some(REPLY_TIMEOUT)).ok()?;
    let mut writer = stream.try_clone().ok()?;
    if writeln!(writer, "{}", json!({"id": 1, "args": args})).is_err() {
        return finish("lost the connection to the daemon".into());
    }
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else {
            return finish("the daemon did not answer".into());
        };
        // events and other clients' lines are not ours
        let Ok(resp) = serde_json::from_str::<Response>(&line) else {
            continue;
        };
        if resp.id != json!(1) {
            continue;
        }
        return Some(if resp.ok {
            let result = resp.result.unwrap_or(Value::Null);
            crate::cli::print_ok(cli.json, &result, resp.text.as_deref().unwrap_or(""))
        } else {
            crate::cli::print_error(cli.json, resp.error.as_deref().unwrap_or("unknown error"))
        });
    }
    finish("the daemon closed the connection".into())
}

fn connect_or_start() -> Result<UnixStream> {
    let path = socket_path().ok_or_else(|| {
        Error::Invalid("neither XDG_RUNTIME_DIR nor OBSBOT4LINUX_SOCKET is set".into())
    })?;
    if let Ok(s) = UnixStream::connect(&path) {
        return Ok(s);
    }
    let exe = std::env::current_exe()?;
    // its errors go next to the state file; without a log they are lost, not fatal
    let log = obsbot4linux::state::path().with_file_name("daemon.log");
    let stderr = log
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| OpenOptions::new().create(true).append(true).open(&log))
        .map_or_else(|_| Stdio::null(), Stdio::from);
    // detached: own process group, no stdin/stdout, outlives this process
    Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .process_group(0)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match UnixStream::connect(&path) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() >= deadline => {
                return Err(Error::Invalid(format!(
                    "cannot reach the daemon: {e} (its log: {})",
                    log.display()
                )));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// Copies the daemon's lines to `out`. The exit code: 1 when the daemon closed the connection
/// (so a widget can tell a crash from a normal end), 0 when `out` closed.
fn relay(daemon: impl BufRead, mut out: impl Write) -> i32 {
    for line in daemon.lines() {
        let Ok(line) = line else { break };
        if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
            return 0;
        }
    }
    eprintln!("obsbot4linux: the daemon closed the connection");
    1
}

/// `obsbot4linux watch`: the daemon's lines (state events and answers) go to stdout, JSON request
/// lines from stdin go to the daemon. Ends when stdin closes (exit 0) or the daemon goes away
/// (exit 1).
pub fn watch() -> Result<()> {
    let stream = connect_or_start()?;
    let mut writer = stream.try_clone()?;
    std::thread::spawn(move || {
        std::process::exit(relay(BufReader::new(stream), std::io::stdout()));
    });
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if !line.trim().is_empty() && writeln!(writer, "{line}").is_err() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    #[test]
    fn watch_fails_when_the_daemon_goes_away() {
        let mut out = Vec::new();
        assert_eq!(relay(Cursor::new("a\nb\n"), &mut out), 1);
        assert_eq!(out, b"a\nb\n");
    }

    struct Closed;

    impl Write for Closed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn watch_ends_normally_when_its_reader_goes_away() {
        assert_eq!(relay(Cursor::new("a\n"), Closed), 0);
    }
}
