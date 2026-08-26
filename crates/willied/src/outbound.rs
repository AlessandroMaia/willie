//! One thread owns the daemon's stdout so responses and notifications
//! never interleave. Everything else sends through a channel.
//!
//! Scaffolding until the daemon wires this module in (Task 9).
#![cfg_attr(target_os = "linux", allow(dead_code))]

use std::{
    io::{self, Write},
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
};

use willie_proto::rpc::{Notification, Response};

enum Out {
    Response(Box<Response>),
    Notification(Box<Notification>),
}

#[derive(Clone)]
pub struct Outbound {
    tx: Sender<Out>,
}

impl std::fmt::Debug for Outbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Outbound").finish_non_exhaustive()
    }
}

impl Outbound {
    pub fn spawn<W: Write + Send + 'static>(
        mut writer: W,
    ) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<Out>();
        let handle = thread::spawn(move || {
            for msg in rx {
                let line = match &msg {
                    Out::Response(r) => serde_json::to_string(r),
                    Out::Notification(n) => serde_json::to_string(n),
                };
                let Ok(line) = line else { continue };
                if writeln!(writer, "{line}").is_err()
                    || writer.flush().is_err()
                {
                    break;
                }
            }
        });
        (Self { tx }, handle)
    }

    pub fn send_response(&self, r: Response) {
        let _ = self.tx.send(Out::Response(Box::new(r)));
    }

    pub fn send_notification(&self, n: Notification) {
        let _ = self.tx.send(Out::Notification(Box::new(n)));
    }
}

/// Convenience for a writer that is not `Send` (e.g. a stdout lock): the
/// caller owns the loop. Used by tests and by the real server, which
/// holds `io::Stdout` (which is `Send`).
pub fn write_line<W: Write>(writer: &mut W, line: &str) -> io::Result<()> {
    writeln!(writer, "{line}")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use willie_proto::rpc::RpcError;

    #[test]
    fn responses_and_notifications_reach_the_writer_in_order() {
        let buf = SharedBuf::default();
        let (out, handle) = Outbound::spawn(buf.clone());
        out.send_response(Response::err(1, RpcError::new("x", "y")));
        out.send_notification(Notification::new(
            "state.event",
            serde_json::Value::Null,
        ));
        drop(out);
        handle.join().unwrap();
        let text = buf.contents();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"id\":1"));
        assert!(lines[1].contains("state.event"));
    }

    #[derive(Clone, Default)]
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl SharedBuf {
        fn contents(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }
    impl Write for SharedBuf {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
