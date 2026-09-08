//! The embedded terminals: sessions rendered inside the Willie window.
//! One bridge per live session — the Session screen keeps every tab
//! mounted, so they all stream at once. The engine spawns `willie attach
//! --host` as a child per bridge, streams its stdout to a sink the app
//! turns into `session://output`, and writes `hostterm` frames to its
//! stdin for input and resize. Closing detaches (the session keeps
//! running).

#[cfg(any(windows, test))]
use std::collections::HashMap;

use willie_core::id::SessionId;

use crate::error::EngineError;

/// `wsl.exe` args that run `willie attach <id> --host` in the distro.
#[must_use]
pub fn attach_host_argv(id: &str) -> Vec<String> {
    vec![
        "-d".into(),
        crate::wsl::DISTRO_NAME.to_owned(),
        "--user".into(),
        "willie".into(),
        "--exec".into(),
        crate::terminal::WILLIE_BIN.to_owned(),
        "attach".into(),
        id.to_owned(),
        "--host".into(),
    ]
}

#[cfg(windows)]
fn failed(message: impl Into<String>) -> EngineError {
    EngineError::EmbeddedTerminal {
        message: message.into(),
    }
}

/// What [`Bridges`] needs from a bridge: a frame to write and a way to
/// tear it down. The Windows child process implements it; a test
/// implements it with a recorder, so the routing below is provable
/// without a `wsl.exe` in the loop.
#[cfg(any(windows, test))]
pub(crate) trait Bridge {
    fn write_frame(&mut self, frame: &[u8]);
    fn close(self);
}

/// Every open embedded terminal, keyed by the session it renders. One
/// slot per session is the invariant: opening a second session must
/// never tear down the first one's stream, and a frame for a session
/// with no bridge is an error, never a silent success.
#[cfg(any(windows, test))]
#[derive(Debug)]
pub(crate) struct Bridges<T> {
    open: HashMap<SessionId, T>,
}

#[cfg(any(windows, test))]
impl<T> Default for Bridges<T> {
    fn default() -> Self {
        Self {
            open: HashMap::new(),
        }
    }
}

#[cfg(any(windows, test))]
impl<T: Bridge> Bridges<T> {
    /// Replaces only this session's own bridge — a re-open of a tab
    /// whose child died — and leaves every other session streaming.
    fn insert(&mut self, id: SessionId, bridge: T) {
        if let Some(stale) = self.open.remove(&id) {
            stale.close();
        }
        self.open.insert(id, bridge);
    }

    fn write(
        &mut self,
        id: SessionId,
        frame: &[u8],
    ) -> Result<(), EngineError> {
        match self.open.get_mut(&id) {
            Some(bridge) => {
                bridge.write_frame(frame);
                Ok(())
            }
            None => {
                Err(EngineError::EmbeddedTerminalNotOpen { id: id.to_string() })
            }
        }
    }

    fn remove(&mut self, id: SessionId) {
        if let Some(bridge) = self.open.remove(&id) {
            bridge.close();
        }
    }

    /// Every bridge is a child process of this app; the window closing
    /// must not leave one behind.
    fn close_all(&mut self) {
        for (_, bridge) in self.open.drain() {
            bridge.close();
        }
    }
}

#[cfg(windows)]
pub(crate) mod imp {
    use std::io::{Read, Write};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::thread::JoinHandle;

    use willie_core::id::SessionId;
    use willie_proto::hostterm;

    use super::{Bridge, attach_host_argv, failed};
    use crate::error::EngineError;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// One live embedded terminal. `Debug` so the `Engine` that holds
    /// the bridges keeps its own `#[derive(Debug)]` (`Child`,
    /// `ChildStdin` and `JoinHandle` are all `Debug`).
    #[derive(Debug)]
    pub struct Embedded {
        child: Child,
        stdin: ChildStdin,
        _reader: JoinHandle<()>,
    }

