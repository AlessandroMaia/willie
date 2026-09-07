//! The syscall filter, as data.
//!
//! A classic-BPF program the in-namespace stage installs before it execs
//! the harness. The wrong architecture and the x32 ABI are killed; the
//! calls a session has no business making are marked for user
//! notification, so the supervisor can answer them with `EPERM` and
//! record what was asked; everything else passes. It is built here as
//! portable instructions with no `libc`, so a small interpreter in the
//! tests runs it on any host before the kernel ever sees it, and the
//! supervisor copies each [`Insn`] onto the kernel's `sock_filter` field
//! by field.
//!
//! The syscall constants keep the kernel's own names so the Linux test
//! that compares each against `libc` reads without translation.
#![allow(non_upper_case_globals)]

/// One classic-BPF instruction, laid out as the kernel's `sock_filter`:
/// the code, the two jump offsets, the constant. The supervisor copies
/// each field into the kernel's own type at install time. An offset
/// counts from the instruction after the jump, and only forward.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insn {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

/// The audit architecture of the native 64-bit ABI: the machine number
/// with the 64-bit and little-endian bits set. The x32 ABI reports the
/// same value, which is why `nr` is also checked against its bit.
pub const AUDIT_ARCH_X86_64: u32 = 0xC000_003E;

/// Set in `nr` by every x32 syscall.
const X32_SYSCALL_BIT: u32 = 0x4000_0000;

// Byte offsets into `seccomp_data`: `nr` and `arch` are 32-bit words,
// the instruction pointer follows, and each argument is a little-endian
// 64-bit value, so a word load at its base reads its low half.
const DATA_NR: u32 = 0;
const DATA_ARCH: u32 = 4;
const DATA_ARG0: u32 = 16;
const DATA_ARG1: u32 = 24;
const DATA_ARG2: u32 = 32;

// The five instructions the program is made of: a class combined with
// its size and mode, or with its operation and operand source.
/// `BPF_LD | BPF_W | BPF_ABS`: the accumulator takes the word at `k`.
const LD_W_ABS: u16 = 0x20;
/// `BPF_JMP | BPF_JEQ | BPF_K`: skip `jt` instructions when the
/// accumulator equals `k`, `jf` otherwise.
const JEQ_K: u16 = 0x15;
/// `BPF_JMP | BPF_JGE | BPF_K`: the same for accumulator `>= k`, unsigned.
const JGE_K: u16 = 0x35;
/// `BPF_ALU | BPF_AND | BPF_K`: the accumulator is masked with `k`.
const AND_K: u16 = 0x54;
/// `BPF_RET | BPF_K`: the verdict is `k`.
const RET_K: u16 = 0x06;

/// Kill the whole process: for a call no program built for this session
/// makes, where refusing would only hide a broken or hostile binary.
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
/// Hand the call to the supervisor, which answers `EPERM` and records it.
const SECCOMP_RET_USER_NOTIF: u32 = 0x7FC0_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7FFF_0000;

/// Terminal input injection: the one `ioctl` request refused.
const TIOCSTI: u32 = 0x5412;
/// prctl's PR_SET_SECCOMP: the one prctl option refused, so a session
/// cannot install a filter that could shadow this one.
const PR_SET_SECCOMP: u32 = 22;
const AF_NETLINK: u32 = 16;
const AF_PACKET: u32 = 17;
const SOCK_RAW: u32 = 3;
/// The obsolete packet socket type: the kernel turns an `AF_INET` socket
/// of this type into an `AF_PACKET` one, so it is judged by type too.
const SOCK_PACKET: u32 = 10;
/// The socket type proper is the low byte; the non-blocking and
/// close-on-exec flags sit above it.
const SOCK_TYPE_MASK: u32 = 0xFF;
/// The netlink protocol that lists interfaces and addresses.
const NETLINK_ROUTE: u32 = 0;

