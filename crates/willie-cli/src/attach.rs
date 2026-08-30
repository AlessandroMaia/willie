//! `willie attach` — the terminal side of a session.
//!
//! The client is deliberately thin: it puts the local terminal in raw mode,
//! forwards every byte it reads to the supervisor and writes back whatever
//! the PTY produced. No interpretation, no line buffering, `OPOST` off, so
//! what a full-screen program draws is what the user sees.
//!
//! Both directions are framed (`willie_linux::wire`): the client sends
//! `hello`, `input`, `resize`, `detach`; the supervisor sends `output` and,
//! last, `closed` with the reason.

use std::{
    io,
    net::Shutdown,
    os::{fd::AsRawFd, unix::net::UnixStream},
    process::ExitCode,
    ptr,
    sync::atomic::{AtomicBool, AtomicPtr, Ordering},
    thread,
};

use willie_linux::wire;
use willie_proto::supervisor::{CloseReason, Closed, Hello, Role};

/// Byte that means "leave, but keep the session": `Ctrl-]`.
const DETACH_KEY: u8 = 0x1D;

/// The terminal settings to put back, published for the signal handlers.
/// A raw pointer rather than a lock: a handler may run at any moment and
/// must not be able to block on a mutex the interrupted thread holds.
static SAVED: AtomicPtr<libc::termios> = AtomicPtr::new(ptr::null_mut());

/// Set by the `SIGWINCH` handler; the input loop notices the interrupted
/// read and sends a new size.
static RESIZED: AtomicBool = AtomicBool::new(false);

/// Set when the user asked to detach, before the frame goes out, so the
/// output thread does not announce the end of a session that is still
/// running.
static LEAVING: AtomicBool = AtomicBool::new(false);

fn errno() -> io::Error {
    io::Error::last_os_error()
}

/// Put the saved settings back, at most once. Safe to call from a signal
/// handler: one atomic swap and one `ioctl`.
fn restore_terminal() {
    let saved = SAVED.swap(ptr::null_mut(), Ordering::SeqCst);
    if !saved.is_null() {
        // SAFETY: `saved` was published by `raw_mode` from a leaked box and
        // is taken exactly once thanks to the swap.
        unsafe { libc::tcsetattr(0, libc::TCSANOW, saved) };
    }
}

/// Restores the terminal when the scope ends, including while unwinding.
#[derive(Debug)]
struct RawGuard;

impl Drop for RawGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

extern "C" fn on_winch(_signal: libc::c_int) {
    RESIZED.store(true, Ordering::SeqCst);
}

extern "C" fn on_hangup(signal: libc::c_int) {
    restore_terminal();
    // SAFETY: the only safe thing left to do in a handler that must not
    // return: skip destructors and leave with the conventional code.
    unsafe { libc::_exit(128 + signal) }
}

/// Install a handler without `SA_RESTART`, so a blocking `read` returns
/// `EINTR` and the loop gets a chance to look at the flag.
fn install(signal: libc::c_int, handler: extern "C" fn(libc::c_int)) {
    // SAFETY: `action` is fully initialised before use and the handler has
    // the signature the kernel expects.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as libc::sighandler_t;
        libc::sigemptyset(&mut action.sa_mask);
        action.sa_flags = 0;
        libc::sigaction(signal, &action, ptr::null_mut());
    }
}

/// Switch descriptor 0 to raw mode and remember how it was.
fn raw_mode() -> io::Result<RawGuard> {
    // SAFETY: `original` is filled by `tcgetattr` before being read.
    let original = unsafe {
        let mut original: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut original) != 0 {
            return Err(errno());
        }
        original
    };
    let mut raw = original;
    // SAFETY: `raw` is a valid, initialised structure.
    unsafe {
        libc::cfmakeraw(&mut raw);
        if libc::tcsetattr(0, libc::TCSANOW, &raw) != 0 {
            return Err(errno());
        }
    }
    SAVED.store(Box::into_raw(Box::new(original)), Ordering::SeqCst);
    Ok(RawGuard)
}

/// Size of the local terminal, or `None` when descriptor 0 is not one.
fn window_size() -> Option<(u16, u16)> {
    // SAFETY: `ws` is filled by the ioctl before being read.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(0, libc::TIOCGWINSZ as _, &mut ws) != 0 {
            return None;
        }
        Some((ws.ws_row, ws.ws_col))
    }
}

fn is_a_terminal(fd: libc::c_int) -> bool {
    // SAFETY: `isatty` only inspects the descriptor.
    unsafe { libc::isatty(fd) == 1 }
}

