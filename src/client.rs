// SPDX-License-Identifier: GPL-3.0-or-later

//! Client side of the socket: forwarding CLI commands to a running daemon, and `t2l watch`.

use crate::cli::Cli;
use crate::proto::{Response, socket_path};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use t2l::error::{Error, Result};

const REPLY_TIMEOUT: Duration = Duration::from_secs(30);

/// Sends the command line to the daemon and prints its answer like a direct call.
/// Returns None when no daemon is listening (the caller then runs the command itself).
pub fn forward(cli: &Cli, args: Vec<String>) -> Option<i32> {
    let stream = UnixStream::connect(socket_path()).ok()?;
    let finish = |msg: String| -> Option<i32> { Some(crate::cli::print_error(cli.json, &msg)) };
    stream.set_read_timeout(Some(REPLY_TIMEOUT)).ok()?;
    let mut writer = stream.try_clone().ok()?;
    if writeln!(writer, "{}", json!({"id": 1, "args": args})).is_err() {
        return finish("lost the connection to the daemon".into());
    }
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { return finish("the daemon did not answer".into()) };
        // events and other clients' lines are not ours
        let Ok(resp) = serde_json::from_str::<Response>(&line) else { continue };
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
    if let Ok(s) = UnixStream::connect(socket_path()) {
        return Ok(s);
    }
    let exe = std::env::current_exe()?;
    // detached: own process group, no stdio, outlives this process
    Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match UnixStream::connect(socket_path()) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() >= deadline => {
                return Err(Error::Invalid(format!("cannot reach the daemon: {e}")));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

/// `t2l watch`: the daemon's lines (state events and answers) go to stdout, JSON request
/// lines from stdin go to the daemon. Ends when stdin closes.
pub fn watch() -> Result<()> {
    let stream = connect_or_start()?;
    let mut writer = stream.try_clone()?;
    std::thread::spawn(move || {
        let mut out = std::io::stdout();
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if writeln!(out, "{line}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
        // the daemon went away or stdout closed
        std::process::exit(0);
    });
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if !line.trim().is_empty() && writeln!(writer, "{line}").is_err() {
            break;
        }
    }
    Ok(())
}