// The syscall numbers of the native 64-bit ABI. These two are judged by
// their arguments:
pub const SYS_ioctl: u32 = 16;
pub const SYS_socket: u32 = 41;
pub const SYS_prctl: u32 = 157;
// These are refused outright; `DENIED` groups and names them.
pub const SYS_ptrace: u32 = 101;
pub const SYS_process_vm_readv: u32 = 310;
pub const SYS_process_vm_writev: u32 = 311;
pub const SYS_pidfd_getfd: u32 = 438;
pub const SYS_bpf: u32 = 321;
pub const SYS_io_uring_setup: u32 = 425;
pub const SYS_io_uring_enter: u32 = 426;
pub const SYS_io_uring_register: u32 = 427;
pub const SYS_perf_event_open: u32 = 298;
pub const SYS_userfaultfd: u32 = 323;
pub const SYS_seccomp: u32 = 317;
pub const SYS_mount: u32 = 165;
pub const SYS_umount2: u32 = 166;
pub const SYS_pivot_root: u32 = 155;
pub const SYS_mount_setattr: u32 = 442;
pub const SYS_open_tree: u32 = 428;
pub const SYS_move_mount: u32 = 429;
pub const SYS_fsopen: u32 = 430;
pub const SYS_fsconfig: u32 = 431;
pub const SYS_fsmount: u32 = 432;
pub const SYS_fspick: u32 = 433;
pub const SYS_unshare: u32 = 272;
pub const SYS_setns: u32 = 308;
pub const SYS_init_module: u32 = 175;
pub const SYS_finit_module: u32 = 313;
pub const SYS_delete_module: u32 = 176;
pub const SYS_kexec_load: u32 = 246;
pub const SYS_kexec_file_load: u32 = 320;
pub const SYS_add_key: u32 = 248;
pub const SYS_request_key: u32 = 249;
pub const SYS_keyctl: u32 = 250;

/// The calls refused whatever their arguments, each with the name the
/// log gives it, grouped by what a session would reach through them.
const DENIED: &[(u32, &str)] = &[
    // Another process's execution and memory.
    (SYS_ptrace, "ptrace"),
    (SYS_process_vm_readv, "process_vm_readv"),
    (SYS_process_vm_writev, "process_vm_writev"),
    (SYS_pidfd_getfd, "pidfd_getfd"),
    // Programs and rings inside the kernel, and its instrumentation.
    (SYS_bpf, "bpf"),
    (SYS_io_uring_setup, "io_uring_setup"),
    (SYS_io_uring_enter, "io_uring_enter"),
    (SYS_io_uring_register, "io_uring_register"),
    (SYS_perf_event_open, "perf_event_open"),
    (SYS_userfaultfd, "userfaultfd"),
    // A filter of its own. The kernel runs filters newest first and, when
    // two answer user notification, the newest one's listener gets the
    // call: a nested listener could continue what this filter refuses.
    // The stage's own install is the first filter in the process, so it
    // is not in force yet when that call is made.
    (SYS_seccomp, "seccomp"),
    // The mount table, through the old interface and the new one.
    (SYS_mount, "mount"),
    (SYS_umount2, "umount2"),
    (SYS_pivot_root, "pivot_root"),
    (SYS_mount_setattr, "mount_setattr"),
    (SYS_open_tree, "open_tree"),
    (SYS_move_mount, "move_mount"),
    (SYS_fsopen, "fsopen"),
    (SYS_fsconfig, "fsconfig"),
    (SYS_fsmount, "fsmount"),
    (SYS_fspick, "fspick"),
    // Namespaces: leaving this one, or making another.
    (SYS_unshare, "unshare"),
    (SYS_setns, "setns"),
    // Kernel modules, and a replacement kernel.
    (SYS_init_module, "init_module"),
    (SYS_finit_module, "finit_module"),
    (SYS_delete_module, "delete_module"),
    (SYS_kexec_load, "kexec_load"),
    (SYS_kexec_file_load, "kexec_file_load"),
    // The kernel keyring.
    (SYS_add_key, "add_key"),
    (SYS_request_key, "request_key"),
    (SYS_keyctl, "keyctl"),
];

/// The name of a call the filter singles out, for the log; `None` for
/// any other number, which the log then writes as `syscall_<nr>`.
#[must_use]
pub fn name(nr: u32) -> Option<&'static str> {
    match nr {
        SYS_ioctl => Some("ioctl"),
        SYS_socket => Some("socket"),
        SYS_prctl => Some("prctl"),
        _ => DENIED.iter().find(|(n, _)| *n == nr).map(|(_, name)| *name),
    }
}