/// Write every byte to a descriptor. Used instead of `std::io::Stdout`
/// because that one is line buffered, which would hold a redraw back until
/// a newline arrives.
fn write_all_fd(fd: libc::c_int, buf: &[u8]) -> io::Result<()> {
    let mut done = 0;
    while done < buf.len() {
        // SAFETY: the slice is valid for the length passed.
        let n = unsafe {
            libc::write(fd, buf[done..].as_ptr().cast(), buf.len() - done)
        };
        if n > 0 {
            done += n as usize;
            continue;
        }
        let e = errno();
        if e.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(e);
    }
    Ok(())
}

/// Read from a descriptor, reporting `EINTR` instead of hiding it: an
/// interrupted read is how this client learns the window changed.
fn read_fd(fd: libc::c_int, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: the buffer is valid for `buf.len()` bytes.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n >= 0 { Ok(n as usize) } else { Err(errno()) }
}

/// Decode frames from the supervisor: `output` goes to the terminal
/// untouched, `closed` says how it ended. EOF without a `closed` is a
/// lost connection.
fn output_loop(mut socket: UnixStream) {
    use std::io::Read;
    let mut decoder = wire::Decoder::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        while let Some(frame) = decoder.pop() {
            match frame.kind {
                wire::OUTPUT => {
                    if write_all_fd(1, &frame.payload).is_err() {
                        finish(
                            Some(b"willie: cannot write to the terminal"),
                            1,
                        );
                    }
                }
                wire::CLOSED => {
                    let closed: Option<Closed> =
                        wire::decode_json(&frame.payload).ok();
                    let (text, code) = describe(closed.as_ref());
                    finish(Some(text.as_bytes()), code);
                }
                _ => {}
            }
        }
        match socket.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => decoder.push(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    if LEAVING.load(Ordering::SeqCst) {
        finish(Some(b"willie: detached, the session keeps running"), 0);
    }
    finish(Some(b"willie: connection to the session lost"), 1);
}

/// The stderr line and exit code for a `closed` frame. A clean end (exit
/// 0, a stop, a shutdown) exits 0 so the terminal tab closes; anything
/// the user should look at exits 1 so the tab stays open with the text.
fn describe(closed: Option<&Closed>) -> (String, u8) {
    let Some(closed) = closed else {
        return ("willie: session ended".to_owned(), 1);
    };
    match closed.reason {
        CloseReason::Exited => match (closed.code, closed.signal) {
            (Some(0), _) => ("willie: session ended".to_owned(), 0),
            (Some(code), _) => {
                (format!("willie: session exited with code {code}"), 1)
            }
            (None, Some(signal)) => {
                (format!("willie: session ended by signal {signal}"), 1)
            }
            (None, None) => ("willie: session ended".to_owned(), 1),
        },
        CloseReason::Stopped => ("willie: session stopped".to_owned(), 0),
        CloseReason::Shutdown => {
            ("willie: supervisor shutting down".to_owned(), 0)
        }
        CloseReason::TooSlow => (
            "willie: connection too slow, reattach with the same command"
                .to_owned(),
            1,
        ),
        CloseReason::Protocol => {
            ("willie: the supervisor refused this client".to_owned(), 1)
        }
    }
}

/// Restore the terminal, print the reason, exit with `code`. Called from
/// whichever side notices the end first.
fn finish(text: Option<&[u8]>, code: u8) -> ! {
    restore_terminal();
    if let Some(text) = text {
        let _ = write_all_fd(2, text);
        let _ = write_all_fd(2, b"\r\n");
    }
    std::process::exit(i32::from(code));
}

/// Attach to the supervisor listening on `target`. In host mode (`host`)
/// the client has no terminal of its own: stdin carries `hostterm` control
/// frames instead of raw keystrokes, and stdout must carry only session
/// bytes, so no status line is printed there.
pub fn run(
    target: &std::path::Path,
    size: Option<(u16, u16)>,
    raw: bool,
    host: bool,
) -> ExitCode {
    let stream = match UnixStream::connect(target) {
        Ok(stream) => stream,
        Err(_) => {
            let name = target
                .file_stem()
                .unwrap_or(target.as_os_str())
                .to_string_lossy();
            eprintln!("willie attach: session {name} is not running");
            return ExitCode::from(1);
        }
    };
    if !host && raw && !is_a_terminal(0) {
        eprintln!(
            "willie attach: stdin is not a terminal; use --no-raw or run \
             this from a terminal"
        );
        return ExitCode::from(1);
    }
    let _guard = if !host && raw {
        match raw_mode() {
            Ok(guard) => Some(guard),
            Err(e) => {
                eprintln!("willie attach: cannot set raw mode: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        None
    };
    if !host {
        install(libc::SIGWINCH, on_winch);
    }
    install(libc::SIGTERM, on_hangup);
    install(libc::SIGHUP, on_hangup);

    let writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(e) => {
            eprintln!("willie attach: cannot split the socket: {e}");
            return ExitCode::from(1);
        }
    };
    let reader = stream;
    let fd = writer.as_raw_fd();

    let (rows, cols) = size.or_else(window_size).unwrap_or((24, 80));
    let hello = Hello {
        role: Role::Terminal,
        rows,
        cols,
    };
    let Ok(frame) = wire::encode_json(wire::HELLO, &hello) else {
        eprintln!("willie attach: cannot encode the hello");
        return ExitCode::from(1);
    };
    if write_all_fd(fd, &frame).is_err() {
        eprintln!("willie attach: cannot send the hello");
        return ExitCode::from(1);
    }
    let _ = write_all_fd(2, b"willie: detach with Ctrl-]\r\n");

    let output = thread::Builder::new()
        .name("output".to_owned())
        .spawn(move || output_loop(reader));
    if let Err(e) = output {
        eprintln!("willie attach: cannot start the output thread: {e}");
        return ExitCode::from(1);
    }

    let code = if host {
        host_input_loop(fd)
    } else {
        input_loop(fd, size)
    };
    if code == 0 {
        // Detach (or stdin ended): the output thread ends the process
        // when the socket closes; give it a moment, then leave anyway.
        let _ = writer.shutdown(Shutdown::Write);
        thread::sleep(std::time::Duration::from_millis(200));
        // Host stdout carries only session bytes: the interactive
        // "detached…" line would corrupt the app's byte stream.
        if host {
            finish(None, 0);
        } else {
            finish(Some(b"willie: detached, the session keeps running"), 0);
        }
    }
    ExitCode::from(code)
}

/// Host mode: read `hostterm` frames from stdin and forward them to the
/// supervisor as wire frames. Output still flows through `output_loop`.
/// EOF on stdin is a clean detach; the session keeps running.
#[cfg(target_os = "linux")]
fn host_input_loop(fd: libc::c_int) -> u8 {
    use willie_proto::hostterm::{self, HostFrame};
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8 * 1024];
    loop {
        match read_fd(0, &mut chunk) {
            Ok(0) => {
                LEAVING.store(true, Ordering::SeqCst);
                let _ = write_all_fd(fd, &wire::encode(wire::DETACH, b""));
                return 0;
            }
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                loop {
                    match hostterm::decode_frame(&buf) {
                        Ok(Some((frame, used))) => {
                            buf.drain(..used);
                            match frame {
                                HostFrame::Input(bytes) => {
                                    for part in bytes.chunks(wire::MAX_PAYLOAD)
                                    {
                                        if write_all_fd(
                                            fd,
                                            &wire::encode(wire::INPUT, part),
                                        )
                                        .is_err()
                                        {
                                            return 1;
                                        }
                                    }
                                }
                                HostFrame::Resize { rows, cols } => {
                                    if write_all_fd(
                                        fd,
                                        &wire::encode_resize(rows, cols),
                                    )
                                    .is_err()
                                    {
                                        return 1;
                                    }
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(_) => return 1,
                    }
                }
            }
            Err(e) if e.raw_os_error() == Some(libc::EINTR) => continue,
            Err(_) => return 1,
        }
    }
}

/// Forward keystrokes and window changes until the user detaches or stdin
/// ends. `size` fixed on the command line disables resize tracking, which
/// is what an automated probe wants.
fn input_loop(fd: libc::c_int, fixed_size: Option<(u16, u16)>) -> u8 {
    let mut buf = [0u8; 8 * 1024];
    loop {
        if RESIZED.swap(false, Ordering::SeqCst)
            && fixed_size.is_none()
            && let Some((rows, cols)) = window_size()
            && write_all_fd(fd, &wire::encode_resize(rows, cols)).is_err()
        {
            return 1;
        }
        match read_fd(0, &mut buf) {
            Ok(0) => {
                LEAVING.store(true, Ordering::SeqCst);
                let _ = write_all_fd(fd, &wire::encode(wire::DETACH, b""));
                return 0;
            }
            Ok(n) => {
                let chunk = &buf[..n];
                match chunk.iter().position(|b| *b == DETACH_KEY) {
                    Some(at) => {
                        if at > 0
                            && write_all_fd(
                                fd,
                                &wire::encode(wire::INPUT, &chunk[..at]),
                            )
                            .is_err()
                        {
                            return 1;
                        }
                        LEAVING.store(true, Ordering::SeqCst);
                        let _ =
                            write_all_fd(fd, &wire::encode(wire::DETACH, b""));
                        return 0;
                    }
                    None => {
                        for part in chunk.chunks(wire::MAX_PAYLOAD) {
                            if write_all_fd(
                                fd,
                                &wire::encode(wire::INPUT, part),
                            )
                            .is_err()
                            {
                                return 1;
                            }
                        }
                    }
                }
            }
            Err(e) if e.raw_os_error() == Some(libc::EINTR) => continue,
            Err(_) => return 1,
        }
    }
}
