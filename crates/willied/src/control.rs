//! The daemon's own connection to a session supervisor. It attaches as a
//! control client, asks for status when adopting a session, and watches
//! the supervisor's event stream — turning each line into a `Session`
//! change — until the socket closes.
//!
//! Unix-socket only: the whole module is gated to Linux so the crate
//! still type-checks on the Windows host, matching `willie-sess`'s
//! `server` module.

#![cfg(target_os = "linux")]

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    thread,
    time::Duration,
};

use willie_core::session::SessionEvent;
use willie_linux::wire;
use willie_proto::supervisor::{Hello, Role, Status};

/// A live control connection.
#[derive(Debug)]
pub struct Control {
    stream: UnixStream,
}

/// Attach to a supervisor as the control client.
///
/// Called by session adoption once it lands (Task 8c); allow dead code
/// until then so the plain (non-test) binary still builds clean.
#[allow(dead_code)]
pub fn connect(socket: &Path) -> io::Result<Control> {
    let mut stream = UnixStream::connect(socket)?;
    let hello = Hello {
        role: Role::Control,
        rows: 0,
        cols: 0,
    };
    let frame = wire::encode_json(wire::HELLO, &hello)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    stream.write_all(&frame)?;
    Ok(Control { stream })
}

impl Control {
    /// Ask for and read one status. Used when adopting a session at start.
    ///
    /// Wired in the sessions RPC task (8c); until then nothing calls it.
    #[allow(dead_code)]
    pub fn status(&mut self) -> io::Result<Status> {
        self.stream
            .write_all(&wire::encode(wire::STATUS_REQ, b""))?;
        self.stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut decoder = wire::Decoder::new();
        let mut buf = [0u8; 4096];
        loop {
            if let Some(f) = decoder.pop()
                && f.kind == wire::STATUS
            {
                self.stream.set_read_timeout(None)?;
                return wire::decode_json(&f.payload).map_err(|e| {
                    io::Error::new(io::ErrorKind::InvalidData, e)
                });
            }
            let n = self.stream.read(&mut buf)?;
            if n == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            decoder.push(&buf[..n]);
        }
    }

    /// Ask the supervisor to stop its harness.
    ///
    /// Wired in the sessions RPC task (8c); until then nothing calls it.
    #[allow(dead_code)]
    pub fn stop(&mut self) -> io::Result<()> {
        self.stream.write_all(&wire::encode(wire::STOP, b""))
    }

    /// Spawn a reader that folds every event into `on_event` until the
    /// supervisor closes. The thread ends on a `closed` frame or EOF, so
    /// the caller learns the connection is over when `on_event` stops
    /// being called — Task 8c owns finalising the session from there.
    ///
    /// Wired in the sessions RPC task (8c); until then nothing calls it.
    #[allow(dead_code)]
    pub fn watch(
        &mut self,
        mut on_event: impl FnMut(SessionEvent) + Send + 'static,
    ) {
        let Ok(stream) = self.stream.try_clone() else {
            return;
        };
        let _ = thread::Builder::new().name("control".to_owned()).spawn(
            move || {
                let mut stream = stream;
                let mut decoder = wire::Decoder::new();
                let mut buf = [0u8; 8192];
                loop {
                    while let Some(f) = decoder.pop() {
                        if f.kind == wire::EVENT
                            && let Ok(ev) =
                                wire::decode_json::<SessionEvent>(&f.payload)
                        {
                            on_event(ev);
                        }
                        if f.kind == wire::CLOSED {
                            return;
                        }
                    }
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => decoder.push(&buf[..n]),
                    }
                }
            },
        );
    }
}

#[cfg(test)]
#[cfg(target_os = "linux")]
mod tests {
    use std::{
        io::{Read, Write},
        os::unix::net::UnixListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    use willie_core::session::SessionEventKind;
    use willie_linux::wire;
    use willie_proto::supervisor::{CloseReason, Closed, Hello, Role, Status};

    use super::*;

    fn socket(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("willie-control-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("s.sock")
    }

    #[test]
    fn connect_reports_status_and_watch_delivers_events_until_closed() {
        let path = socket("basic");
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut dec = wire::Decoder::new();
            let mut buf = [0u8; 1024];
            // A stream socket has no frame boundaries: the client's hello
            // and status request, sent back to back, may already share
            // one read. Drain buffered frames before asking for more.
            let mut next_frame = || loop {
                if let Some(f) = dec.pop() {
                    return f;
                }
                let n = s.read(&mut buf).unwrap();
                dec.push(&buf[..n]);
            };
            // hello
            let hello: Hello =
                wire::decode_json(&next_frame().payload).unwrap();
            assert_eq!(hello.role, Role::Control);
            // answer a status request
            assert_eq!(next_frame().kind, wire::STATUS_REQ);
            let status = Status {
                pid: 7,
                state: "running".into(),
                clients: 0,
                started_at: "1".into(),
            };
            s.write_all(&wire::encode_json(wire::STATUS, &status).unwrap())
                .unwrap();
            // one event, then closed
            let ev = SessionEvent {
                at: "2".into(),
                kind: SessionEventKind::Attached { client: 1 },
            };
            s.write_all(&wire::encode_json(wire::EVENT, &ev).unwrap())
                .unwrap();
            s.write_all(
                &wire::encode_json(
                    wire::CLOSED,
                    &Closed {
                        reason: CloseReason::Exited,
                        code: Some(0),
                        signal: None,
                    },
                )
                .unwrap(),
            )
            .unwrap();
        });
        let mut control = connect(&path).unwrap();
        assert_eq!(control.status().unwrap().pid, 7);
        let (tx, rx) = mpsc::channel();
        control.watch(move |ev| {
            let _ = tx.send(ev);
        });
        let first = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(first.kind, SessionEventKind::Attached { .. }));
        server.join().unwrap();
    }
}