    impl Embedded {
        pub fn open(
            id: SessionId,
            sink: impl Fn(Vec<u8>) + Send + 'static,
        ) -> Result<Self, EngineError> {
            let mut child = Command::new("wsl.exe")
                .args(attach_host_argv(&id.to_string()))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn()
                .map_err(|e| failed(e.to_string()))?;
            let stdin = child.stdin.take().ok_or_else(|| failed("no stdin"))?;
            let mut stdout =
                child.stdout.take().ok_or_else(|| failed("no stdout"))?;
            let reader = std::thread::Builder::new()
                .name("embed-out".to_owned())
                .spawn(move || {
                    let mut buf = [0u8; 16 * 1024];
                    loop {
                        match stdout.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => sink(buf[..n].to_vec()),
                        }
                    }
                })
                .map_err(|e| failed(e.to_string()))?;
            Ok(Self {
                child,
                stdin,
                _reader: reader,
            })
        }
    }

    impl Bridge for Embedded {
        fn write_frame(&mut self, frame: &[u8]) {
            // A dead child just means the terminal ended; the next stdout
            // EOF/close path reports it. Ignore the write error here.
            let _ = self.stdin.write_all(frame);
            let _ = self.stdin.flush();
        }

        fn close(mut self) {
            // Dropping stdin sends EOF → attach detaches cleanly and the
            // session keeps running; kill+reap as a fallback.
            drop(self.stdin);
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    pub fn input_frame(bytes: &[u8]) -> Vec<u8> {
        hostterm::encode_input(bytes)
    }
    pub fn resize_frame(rows: u16, cols: u16) -> Vec<u8> {
        hostterm::encode_resize(rows, cols)
    }
}

#[cfg(windows)]
impl crate::engine::Engine {
    pub fn session_terminal_open(
        &mut self,
        id: SessionId,
        sink: impl Fn(Vec<u8>) + Send + 'static,
    ) -> Result<(), EngineError> {
        let bridge = imp::Embedded::open(id, sink)?;
        self.embedded.insert(id, bridge);
        Ok(())
    }

    pub fn session_terminal_input(
        &mut self,
        id: SessionId,
        data: &[u8],
    ) -> Result<(), EngineError> {
        self.embedded.write(id, &imp::input_frame(data))
    }

    pub fn session_terminal_resize(
        &mut self,
        id: SessionId,
        rows: u16,
        cols: u16,
    ) -> Result<(), EngineError> {
        self.embedded.write(id, &imp::resize_frame(rows, cols))
    }

    pub fn session_terminal_close(&mut self, id: SessionId) {
        self.embedded.remove(id);
    }

    /// Quitting the app leaves no `wsl.exe attach` child behind.
    pub(crate) fn close_embedded_terminals(&mut self) {
        self.embedded.close_all();
    }
}

#[cfg(not(windows))]
impl crate::engine::Engine {
    pub fn session_terminal_open(
        &mut self,
        _id: SessionId,
        _sink: impl Fn(Vec<u8>) + Send + 'static,
    ) -> Result<(), EngineError> {
        Err(EngineError::EmbeddedTerminal {
            message: "the embedded terminal is Windows-only".into(),
        })
    }
    pub fn session_terminal_input(
        &mut self,
        id: SessionId,
        _data: &[u8],
    ) -> Result<(), EngineError> {
        Err(EngineError::EmbeddedTerminalNotOpen { id: id.to_string() })
    }
    pub fn session_terminal_resize(
        &mut self,
        id: SessionId,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), EngineError> {
        Err(EngineError::EmbeddedTerminalNotOpen { id: id.to_string() })
    }
    pub fn session_terminal_close(&mut self, _id: SessionId) {}
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    /// A bridge that records what reached it and whether it was closed,
    /// so a test can tell "wrote to the right one" from "tore the other
    /// one down" without spawning a process.
    #[derive(Debug, Default)]
    struct Recorder {
        frames: Vec<Vec<u8>>,
        closed: bool,
    }

    #[derive(Debug, Clone)]
    struct FakeBridge(Rc<RefCell<Recorder>>);

    impl FakeBridge {
        fn new() -> Self {
            Self(Rc::new(RefCell::new(Recorder::default())))
        }

        fn frames(&self) -> Vec<Vec<u8>> {
            self.0.borrow().frames.clone()
        }

        fn closed(&self) -> bool {
            self.0.borrow().closed
        }
    }

    impl Bridge for FakeBridge {
        fn write_frame(&mut self, frame: &[u8]) {
            self.0.borrow_mut().frames.push(frame.to_vec());
        }

        fn close(self) {
            self.0.borrow_mut().closed = true;
        }
    }

    #[test]
    fn the_host_argv_runs_attach_with_the_host_flag() {
        let a = attach_host_argv("sess_01J");
        assert_eq!(a[0], "-d");
        assert_eq!(a[1], crate::wsl::DISTRO_NAME);
        assert!(a.windows(2).any(|w| w == ["--user", "willie"]));
        assert!(
            a.windows(2)
                .any(|w| w == ["--exec", crate::terminal::WILLIE_BIN])
        );
        assert!(a.windows(2).any(|w| w == ["attach", "sess_01J"]));
        assert_eq!(a.last().unwrap(), "--host");
    }

    /// The whole point of one bridge per session: the Session screen
    /// mounts every live tab, so a second open may not detach the first
    /// one, and input must keep reaching the tab it was typed into.
    #[test]
    fn a_second_open_leaves_the_first_bridge_streaming() {
        let (first, second) = (SessionId::new(), SessionId::new());
        let (a, b) = (FakeBridge::new(), FakeBridge::new());
        let mut bridges = Bridges::default();

        bridges.insert(first, a.clone());
        bridges.insert(second, b.clone());

        assert!(!a.closed(), "opening another session closed the first");
        bridges.write(first, b"hello").expect("first still bridged");
        bridges
            .write(second, b"there")
            .expect("second still bridged");
        assert_eq!(a.frames(), vec![b"hello".to_vec()]);
        assert_eq!(b.frames(), vec![b"there".to_vec()]);
    }

    /// Re-opening the same tab replaces its own child (the previous one
    /// is detached) and touches no other session.
    #[test]
    fn re_opening_one_session_replaces_only_its_own_bridge() {
        let (first, second) = (SessionId::new(), SessionId::new());
        let (stale, fresh, other) =
            (FakeBridge::new(), FakeBridge::new(), FakeBridge::new());
        let mut bridges = Bridges::default();

        bridges.insert(first, stale.clone());
        bridges.insert(second, other.clone());
        bridges.insert(first, fresh.clone());

        assert!(stale.closed());
        assert!(!other.closed());
        bridges.write(first, b"x").expect("re-opened");
        assert_eq!(fresh.frames(), vec![b"x".to_vec()]);
    }

    /// Fail closed: a tab whose bridge is gone must hear about it
    /// instead of typing into a successful-looking void.
    #[test]
    fn writing_to_a_session_with_no_bridge_is_a_typed_error() {
        let mut bridges: Bridges<FakeBridge> = Bridges::default();

        let err = bridges.write(SessionId::new(), b"x").unwrap_err();

        assert_eq!(err.code(), "embedded_terminal_not_open");
        assert!(!err.remediation().is_empty());
    }

    #[test]
    fn closing_one_session_keeps_the_others_and_close_all_ends_them() {
        let (first, second) = (SessionId::new(), SessionId::new());
        let (a, b) = (FakeBridge::new(), FakeBridge::new());
        let mut bridges = Bridges::default();
        bridges.insert(first, a.clone());
        bridges.insert(second, b.clone());

        bridges.remove(first);

        assert!(a.closed());
        assert!(!b.closed());
        assert_eq!(
            bridges.write(first, b"x").unwrap_err().code(),
            "embedded_terminal_not_open"
        );

        bridges.close_all();

        assert!(b.closed());
        assert_eq!(
            bridges.write(second, b"x").unwrap_err().code(),
            "embedded_terminal_not_open"
        );
    }
}
