//! The socket side of a session: several clients, each with its own
//! bounded queue, fed from one PTY pump. Terminals get output (and a
//! replay when they arrive late); the daemon's control client gets the
//! event stream and may ask for status or a stop.
//!
//! Locks: `screen` and `clients` are never held together; `phase` is a
//! flag. A wedged client costs itself its connection, never the PTY pump.

use std::{
    fs::{self, Permissions},
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::AsRawFd,
        unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use willie_core::session::{SessionEvent, SessionEventKind};
use willie_linux::wire;
use willie_proto::supervisor::{CloseReason, Closed, Hello, Role, Status};

use crate::{
    events::EventLog,
    pty,
    screen::{AltScreen, Ring},
};

/// Output kept for a late terminal. Claude Code's screens are large.
pub const RING_BYTES: usize = 256 * 1024;
/// Bytes a client may have queued before it counts as wedged.
pub const QUEUE_BYTES: usize = 1024 * 1024;
/// Kernel send buffer per client. Bounding it keeps a client that stops
/// reading from backing megabytes up in an unbounded socket buffer, so the
/// `QUEUE_BYTES` budget is the real backpressure and the too-slow drop is
/// deterministic rather than a function of the host's socket buffer size.
const SEND_BUFFER_BYTES: libc::c_int = 64 * 1024;
/// How long the supervisor waits for clients to drain after the end.
pub const DRAIN_GRACE: Duration = Duration::from_secs(2);
/// A client that does not say hello in time is a broken client.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Running,
    Stopping,
    Exited,
}

enum Msg {
    Data(Vec<u8>),
    /// Written, then the socket is shut down and the writer ends.
    Close(Vec<u8>),
}

struct ClientHandle {
    id: u64,
    role: Role,
    tx: Sender<Msg>,
    queued: Arc<AtomicUsize>,
    /// For a forced shutdown when the writer is stuck in `write`.
    stream: UnixStream,
}

struct Screen {
    ring: Ring,
    alt: AltScreen,
    rows: u16,
    cols: u16,
}

/// Everything the threads share.
pub struct Shared {
    master: pty::Fd,
    child: libc::pid_t,
    started_at: String,
    socket: PathBuf,
    screen: Mutex<Screen>,
    clients: Mutex<Vec<ClientHandle>>,
    next_id: AtomicU64,
    events: EventLog,
    phase: Mutex<Phase>,
    /// Why the session is ending when a stop was asked for.
    stop_cause: Mutex<Option<CloseReason>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("child", &self.child)
            .finish()
    }
}

impl Shared {
    #[must_use]
    pub fn new(
        master: pty::Fd,
        child: libc::pid_t,
        started_at: String,
        socket: PathBuf,
        events: EventLog,
    ) -> Arc<Self> {
        Arc::new(Self {
            master,
            child,
            started_at,
            socket,
            screen: Mutex::new(Screen {
                ring: Ring::new(RING_BYTES),
                alt: AltScreen::default(),
                rows: pty::DEFAULT_ROWS,
                cols: pty::DEFAULT_COLS,
            }),
            clients: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(0),
            events,
            phase: Mutex::new(Phase::Running),
            stop_cause: Mutex::new(None),
        })
    }

    // The harness pid, exposed for the stop task and daemon adoption.
    #[allow(dead_code)]
    #[must_use]
    pub fn child(&self) -> libc::pid_t {
        self.child
    }

    #[must_use]
    pub fn phase(&self) -> Phase {
        *lock(&self.phase)
    }
}

