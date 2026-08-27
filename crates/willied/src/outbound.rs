//! One thread owns the daemon's stdout so responses and notifications
//! never interleave. Everything else sends through a channel.

use std::{
    io::Write,
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
};

use willie_proto::{
    rpc::{Notification, Response},
    state::{self, Event},
};

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

    /// The one place an [`Event`] becomes a wire notification, so the whole
    /// daemon serialises state changes the same way. Job and project code
    /// both funnel through here.
    pub fn send_event(&self, event: Event) {
        let params =
            serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
        self.send_notification(Notification::new(state::method::EVENT, params));
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use willie_core::id::ProjectId;
    use willie_proto::{
        rpc::RpcError,
        state::{Event, EventKind},
    };

    #[test]
    fn send_event_writes_a_state_event_notification() {
        let buf = SharedBuf::default();
        let (out, handle) = Outbound::spawn(buf.clone());
        out.send_event(Event {
            seq: 3,
            kind: EventKind::ProjectRemoved {
                id: ProjectId::new(),
            },
        });
        drop(out);
        handle.join().unwrap();
        let text = buf.contents();
        assert!(text.contains("state.event"), "{text}");
        assert!(text.contains("\"seq\":3"), "{text}");
        assert!(text.contains("project_removed"), "{text}");
    }

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