const fn stmt(code: u16, k: u32) -> Insn {
    Insn {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

const fn jump(code: u16, jt: u8, jf: u8, k: u32) -> Insn {
    Insn { code, jt, jf, k }
}

/// Where a jump lands, by name. Resolved to an offset once the whole
/// program is laid out, so no offset is ever counted by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum To {
    /// The instruction after the jump.
    Next,
    Allow,
    Notify,
    Kill,
    Ioctl,
    Socket,
    /// The socket block's type check, past the netlink protocol check.
    SocketType,
    Prctl,
}

#[derive(Debug, Clone, Copy)]
enum Item {
    Stmt(Insn),
    Jump { code: u16, k: u32, jt: To, jf: To },
}

/// The program under construction: its instructions, and where each
/// label was placed among them.
#[derive(Debug, Default)]
struct Asm {
    items: Vec<Item>,
    labels: Vec<(To, usize)>,
}

impl Asm {
    fn stmt(&mut self, code: u16, k: u32) {
        self.items.push(Item::Stmt(stmt(code, k)));
    }

    fn jump(&mut self, code: u16, k: u32, jt: To, jf: To) {
        self.items.push(Item::Jump { code, k, jt, jf });
    }

    /// The next instruction pushed is where `label` lands.
    fn mark(&mut self, label: To) {
        self.labels.push((label, self.items.len()));
    }

    /// The offset from the instruction after `at` to `label`. A label
    /// that was never placed, lies behind the jump or is too far has no
    /// encoding; the value then points past the end, which the tests
    /// refuse and, failing them, the kernel refuses at install, rather
    /// than run a program that means something else.
    fn offset(&self, at: usize, label: To) -> u8 {
        if label == To::Next {
            return 0;
        }
        self.labels
            .iter()
            .find(|(placed, _)| *placed == label)
            .and_then(|(_, target)| target.checked_sub(at + 1))
            .and_then(|distance| u8::try_from(distance).ok())
            .unwrap_or(u8::MAX)
    }

    fn finish(self) -> Vec<Insn> {
        self.items
            .iter()
            .enumerate()
            .map(|(at, item)| match *item {
                Item::Stmt(insn) => insn,
                Item::Jump { code, k, jt, jf } => {
                    jump(code, self.offset(at, jt), self.offset(at, jf), k)
                }
            })
            .collect()
    }
}

/// The whole filter, ready to install: the architecture and x32 checks,
/// one comparison per denied call, the two calls judged by their
/// arguments, and the default. Every jump goes forward, to a block or to
/// one of the three shared verdicts at the end, so the program is a
/// straight line the tests walk in full.
#[must_use]
pub fn program() -> Vec<Insn> {
    let mut asm = Asm::default();

    // Only the native ABI. Any other architecture's numbering dies, and
    // so does the x32 ABI, which shares this audit value but sets a bit
    // in `nr` — the architecture check alone would let it through.
    asm.stmt(LD_W_ABS, DATA_ARCH);
    asm.jump(JEQ_K, AUDIT_ARCH_X86_64, To::Next, To::Kill);
    asm.stmt(LD_W_ABS, DATA_NR);
    asm.jump(JGE_K, X32_SYSCALL_BIT, To::Kill, To::Next);

    // One comparison per denied call; a miss falls through to the next.
    for (nr, _) in DENIED {
        asm.jump(JEQ_K, *nr, To::Notify, To::Next);
    }

    // The calls judged by their arguments, then the default.
    asm.jump(JEQ_K, SYS_ioctl, To::Ioctl, To::Next);
    asm.jump(JEQ_K, SYS_socket, To::Socket, To::Next);
    asm.jump(JEQ_K, SYS_prctl, To::Prctl, To::Next);
    asm.stmt(RET_K, SECCOMP_RET_ALLOW);

    // ioctl: the request is the second argument; only terminal input
    // injection is refused.
    asm.mark(To::Ioctl);
    asm.stmt(LD_W_ABS, DATA_ARG1);
    asm.jump(JEQ_K, TIOCSTI, To::Notify, To::Allow);

    // socket: domain, type, protocol. Packet sockets are refused. A
    // netlink socket is decided by its protocol alone, before the type
    // is looked at, because listing interfaces opens a raw netlink
    // socket for the route protocol and that must pass; every other
    // netlink protocol is refused. Any other family is refused when
    // raw, once the flags above the type's low byte are masked off, and
    // when its type is the obsolete packet one, which the kernel turns
    // into a packet socket whatever the family asked for.
    asm.mark(To::Socket);
    asm.stmt(LD_W_ABS, DATA_ARG0);
    asm.jump(JEQ_K, AF_PACKET, To::Notify, To::Next);
    asm.jump(JEQ_K, AF_NETLINK, To::Next, To::SocketType);
    asm.stmt(LD_W_ABS, DATA_ARG2);
    asm.jump(JEQ_K, NETLINK_ROUTE, To::Allow, To::Notify);
    asm.mark(To::SocketType);
    asm.stmt(LD_W_ABS, DATA_ARG1);
    asm.stmt(AND_K, SOCK_TYPE_MASK);
    asm.jump(JEQ_K, SOCK_RAW, To::Notify, To::Next);
    asm.jump(JEQ_K, SOCK_PACKET, To::Notify, To::Allow);

    // prctl: the option is the first argument; only installing a seccomp
    // filter is refused, so a nested filter cannot shadow this one.
    asm.mark(To::Prctl);
    asm.stmt(LD_W_ABS, DATA_ARG0);
    asm.jump(JEQ_K, PR_SET_SECCOMP, To::Notify, To::Allow);

    // The three verdicts every jump above lands on.
    asm.mark(To::Allow);
    asm.stmt(RET_K, SECCOMP_RET_ALLOW);
    asm.mark(To::Notify);
    asm.stmt(RET_K, SECCOMP_RET_USER_NOTIF);
    asm.mark(To::Kill);
    asm.stmt(RET_K, SECCOMP_RET_KILL_PROCESS);

    asm.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The action bits of a verdict; the low sixteen carry data this
    /// filter never sets.
    const SECCOMP_RET_ACTION_FULL: u32 = 0xFFFF_0000;

    // Calls and values the filter must leave alone, so the module itself
    // has no reason to name them.
    const SYS_read: u32 = 0;
    const SYS_write: u32 = 1;
    const SYS_execve: u32 = 59;
    const SYS_clone3: u32 = 435;
    const AUDIT_ARCH_I386: u32 = 0x4000_0003;
    const TIOCGWINSZ: u32 = 0x5413;
    const PR_SET_SECCOMP: u32 = 22;
    const PR_SET_NAME: u32 = 15;
    const AF_INET: u32 = 2;
    const SOCK_STREAM: u32 = 1;
    const SOCK_DGRAM: u32 = 2;
    const SOCK_NONBLOCK: u32 = 0o4000;
    const SOCK_CLOEXEC: u32 = 0o2_000_000;
    const NETLINK_KOBJECT_UEVENT: u32 = 15;

    const NO_ARGS: [u64; 6] = [0; 6];

    /// `seccomp_data` as the kernel hands it to the filter: `nr` and
    /// `arch` as 32-bit words, the instruction pointer, then the six
    /// arguments as little-endian 64-bit values.
    fn seccomp_data(arch: u32, nr: u32, args: [u64; 6]) -> [u8; 64] {
        let mut bytes = [0u8; 64];
        bytes[0..4].copy_from_slice(&nr.to_le_bytes());
        bytes[4..8].copy_from_slice(&arch.to_le_bytes());
        for (i, arg) in args.iter().enumerate() {
            let at = 16 + 8 * i;
            bytes[at..at + 8].copy_from_slice(&arg.to_le_bytes());
        }
        bytes
    }

    /// A classic-BPF interpreter for exactly the instructions the filter
    /// uses, stepping the way the kernel does: a jump skips `jt` or `jf`
    /// instructions past the next one. A load outside the data, a path
    /// off the end or any other instruction is a construction bug, so it
    /// panics the test.
    fn run(prog: &[Insn], data: &[u8; 64]) -> u32 {
        let mut acc = 0u32;
        let mut pc = 0usize;
        loop {
            let Some(insn) = prog.get(pc) else {
                panic!(
                    "pc {pc} ran off the end of {} instructions",
                    prog.len()
                );
            };
            let skip = |taken: bool| {
                usize::from(if taken { insn.jt } else { insn.jf })
            };
            match insn.code {
                LD_W_ABS => {
                    let at = usize::try_from(insn.k).unwrap();
                    acc = u32::from_le_bytes(
                        data[at..at + 4].try_into().unwrap(),
                    );
                    pc += 1;
                }
                JEQ_K => pc += 1 + skip(acc == insn.k),
                JGE_K => pc += 1 + skip(acc >= insn.k),
                AND_K => {
                    acc &= insn.k;
                    pc += 1;
                }
                RET_K => return insn.k,
                other => panic!(
                    "instruction {other:#06x} at {pc} is not one the filter uses"
                ),
            }
        }
    }

    /// The verdict's action word for one call under the real program.
    fn action(arch: u32, nr: u32, args: [u64; 6]) -> u32 {
        run(&program(), &seccomp_data(arch, nr, args)) & SECCOMP_RET_ACTION_FULL
    }

    fn socket_args(domain: u32, ty: u32, protocol: u32) -> [u64; 6] {
        [
            u64::from(domain),
            u64::from(ty),
            u64::from(protocol),
            0,
            0,
            0,
        ]
    }

    fn ioctl_args(request: u32) -> [u64; 6] {
        [0, u64::from(request), 0, 0, 0, 0]
    }

    fn prctl_args(option: u32) -> [u64; 6] {
        [u64::from(option), 0, 0, 0, 0, 0]
    }

    /// The interpreter reads the data the way the kernel lays it out, or
    /// every other test here proves nothing: `nr` first, `arch` second,
    /// and each argument's low word at the argument's own offset.
    #[test]
    fn the_interpreter_reads_the_fields_where_the_kernel_puts_them() {
        let data = seccomp_data(
            0xAAAA_0001,
            0xBBBB_0002,
            [0x1111_2222_3333_4444, 0x5555_6666_7777_8888, 9, 0, 0, 0],
        );
        let loads = |k: u32, expected: u32| {
            let probe = [
                stmt(LD_W_ABS, k),
                jump(JEQ_K, 0, 1, expected),
                stmt(RET_K, 1),
                stmt(RET_K, 0),
            ];
            run(&probe, &data) == 1
        };

        assert!(loads(0, 0xBBBB_0002), "nr");
        assert!(loads(4, 0xAAAA_0001), "arch");
        assert!(loads(16, 0x3333_4444), "args[0], low word");
        assert!(loads(20, 0x1111_2222), "args[0], high word");
        assert!(loads(24, 0x7777_8888), "args[1], low word");
        assert!(loads(32, 9), "args[2]");
        assert!(!loads(24, 0x5555_6666), "the high word is not the low one");
    }

    /// The compare is unsigned, as the kernel's is: a value with the top
    /// bit set is large, not negative.
    #[test]
    fn the_interpreter_masks_and_compares_unsigned() {
        let probe = [
            stmt(LD_W_ABS, 0),
            stmt(AND_K, 0xFFFF_FF00),
            jump(JGE_K, 0, 1, 0x8000_0000),
            stmt(RET_K, 1),
            stmt(RET_K, 0),
        ];

        assert_eq!(run(&probe, &seccomp_data(0, 0xFFFF_FFFF, NO_ARGS)), 1);
        assert_eq!(run(&probe, &seccomp_data(0, 0x7FFF_FFFF, NO_ARGS)), 0);
        assert_eq!(run(&probe, &seccomp_data(0, 0x8000_00FF, NO_ARGS)), 1);
        assert_eq!(run(&probe, &seccomp_data(0, 0x0000_00FF, NO_ARGS)), 0);
    }

    #[test]
    fn a_denied_syscall_is_notified() {
        for (nr, name) in DENIED {
            assert_eq!(
                action(AUDIT_ARCH_X86_64, *nr, NO_ARGS),
                SECCOMP_RET_USER_NOTIF,
                "{name} ({nr})"
            );
        }
    }

    /// The install the stage makes is the first filter in the process and
    /// runs before this program is in force; once it is, the same call
    /// from the harness is refused, so no nested filter can hold a
    /// listener that pre-empts the supervisor's.
    #[test]
    fn installing_a_filter_of_its_own_is_notified() {
        assert_eq!(
            action(AUDIT_ARCH_X86_64, SYS_seccomp, [1, 8, 0, 0, 0, 0]),
            SECCOMP_RET_USER_NOTIF
        );
        assert_eq!(
            action(AUDIT_ARCH_X86_64, SYS_seccomp, NO_ARGS),
            SECCOMP_RET_USER_NOTIF
        );
    }

    #[test]
    fn an_ordinary_syscall_is_allowed() {
        for nr in [SYS_read, SYS_write, SYS_execve, SYS_clone3] {
            assert_eq!(
                action(AUDIT_ARCH_X86_64, nr, NO_ARGS),
                SECCOMP_RET_ALLOW,
                "{nr}"
            );
        }
    }

    #[test]
    fn tiocsti_is_notified_other_ioctls_pass() {
        assert_eq!(
            action(AUDIT_ARCH_X86_64, SYS_ioctl, ioctl_args(TIOCSTI)),
            SECCOMP_RET_USER_NOTIF
        );
        assert_eq!(
            action(AUDIT_ARCH_X86_64, SYS_ioctl, ioctl_args(TIOCGWINSZ)),
            SECCOMP_RET_ALLOW
        );
        assert_eq!(
            action(AUDIT_ARCH_X86_64, SYS_ioctl, NO_ARGS),
            SECCOMP_RET_ALLOW
        );
    }

    /// The harness sets its process name, death signal and no-new-privs
    /// through prctl; only PR_SET_SECCOMP is refused, because a filter
    /// installed that way could shadow this one and mute a denial line.
    #[test]
    fn prctl_set_seccomp_is_notified_other_prctls_pass() {
        assert_eq!(
            action(AUDIT_ARCH_X86_64, SYS_prctl, prctl_args(PR_SET_SECCOMP)),
            SECCOMP_RET_USER_NOTIF
        );
        assert_eq!(
            action(AUDIT_ARCH_X86_64, SYS_prctl, prctl_args(PR_SET_NAME)),
            SECCOMP_RET_ALLOW
        );
        assert_eq!(
            action(AUDIT_ARCH_X86_64, SYS_prctl, NO_ARGS),
            SECCOMP_RET_ALLOW
        );
    }

    /// Listing interfaces opens a raw netlink socket for the route
    /// protocol, so that one decision has to come before the raw-socket
    /// refusal; every other netlink protocol, packet sockets by family or
    /// by the obsolete type the kernel rewrites into that family, and raw
    /// sockets of any other family are refused.
    #[test]
    fn netlink_route_passes_other_netlink_and_packet_and_raw_do_not() {
        let socket = |domain, ty, protocol| {
            action(
                AUDIT_ARCH_X86_64,
                SYS_socket,
                socket_args(domain, ty, protocol),
            )
        };

        assert_eq!(
            socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE),
            SECCOMP_RET_ALLOW
        );
        assert_eq!(
            socket(AF_NETLINK, SOCK_DGRAM, NETLINK_KOBJECT_UEVENT),
            SECCOMP_RET_USER_NOTIF
        );
        assert_eq!(socket(AF_PACKET, SOCK_RAW, 0), SECCOMP_RET_USER_NOTIF);
        assert_eq!(socket(AF_PACKET, SOCK_DGRAM, 0), SECCOMP_RET_USER_NOTIF);
        assert_eq!(socket(AF_INET, SOCK_RAW, 0), SECCOMP_RET_USER_NOTIF);
        assert_eq!(socket(AF_INET, SOCK_PACKET, 0), SECCOMP_RET_USER_NOTIF);
        assert_eq!(socket(AF_INET, SOCK_STREAM, 0), SECCOMP_RET_ALLOW);
        assert_eq!(socket(AF_INET, SOCK_DGRAM, 0), SECCOMP_RET_ALLOW);
    }

    /// The type argument carries the non-blocking and close-on-exec
    /// flags above the type itself; the filter judges the type alone.
    #[test]
    fn a_raw_socket_is_refused_whatever_flags_its_type_carries() {
        let flags = SOCK_NONBLOCK | SOCK_CLOEXEC;
        let socket = |ty| {
            action(AUDIT_ARCH_X86_64, SYS_socket, socket_args(AF_INET, ty, 0))
        };

        assert_eq!(socket(SOCK_RAW | flags), SECCOMP_RET_USER_NOTIF);
        assert_eq!(socket(SOCK_PACKET | flags), SECCOMP_RET_USER_NOTIF);
        assert_eq!(socket(SOCK_STREAM | flags), SECCOMP_RET_ALLOW);
        assert_eq!(socket(SOCK_DGRAM | SOCK_CLOEXEC), SECCOMP_RET_ALLOW);
    }

    /// The x32 ABI reports the x86_64 audit architecture, so the
    /// architecture check alone would let it through: its bit in `nr` is
    /// what kills it.
    #[test]
    fn the_wrong_arch_and_x32_are_killed() {
        assert_eq!(action(0, SYS_read, NO_ARGS), SECCOMP_RET_KILL_PROCESS);
        assert_eq!(
            action(AUDIT_ARCH_I386, SYS_read, NO_ARGS),
            SECCOMP_RET_KILL_PROCESS
        );
        assert_eq!(
            action(AUDIT_ARCH_X86_64, X32_SYSCALL_BIT | SYS_read, NO_ARGS),
            SECCOMP_RET_KILL_PROCESS
        );
        assert_eq!(
            action(AUDIT_ARCH_X86_64, X32_SYSCALL_BIT | SYS_ptrace, NO_ARGS),
            SECCOMP_RET_KILL_PROCESS
        );
    }

    /// Offsets are unsigned and count from the next instruction, so a
    /// jump can only go forward; this pins that none goes past the end.
    #[test]
    fn every_jump_lands_inside_the_program() {
        let prog = program();
        let mut jumps = 0;

        for (at, insn) in prog.iter().enumerate() {
            if !matches!(insn.code, JEQ_K | JGE_K) {
                continue;
            }
            jumps += 1;
            for (leg, off) in [("jt", insn.jt), ("jf", insn.jf)] {
                let target = at + 1 + usize::from(off);
                assert!(
                    target < prog.len(),
                    "{leg} of instruction {at} lands at {target}, past {}",
                    prog.len()
                );
            }
        }
        // The arch and x32 checks, one per denied call, the two
        // dispatches, and the ioctl and socket blocks' own.
        assert!(jumps > DENIED.len() + 4, "{jumps} jumps");
    }

    /// The kernel refuses a filter that loads outside `seccomp_data` or
    /// off a word boundary; this refuses it first.
    #[test]
    fn every_load_is_an_aligned_word_inside_seccomp_data() {
        let prog = program();
        let loads: Vec<u32> = prog
            .iter()
            .filter(|insn| insn.code == LD_W_ABS)
            .map(|insn| insn.k)
            .collect();

        assert!(!loads.is_empty());
        for k in loads {
            assert_eq!(k % 4, 0, "offset {k}");
            assert!(k + 4 <= 64, "offset {k}");
        }
    }

    /// Only the five instructions the interpreter knows appear, so what
    /// the tests ran is what the kernel will run; and the last one
    /// returns, so no path can fall off the end.
    #[test]
    fn the_program_is_made_only_of_the_five_instructions_and_ends_in_a_return()
    {
        let prog = program();

        for (at, insn) in prog.iter().enumerate() {
            assert!(
                matches!(insn.code, LD_W_ABS | JEQ_K | JGE_K | AND_K | RET_K),
                "instruction {at} has code {:#06x}",
                insn.code
            );
            if insn.code != JEQ_K && insn.code != JGE_K {
                assert_eq!((insn.jt, insn.jf), (0, 0), "instruction {at}");
            }
        }
        assert_eq!(prog.last().map(|insn| insn.code), Some(RET_K));
    }

    #[test]
    fn name_maps_the_denied_calls() {
        for (nr, expected) in DENIED {
            assert_eq!(name(*nr), Some(*expected));
        }
        assert_eq!(name(SYS_unshare), Some("unshare"));
        assert_eq!(name(SYS_ioctl), Some("ioctl"));
        assert_eq!(name(SYS_socket), Some("socket"));
        assert_eq!(name(SYS_prctl), Some("prctl"));
        assert_eq!(name(SYS_read), None);
        assert_eq!(name(999_999), None);
    }

    /// The supervisor copies each instruction into the kernel's
    /// `sock_filter`, whose layout this is; the Linux test compares it
    /// against the real one.
    #[test]
    fn an_instruction_is_eight_bytes_laid_out_as_sock_filter() {
        assert_eq!(size_of::<Insn>(), 8);
        assert_eq!(align_of::<Insn>(), 4);
    }

    #[cfg(target_os = "linux")]
    mod on_linux {
        use super::*;

        /// This libc's number for a name in the deny list, so the test
        /// ties number, name and libc together in one pass.
        fn libc_number(name: &str) -> i64 {
            match name {
                "ptrace" => libc::SYS_ptrace,
                "process_vm_readv" => libc::SYS_process_vm_readv,
                "process_vm_writev" => libc::SYS_process_vm_writev,
                "pidfd_getfd" => libc::SYS_pidfd_getfd,
                "bpf" => libc::SYS_bpf,
                "io_uring_setup" => libc::SYS_io_uring_setup,
                "io_uring_enter" => libc::SYS_io_uring_enter,
                "io_uring_register" => libc::SYS_io_uring_register,
                "perf_event_open" => libc::SYS_perf_event_open,
                "userfaultfd" => libc::SYS_userfaultfd,
                "seccomp" => libc::SYS_seccomp,
                "mount" => libc::SYS_mount,
                "umount2" => libc::SYS_umount2,
                "pivot_root" => libc::SYS_pivot_root,
                "mount_setattr" => libc::SYS_mount_setattr,
                "open_tree" => libc::SYS_open_tree,
                "move_mount" => libc::SYS_move_mount,
                "fsopen" => libc::SYS_fsopen,
                "fsconfig" => libc::SYS_fsconfig,
                "fsmount" => libc::SYS_fsmount,
                "fspick" => libc::SYS_fspick,
                "unshare" => libc::SYS_unshare,
                "setns" => libc::SYS_setns,
                "init_module" => libc::SYS_init_module,
                "finit_module" => libc::SYS_finit_module,
                "delete_module" => libc::SYS_delete_module,
                "kexec_load" => libc::SYS_kexec_load,
                "kexec_file_load" => libc::SYS_kexec_file_load,
                "add_key" => libc::SYS_add_key,
                "request_key" => libc::SYS_request_key,
                "keyctl" => libc::SYS_keyctl,
                other => panic!("{other} is not a denied call"),
            }
        }

        #[test]
        fn the_numbers_match_this_libc() {
            for (nr, name) in DENIED {
                assert_eq!(i64::from(*nr), libc_number(name), "{name}");
            }
            assert_eq!(i64::from(SYS_ioctl), libc::SYS_ioctl);
            assert_eq!(i64::from(SYS_socket), libc::SYS_socket);
            assert_eq!(i64::from(SYS_prctl), libc::SYS_prctl);
            assert_eq!(i64::from(SYS_read), libc::SYS_read);
            assert_eq!(i64::from(SYS_write), libc::SYS_write);
            assert_eq!(i64::from(SYS_execve), libc::SYS_execve);
            assert_eq!(i64::from(SYS_clone3), libc::SYS_clone3);
        }

        /// An instruction's code is class, then size and mode for a load
        /// or operation and source for the rest, exactly as the kernel
        /// takes it apart.
        #[test]
        fn the_encodings_verdicts_and_arguments_match_this_libc() {
            let class = |c: u16| u32::from(c & 0x07);
            let size = |c: u16| u32::from(c & 0x18);
            let mode = |c: u16| u32::from(c & 0xE0);
            let op = |c: u16| u32::from(c & 0xF0);
            let src = |c: u16| u32::from(c & 0x08);

            assert_eq!(
                (class(LD_W_ABS), size(LD_W_ABS), mode(LD_W_ABS)),
                (libc::BPF_LD, libc::BPF_W, libc::BPF_ABS)
            );
            assert_eq!(
                (class(JEQ_K), op(JEQ_K), src(JEQ_K)),
                (libc::BPF_JMP, libc::BPF_JEQ, libc::BPF_K)
            );
            assert_eq!(
                (class(JGE_K), op(JGE_K), src(JGE_K)),
                (libc::BPF_JMP, libc::BPF_JGE, libc::BPF_K)
            );
            assert_eq!(
                (class(AND_K), op(AND_K), src(AND_K)),
                (libc::BPF_ALU, libc::BPF_AND, libc::BPF_K)
            );
            assert_eq!(
                (class(RET_K), src(RET_K)),
                (libc::BPF_RET, libc::BPF_K)
            );

            assert_eq!(
                SECCOMP_RET_KILL_PROCESS,
                libc::SECCOMP_RET_KILL_PROCESS
            );
            assert_eq!(SECCOMP_RET_USER_NOTIF, libc::SECCOMP_RET_USER_NOTIF);
            assert_eq!(SECCOMP_RET_ALLOW, libc::SECCOMP_RET_ALLOW);
            assert_eq!(SECCOMP_RET_ACTION_FULL, libc::SECCOMP_RET_ACTION_FULL);

            let int = |v: libc::c_int| u32::try_from(v).unwrap();
            assert_eq!(AF_NETLINK, int(libc::AF_NETLINK));
            assert_eq!(AF_PACKET, int(libc::AF_PACKET));
            assert_eq!(AF_INET, int(libc::AF_INET));
            assert_eq!(SOCK_RAW, int(libc::SOCK_RAW));
            // libc deprecates the name because the family replaced the
            // type; the kernel still accepts the type, which is why the
            // filter judges it, so the number is still pinned to libc's.
            #[allow(deprecated)]
            let libc_sock_packet = libc::SOCK_PACKET;
            assert_eq!(SOCK_PACKET, int(libc_sock_packet));
            assert_eq!(SOCK_NONBLOCK, int(libc::SOCK_NONBLOCK));
            assert_eq!(SOCK_CLOEXEC, int(libc::SOCK_CLOEXEC));
            assert_eq!(PR_SET_SECCOMP, int(libc::PR_SET_SECCOMP));
            assert_eq!(
                u64::from(TIOCSTI),
                u64::try_from(libc::TIOCSTI).unwrap()
            );
            assert_eq!(
                u64::from(TIOCGWINSZ),
                u64::try_from(libc::TIOCGWINSZ).unwrap()
            );

            assert_eq!(size_of::<Insn>(), size_of::<libc::sock_filter>());
            assert_eq!(align_of::<Insn>(), align_of::<libc::sock_filter>());
        }
    }
}
