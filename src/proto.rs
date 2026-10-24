// SPDX-License-Identifier: GPL-3.0-or-later

//! Socket protocol (JSON lines) shared by the daemon and its clients.
//!
//! Request:  `{"id":1,"args":["fov","wide"]}` (args are the CLI arguments)
//! Response: `{"id":1,"ok":true,"result":...,"text":"..."}` or `{"id":1,"ok":false,"error":"..."}`
//! Event:    `{"event":"state","state":{...}}` to every client after each change.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// `T2L_SOCKET`, else `$XDG_RUNTIME_DIR/t2l.sock`. There is no fallback to a shared
/// directory such as /tmp: another local user could create the socket first.
pub fn socket_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("T2L_SOCKET") {
        return Some(p.into());
    }
    std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("t2l.sock"))
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct Request {
    pub id: Value,
    pub args: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: Value,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The text the CLI prints without --json.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn state_event(state: &Value) -> Value {
    json!({"event": "state", "state": state})
}

type Writer = Arc<Mutex<UnixStream>>;

fn write_line(w: &Writer, line: &Value) -> io::Result<()> {
    let mut s = w.lock().unwrap_or_else(|e| e.into_inner());
    let mut buf = line.to_string();
    buf.push('\n');
    s.write_all(buf.as_bytes())
}

/// The connected clients. Broadcasts go to all of them; a client that cannot be written to
/// (gone, or not reading) is dropped.
#[derive(Default)]
pub struct Hub {
    clients: Mutex<Vec<(u64, Writer)>>,
    last: Mutex<Option<Value>>,
    next: AtomicU64,
}

impl Hub {
    /// Registers a client and sends it the latest state, if there is one.
    pub fn add(&self, stream: UnixStream) -> io::Result<(u64, Writer)> {
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let writer: Writer = Arc::new(Mutex::new(stream));
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = self.last.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            write_line(&writer, &state_event(state))?;
        }
        clients.push((id, writer.clone()));
        Ok((id, writer))
    }

    pub fn remove(&self, id: u64) {
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(i, _)| *i != id);
    }

    pub fn respond(writer: &Writer, response: &Response) -> io::Result<()> {
        write_line(
            writer,
            &serde_json::to_value(response).unwrap_or(Value::Null),
        )
    }

    pub fn broadcast(&self, state: &Value) {
        let event = state_event(state);
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(state.clone());
        clients.retain(|(_, w)| write_line(w, &event).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    #[test]
    fn request_and_response_serialise_as_documented() {
        let r: Request = serde_json::from_str(r#"{"id":1,"args":["fov","wide"]}"#).unwrap();
        assert_eq!(
            r,
            Request {
                id: json!(1),
                args: vec!["fov".into(), "wide".into()]
            }
        );

        let ok = Response {
            id: json!(1),
            ok: true,
            result: Some(json!({"a": 1})),
            text: None,
            error: None,
        };
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            json!({"id": 1, "ok": true, "result": {"a": 1}})
        );
        let err = Response {
            id: json!("x"),
            ok: false,
            result: None,
            text: None,
            error: Some("boom".into()),
        };
        assert_eq!(
            serde_json::to_value(&err).unwrap(),
            json!({"id": "x", "ok": false, "error": "boom"})
        );
        assert_eq!(
            state_event(&json!({"available": true})),
            json!({"event": "state", "state": {"available": true}})
        );
    }

    fn read_line(s: UnixStream) -> Value {
        let mut line = String::new();
        BufReader::new(s).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn every_client_gets_the_event_and_new_clients_the_last_state() {
        let hub = Hub::default();
        let (a_srv, a) = UnixStream::pair().unwrap();
        let (b_srv, b) = UnixStream::pair().unwrap();
        hub.add(a_srv).unwrap();
        hub.add(b_srv).unwrap();

        hub.broadcast(&json!({"led": 2}));
        assert_eq!(read_line(a), state_event(&json!({"led": 2})));
        assert_eq!(read_line(b), state_event(&json!({"led": 2})));

        let (c_srv, c) = UnixStream::pair().unwrap();
        hub.add(c_srv).unwrap();
        assert_eq!(
            read_line(c),
            state_event(&json!({"led": 2})),
            "late client starts from the last state"
        );
    }

    #[test]
    fn a_closed_client_is_dropped_and_does_not_stop_the_broadcast() {
        let hub = Hub::default();
        let (gone_srv, gone) = UnixStream::pair().unwrap();
        let (live_srv, live) = UnixStream::pair().unwrap();
        hub.add(gone_srv).unwrap();
        hub.add(live_srv).unwrap();
        drop(gone);
        hub.broadcast(&json!(1));
        hub.broadcast(&json!(2));
        assert_eq!(hub.clients.lock().unwrap().len(), 1);
        drop(hub);
        assert_eq!(BufReader::new(live).lines().count(), 2);
    }
}
