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
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok)
                {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })?;
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
}

/// A `wsl.exe --exec` child with piped stdio and collected stderr.
#[derive(Debug)]
pub struct WslProcess {
    child: Child,
    stderr: Receiver<String>,
}

impl WslProcess {
    pub fn spawn(exec: &WslExec) -> Result<Self, WslError> {
        let mut child = wsl_command()
            .args(exec.to_args())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => WslError::NotInstalled(e),
                _ => WslError::Io(e),
            })?;
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
        Ok(Self { child, stderr: rx })
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

    /// Whatever the process wrote to stderr so far (decoded).
    pub fn stderr_text(&mut self) -> String {
        self.stderr.try_iter().collect::<Vec<_>>().join("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    /// Re-runs this test binary as an echo peer: spawning `--exact
    /// <test_name>` with `WILLIE_ECHO_MODE` set makes that same test
    /// function become `echo_main` instead of exercising the transport.
    /// Keeps the test hermetic on every host, no `--echo` flag needed.
    /// `--exact` needs the module-qualified name to match exactly one
    /// test; the bare function name matches none.
    fn spawn_echo(test_name: &str) -> Child {
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &format!("process::tests::{test_name}")])
            .env("WILLIE_ECHO_MODE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// The test harness prints its own startup banner to the child's real
    /// stdout before the echo peer's body runs, with no stable flag to
    /// silence it. Drain lines until the peer's own readiness marker
    /// instead of assuming the banner's exact wording or line count.
    fn await_echo_ready(transport: &mut LineTransport) {
        loop {
            match transport.recv_line(Duration::from_secs(5)).unwrap() {
                Some(line) if line == ECHO_READY => return,
                Some(_) => continue,
                None => panic!("echo peer exited before signalling ready"),
            }
        }
    }

    const ECHO_READY: &str = "@@willie-echo-ready@@";

    #[test]
    fn lines_round_trip_through_the_transport() {
        if std::env::var_os("WILLIE_ECHO_MODE").is_some() {
            echo_main();
            return;
        }
        let mut child = spawn_echo("lines_round_trip_through_the_transport");
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_echo_ready(&mut transport);
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
        let mut child = spawn_echo("eof_is_reported_as_none");
        let mut transport = LineTransport::from_child(&mut child).unwrap();
        await_echo_ready(&mut transport);
        transport.close_input();
        assert_eq!(transport.recv_line(Duration::from_secs(5)).unwrap(), None);
        child.wait().unwrap();
    }

    fn echo_main() {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        writeln!(stdout, "{ECHO_READY}").unwrap();
        stdout.flush().unwrap();
        for line in stdin.lock().lines().map_while(Result::ok) {
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        }
        std::process::exit(0);
    }
}
