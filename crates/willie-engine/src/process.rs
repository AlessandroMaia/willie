//! A program running inside the distribution, reached through pipes.
//!
//! `wsl.exe --exec` forwards the program's stdin/stdout/stderr unchanged,
//! so the engine talks ndjson to `willied` exactly as it would to a local
//! process. Reading happens on a thread so callers get timeouts.

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, Stdio},
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::Duration,
};

use crate::{
    error::WslError,
    wsl::{WslExec, wsl_command},
};

/// One UTF-8 line per message in each direction.
#[derive(Debug)]
pub struct LineTransport {
    input: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl LineTransport {
    /// Takes the child's stdin and stdout. Fails if either was not piped.
    pub fn from_child(child: &mut Child) -> io::Result<Self> {
        let input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("stdin not piped"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("stdout not piped"))?;
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("willie-transport-reader".into())
            .spawn(move || read_lines(BufReader::new(stdout), &tx))?;
        Ok(Self {
            input: Some(input),
            lines: rx,
        })
    }

    pub fn send_line(&mut self, line: &str) -> io::Result<()> {
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| io::Error::other("input closed"))?;
        input.write_all(line.as_bytes())?;
        input.write_all(b"\n")?;
        input.flush()
    }

    /// `Ok(None)` means the peer closed its stdout (EOF).
    pub fn recv_line(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<String>, RecvTimeoutError> {
        match self.lines.recv_timeout(timeout) {
            Ok(line) => Ok(Some(line)),
            Err(RecvTimeoutError::Disconnected) => Ok(None),
            Err(RecvTimeoutError::Timeout) => Err(RecvTimeoutError::Timeout),
        }
    }

    /// Closes the peer's stdin so a well-behaved program exits on EOF.
    pub fn close_input(&mut self) {
        self.input = None;
    }

    /// Hands the write half (the peer's stdin) and the read half (the
    /// decoded line stream) to separate owners. The RPC router blocks a
    /// thread on the reads while calls keep sending, so a blocking read
    /// never holds up a send; splitting the two makes that safe.
    pub(crate) fn split(self) -> (Option<ChildStdin>, Receiver<String>) {
        (self.input, self.lines)
    }
}

/// Forwards one line per read until EOF or a read error.
///
/// The daemon writes UTF-8, but `wsl.exe` writes its own failures to the
/// same stdout as UTF-16LE; reading bytes and decoding every line keeps
/// those messages instead of ending the stream on the first one. The
/// decoder tells the two encodings apart, so it is asked unconditionally:
/// bytes that are ASCII with interleaved NULs are valid UTF-8 too, and
/// trying UTF-8 first would let them through undecoded.
fn read_lines<R: BufRead>(mut reader: R, tx: &mpsc::Sender<String>) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        // A UTF-16LE newline is the pair `0A 00`, so splitting on the
        // `\n` byte leaves its high byte at the head of the next read;
        // without dropping it every later code unit is off by one.
        let start = buf.iter().position(|b| *b != 0).unwrap_or(buf.len());
        let text = crate::text::decode_wsl_output(&buf[start..]);
        let line = text.trim_matches(['\n', '\r', '\0']).to_owned();
        if tx.send(line).is_err() {
            return;
        }
    }
}

/// A `wsl.exe --exec` child with piped stdio and collected stderr.
#[derive(Debug)]
pub struct WslProcess {
    child: Child,
    stderr_rx: Receiver<String>,
    stderr_cache: Option<String>,
}