/// Refuse to steal a live socket, remove a dead one, bind, 0600.
pub fn bind(socket: &Path) -> io::Result<UnixListener> {
    if socket.exists() {
        if UnixStream::connect(socket).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!(
                    "a session is already listening on {}",
                    socket.display()
                ),
            ));
        }
        fs::remove_file(socket)?;
    }
    let listener = UnixListener::bind(socket)?;
    fs::set_permissions(socket, Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Bound this client socket's kernel send buffer (see `SEND_BUFFER_BYTES`).
/// Best effort: a failure only means the fallback is the host default.
fn cap_send_buffer(stream: &UnixStream) {
    let size = SEND_BUFFER_BYTES;
    // SAFETY: `SO_SNDBUF` reads one `c_int` through the pointer for the
    // length we pass; `stream` owns the fd for the duration of the call.
    unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            std::ptr::addr_of!(size).cast(),
            size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

/// Append to the log and fan the line out to control clients.
pub fn log_event(shared: &Shared, kind: SessionEventKind) -> SessionEvent {
    let event = shared.events.append(kind);
    if let Ok(frame) = wire::encode_json(wire::EVENT, &event) {
        let mut clients = lock(&shared.clients);
        clients
            .retain(|c| c.role != Role::Control || enqueue(c, frame.clone()));
    }
    event
}

/// Start accepting clients.
pub fn start(shared: &Arc<Shared>, listener: UnixListener) -> io::Result<()> {
    let owned = Arc::clone(shared);
    thread::Builder::new()
        .name("accept".to_owned())
        .spawn(move || {
            for incoming in listener.incoming() {
                match incoming {
                    Ok(stream) => {
                        let s = Arc::clone(&owned);
                        let spawned = thread::Builder::new()
                            .name("client".to_owned())
                            .spawn(move || client_thread(&s, stream));
                        if let Err(e) = spawned {
                            eprintln!(
                                "willie-sess: cannot start a client: {e}"
                            );
                        }
                    }
                    Err(e) => eprintln!("willie-sess: accept failed: {e}"),
                }
            }
        })?;
    Ok(())
}

/// Pump PTY output to the ring and every terminal until the session's
/// output ends.
pub fn serve(shared: &Shared) {
    let mut buf = [0u8; 16 * 1024];
    loop {
        match pty::read(&shared.master, &mut buf) {
            Ok(0) => break,
            Ok(n) => broadcast(shared, &buf[..n]),
            Err(e) => {
                eprintln!("willie-sess: pty read failed: {e}");
                break;
            }
        }
    }
}

/// Record the end, tell every client, unlink the socket, let them drain.
pub fn finish(shared: &Shared, exit: pty::Exit) {
    *lock(&shared.phase) = Phase::Exited;
    log_event(
        shared,
        SessionEventKind::Exited {
            code: exit.code,
            signal: exit.signal,
        },
    );
    let reason = lock(&shared.stop_cause).unwrap_or(CloseReason::Exited);
    let closed = Closed {
        reason,
        code: exit.code,
        signal: exit.signal,
    };
    let frame = wire::encode_json(wire::CLOSED, &closed).unwrap_or_default();
    let handles: Vec<ClientHandle> =
        std::mem::take(&mut *lock(&shared.clients));
    for c in &handles {
        let _ = c.tx.send(Msg::Close(frame.clone()));
    }
    let _ = fs::remove_file(&shared.socket);
    let until = Instant::now() + DRAIN_GRACE;
    while Instant::now() < until
        && handles.iter().any(|c| c.queued.load(Ordering::SeqCst) > 0)
    {
        thread::sleep(Duration::from_millis(20));
    }
    for c in handles {
        let _ = c.stream.shutdown(Shutdown::Both);
    }
}

/// Ask the harness to stop. Idempotent; extended with the full ladder in
/// the stop task — here it records the request and sends `SIGINT`.
pub fn request_stop(shared: &Arc<Shared>, by: &str, cause: CloseReason) {
    {
        let mut phase = lock(&shared.phase);
        if *phase != Phase::Running {
            return;
        }
        *phase = Phase::Stopping;
    }
    *lock(&shared.stop_cause) = Some(cause);
    log_event(
        shared,
        SessionEventKind::StopRequested { by: by.to_owned() },
    );
    // SAFETY: signalling the harness's own process group (it did setsid).
    unsafe { libc::kill(-shared.child, libc::SIGINT) };
}

fn status_of(shared: &Shared) -> Status {
    let clients = lock(&shared.clients)
        .iter()
        .filter(|c| c.role == Role::Terminal)
        .count();
    Status {
        pid: u32::try_from(shared.child).unwrap_or(0),
        state: match shared.phase() {
            Phase::Stopping => "stopping",
            _ => "running",
        }
        .to_owned(),
        clients: u32::try_from(clients).unwrap_or(u32::MAX),
        started_at: shared.started_at.clone(),
    }
}

/// Queue a frame for one client. `false` when its budget is exhausted:
/// the caller closes it as too slow.
fn enqueue(client: &ClientHandle, frame: Vec<u8>) -> bool {
    let len = frame.len();
    if client.queued.fetch_add(len, Ordering::SeqCst) + len > QUEUE_BYTES {
        client.queued.fetch_sub(len, Ordering::SeqCst);
        return false;
    }
    if client.tx.send(Msg::Data(frame)).is_err() {
        client.queued.fetch_sub(len, Ordering::SeqCst);
        return false;
    }
    true
}

/// Send the last frame and shut the socket down, even if the writer is
/// stuck in a blocked `write` (the shutdown unblocks it).
fn close_client(client: &ClientHandle, reason: CloseReason) {
    let closed = Closed {
        reason,
        code: None,
        signal: None,
    };
    if let Ok(frame) = wire::encode_json(wire::CLOSED, &closed) {
        let _ = client.tx.send(Msg::Close(frame));
    }
    if reason == CloseReason::TooSlow {
        let _ = client.stream.shutdown(Shutdown::Both);
    }
}

fn broadcast(shared: &Shared, bytes: &[u8]) {
    {
        let mut screen = lock(&shared.screen);
        screen.ring.push(bytes);
        screen.alt.feed(bytes);
    }
    let frames = wire::chunks(wire::OUTPUT, bytes);
    let mut clients = lock(&shared.clients);
    let mut dropped = Vec::new();
    clients.retain(|c| {
        if c.role != Role::Terminal {
            return true;
        }
        let ok = frames.iter().all(|f| enqueue(c, f.clone()));
        if !ok {
            close_client(c, CloseReason::TooSlow);
            dropped.push(c.id);
        }
        ok
    });
    drop(clients);
    for id in dropped {
        eprintln!("willie-sess: client {id} dropped as too slow");
        log_event(shared, SessionEventKind::Detached { client: id });
    }
}

fn writer_loop(
    mut stream: UnixStream,
    rx: Receiver<Msg>,
    queued: Arc<AtomicUsize>,
) {
    for msg in rx {
        match msg {
            Msg::Data(bytes) => {
                let n = bytes.len();
                let ok = stream.write_all(&bytes).is_ok();
                queued.fetch_sub(n, Ordering::SeqCst);
                if !ok {
                    let _ = stream.shutdown(Shutdown::Both);
                    break;
                }
            }
            Msg::Close(bytes) => {
                let _ = stream.write_all(&bytes);
                let _ = stream.shutdown(Shutdown::Both);
                break;
            }
        }
    }
}

/// Read one frame with the hello deadline applied.
fn first_frame(
    stream: &mut UnixStream,
    decoder: &mut wire::Decoder,
) -> Option<wire::Frame> {
    let _ = stream.set_read_timeout(Some(HELLO_TIMEOUT));
    let mut buf = [0u8; 4096];
    loop {
        if let Some(f) = decoder.pop() {
            let _ = stream.set_read_timeout(None);
            return Some(f);
        }
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return None,
            Ok(n) => decoder.push(&buf[..n]),
        }
    }
}

/// Reject a client that did not start with a well-formed hello.
fn reject(mut stream: UnixStream) {
    let closed = Closed {
        reason: CloseReason::Protocol,
        code: None,
        signal: None,
    };
    if let Ok(frame) = wire::encode_json(wire::CLOSED, &closed) {
        let _ = stream.write_all(&frame);
    }
    let _ = stream.shutdown(Shutdown::Both);
}

/// Apply a terminal size. On the alternate screen with no change, nudge
/// the width so the program receives `SIGWINCH` and redraws.
fn apply_size(shared: &Shared, rows: u16, cols: u16, force_redraw: bool) {
    let nudge = {
        let mut screen = lock(&shared.screen);
        let same = screen.rows == rows && screen.cols == cols;
        screen.rows = rows;
        screen.cols = cols;
        force_redraw && same && screen.alt.active()
    };
    if nudge {
        let _ =
            pty::set_window_size(&shared.master, rows, cols.saturating_sub(1));
        log_event(
            shared,
            SessionEventKind::Resized {
                rows,
                cols: cols.saturating_sub(1),
            },
        );
    }
    if let Err(e) = pty::set_window_size(&shared.master, rows, cols) {
        eprintln!("willie-sess: resize failed: {e}");
        return;
    }
    log_event(shared, SessionEventKind::Resized { rows, cols });
}

fn client_thread(shared: &Arc<Shared>, mut stream: UnixStream) {
    let mut decoder = wire::Decoder::new();
    let hello: Hello = match first_frame(&mut stream, &mut decoder) {
        Some(f) if f.kind == wire::HELLO => match wire::decode_json(&f.payload)
        {
            Ok(h) => h,
            Err(_) => return reject(stream),
        },
        _ => return reject(stream),
    };
    if shared.phase() == Phase::Exited {
        return reject(stream);
    }
    cap_send_buffer(&stream);
    let Ok(writer) = stream.try_clone() else {
        return reject(stream);
    };
    let Ok(for_shutdown) = stream.try_clone() else {
        return reject(stream);
    };
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    let (tx, rx) = mpsc::channel::<Msg>();
    let queued = Arc::new(AtomicUsize::new(0));
    let handle = ClientHandle {
        id,
        role: hello.role,
        tx: tx.clone(),
        queued: Arc::clone(&queued),
        stream: for_shutdown,
    };
    let writer_queued = Arc::clone(&queued);
    let writer_thread = thread::Builder::new()
        .name(format!("writer-{id}"))
        .spawn(move || writer_loop(writer, rx, writer_queued));
    if writer_thread.is_err() {
        return reject(stream);
    }
    if hello.role == Role::Terminal {
        let replay = {
            let screen = lock(&shared.screen);
            if screen.alt.active() {
                Vec::new()
            } else {
                screen.ring.snapshot()
            }
        };
        for frame in wire::chunks(wire::OUTPUT, &replay) {
            if !replay.is_empty() && !enqueue(&handle, frame) {
                close_client(&handle, CloseReason::TooSlow);
                return;
            }
        }
        apply_size(shared, hello.rows, hello.cols, true);
    }
    lock(&shared.clients).push(handle);
    if hello.role == Role::Terminal {
        log_event(shared, SessionEventKind::Attached { client: id });
    }

    let mut buf = [0u8; 8192];
    'outer: loop {
        while let Some(frame) = decoder.pop() {
            match frame.kind {
                wire::INPUT => {
                    if pty::write_all(&shared.master, &frame.payload).is_err() {
                        break 'outer;
                    }
                }
                wire::RESIZE => {
                    if let Some((rows, cols)) =
                        wire::decode_resize(&frame.payload)
                    {
                        apply_size(shared, rows, cols, false);
                    }
                }
                wire::DETACH => break 'outer,
                wire::STOP if hello.role == Role::Control => {
                    request_stop(shared, "daemon", CloseReason::Stopped);
                }
                wire::STATUS_REQ if hello.role == Role::Control => {
                    if let Ok(f) =
                        wire::encode_json(wire::STATUS, &status_of(shared))
                    {
                        // The status reply skips the budget check, but its
                        // bytes must still be counted: the writer decrements
                        // `queued` after every `Msg::Data`, so an unaccounted
                        // send underflows the counter and every later event
                        // is wrongly rejected as over budget.
                        queued.fetch_add(f.len(), Ordering::SeqCst);
                        let _ = tx.send(Msg::Data(f));
                    }
                }
                _ => {}
            }
        }
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => decoder.push(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let still_registered = {
        let mut clients = lock(&shared.clients);
        let before = clients.len();
        clients.retain(|c| c.id != id);
        clients.len() != before
    };
    let _ = tx.send(Msg::Close(Vec::new()));
    let _ = stream.shutdown(Shutdown::Both);
    if still_registered && hello.role == Role::Terminal {
        log_event(shared, SessionEventKind::Detached { client: id });
    }
}
