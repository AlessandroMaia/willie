//! The embedded terminal: a session rendered inside the Willie window.
//! Exactly one is active at a time. The engine spawns `willie attach
//! --host` as a child, streams its stdout to a sink the app turns into
//! `session://output`, and writes `hostterm` frames to its stdin for
//! input and resize. Closing detaches (the session keeps running).

#[cfg(windows)]
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

#[cfg(windows)]
pub(crate) mod imp {
    use std::io::{Read, Write};
    use std::os::windows::process::CommandExt;
    use std::process::{Child, ChildStdin, Command, Stdio};
    use std::thread::JoinHandle;

    use willie_core::id::SessionId;
    use willie_proto::hostterm;

    use super::{attach_host_argv, failed};
    use crate::error::EngineError;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// The one live embedded terminal. `Debug` so the `Engine` that holds
    /// `Option<Embedded>` keeps its own `#[derive(Debug)]` (`Child`,
    /// `ChildStdin` and `JoinHandle` are all `Debug`).
    #[derive(Debug)]
    pub struct Embedded {
        id: SessionId,
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
                id,
                child,
                stdin,
                _reader: reader,
            })
        }

        pub fn id(&self) -> SessionId {
            self.id
        }

        pub fn write_frame(&mut self, frame: &[u8]) {
            // A dead child just means the terminal ended; the next stdout
            // EOF/close path reports it. Ignore the write error here.
            let _ = self.stdin.write_all(frame);
            let _ = self.stdin.flush();
        }

        pub fn close(mut self) {
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
        if let Some(prev) = self.embedded.take() {
            prev.close();
        }
        self.embedded = Some(imp::Embedded::open(id, sink)?);
        Ok(())
    }

    pub fn session_terminal_input(&mut self, id: SessionId, data: &[u8]) {
        if let Some(t) = self.embedded.as_mut()
            && t.id() == id
        {
            t.write_frame(&imp::input_frame(data));
        }
    }

    pub fn session_terminal_resize(
        &mut self,
        id: SessionId,
        rows: u16,
        cols: u16,
    ) {
        if let Some(t) = self.embedded.as_mut()
            && t.id() == id
        {
            t.write_frame(&imp::resize_frame(rows, cols));
        }
    }

    pub fn session_terminal_close(&mut self, id: SessionId) {
        if self.embedded.as_ref().is_some_and(|t| t.id() == id)
            && let Some(t) = self.embedded.take()
        {
            t.close();
        }
    }
}

#[cfg(not(windows))]
impl crate::engine::Engine {
    pub fn session_terminal_open(
        &mut self,
        _id: willie_core::id::SessionId,
        _sink: impl Fn(Vec<u8>) + Send + 'static,
    ) -> Result<(), EngineError> {
        Err(EngineError::EmbeddedTerminal {
            message: "the embedded terminal is Windows-only".into(),
        })
    }
    pub fn session_terminal_input(
        &mut self,
        _id: willie_core::id::SessionId,
        _data: &[u8],
    ) {
    }
    pub fn session_terminal_resize(
        &mut self,
        _id: willie_core::id::SessionId,
        _rows: u16,
        _cols: u16,
    ) {
    }
    pub fn session_terminal_close(&mut self, _id: willie_core::id::SessionId) {}
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