impl WslProcess {
    pub fn spawn(exec: &WslExec) -> Result<Self, WslError> {
        let child = wsl_command()
            .args(exec.to_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => WslError::NotInstalled(e),
                _ => WslError::Io(e),
            })?;
        Self::from_child(child)
    }

    /// Wraps an already-spawned child with piped stderr and starts the
    /// background reader thread; shared by `spawn` and by tests that
    /// need a `WslProcess` without an actual WSL installation.
    pub(crate) fn from_child(mut child: Child) -> Result<Self, WslError> {
        let (tx, rx) = mpsc::channel();
        if let Some(mut err) = child.stderr.take() {
            thread::Builder::new()
                .name("willie-process-stderr".into())
                .spawn(move || {
                    let mut buf = Vec::new();
                    let _ = err.read_to_end(&mut buf);
                    let _ = tx.send(crate::text::decode_wsl_output(&buf));
                })?;
        }
        Ok(Self {
            child,
            stderr_rx: rx,
            stderr_cache: None,
        })
    }

    pub fn transport(&mut self) -> Result<LineTransport, WslError> {
        Ok(LineTransport::from_child(&mut self.child)?)
    }

    pub fn try_wait(&mut self) -> Result<Option<i32>, WslError> {
        Ok(self.child.try_wait()?.map(|s| s.code().unwrap_or(-1)))
    }

    pub fn wait(&mut self) -> Result<i32, WslError> {
        Ok(self.child.wait()?.code().unwrap_or(-1))
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }

    /// Whatever the process wrote to stderr, read once the pipe closes
    /// (the process is dead, or killed and reaped) and cached from
    /// then on so repeated calls return the same text.
    pub fn stderr_text(&mut self) -> String {
        if let Some(text) = &self.stderr_cache {
            return text.clone();
        }
        let text = self
            .stderr_rx
            .recv_timeout(Duration::from_millis(500))
            .unwrap_or_default();
        self.stderr_cache = Some(text.clone());
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{announce_ready, await_ready, spawn_peer};

    #[test]
    fn lines_round_trip_through_the_transport() {
        if std::env::var_os("WILLIE_ECHO_MODE").is_some() {
            echo_main();
            return;
        }
        let mut child = spawn_peer(
            "process::tests::lines_round_trip_through_the_transport",
            "WILLIE_ECHO_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_ready(&mut transport);
        transport.send_line("{\"id\":1} Versão ✓").unwrap();
        let got = transport.recv_line(Duration::from_secs(5)).unwrap();
        assert_eq!(got.as_deref(), Some("{\"id\":1} Versão ✓"));
        drop(transport);
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn eof_is_reported_as_none() {
        if std::env::var_os("WILLIE_ECHO_MODE").is_some() {
            echo_main();
            return;
        }
        let mut child = spawn_peer(
            "process::tests::eof_is_reported_as_none",
            "WILLIE_ECHO_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_ready(&mut transport);
        transport.close_input();
        assert_eq!(transport.recv_line(Duration::from_secs(5)).unwrap(), None);
        child.wait().unwrap();
    }

    fn echo_main() {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        announce_ready();
        for line in stdin.lock().lines().map_while(Result::ok) {
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        }
        std::process::exit(0);
    }

    /// `wsl.exe` writes its own errors to the child's stdout as UTF-16LE.
    /// A byte-oriented reader must decode every one of them and keep
    /// going: a dropped line leaves the failure with nothing to report,
    /// and a line shifted by one byte reads as interleaved NULs, which
    /// no remediation can be matched against.
    #[test]
    fn utf16_stdout_lines_are_decoded_not_dropped() {
        if std::env::var_os("WILLIE_UTF16_MODE").is_some() {
            crate::test_support::write_utf16_message_and_exit();
        }
        let mut child = spawn_peer(
            "process::tests::utf16_stdout_lines_are_decoded_not_dropped",
            "WILLIE_UTF16_MODE",
        );
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        let mut seen = Vec::new();
        while let Some(line) =
            transport.recv_line(Duration::from_secs(5)).unwrap()
        {
            if !line.is_empty() {
                seen.push(line);
            }
        }
        assert!(
            seen.iter().all(|l| !l.contains('\0')),
            "a line arrived undecoded: {seen:?}"
        );
        // Both lines of the message, in both shapes the peer wrote.
        for expected in crate::test_support::DISTRO_NOT_FOUND {
            let hits = seen.iter().filter(|l| *l == expected).count();
            assert_eq!(hits, 2, "expected `{expected}` twice in {seen:?}");
        }
        assert_eq!(child.wait().unwrap().code(), Some(127));
    }

    /// One stream carries the daemon's UTF-8 ndjson and `wsl.exe`'s own
    /// UTF-16LE messages, with or without a BOM, and a last line that
    /// never got its newline. Every shape has to come out exactly.
    #[test]
    fn the_reader_decodes_each_line_by_its_own_encoding() {
        fn utf16le(text: &str, bom: bool) -> Vec<u8> {
            let mut out = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
            for unit in text.encode_utf16() {
                out.extend_from_slice(&unit.to_le_bytes());
            }
            out
        }
        let json = "{\"id\":1,\"text\":\"Versão ✓\"}";
        let mut bytes = format!("{json}\n").into_bytes();
        bytes.extend(utf16le("with a bom\r\n", true));
        bytes.extend(utf16le("without one\r\n", false));
        bytes.extend(utf16le("no trailing newline", false));

        let (tx, rx) = mpsc::channel();
        read_lines(&bytes[..], &tx);
        drop(tx);
        let lines: Vec<String> =
            rx.into_iter().filter(|l| !l.is_empty()).collect();
        assert_eq!(
            lines,
            [json, "with a bom", "without one", "no trailing newline"]
        );
    }

    #[test]
    fn stderr_text_is_read_once_the_process_has_died() {
        if std::env::var_os("WILLIE_STDERR_MODE").is_some() {
            // `eprint!` goes through the harness's capture hook and
            // never reaches the real pipe; write the handle directly.
            write!(io::stderr(), "boom").unwrap();
            std::process::exit(1);
        }
        let child = crate::test_support::spawn_peer_with_stderr(
            "process::tests::stderr_text_is_read_once_the_process_has_died",
            "WILLIE_STDERR_MODE",
        );
        let mut process = WslProcess::from_child(child).unwrap();
        process.wait().unwrap();
        assert_eq!(process.stderr_text(), "boom");
    }
}
