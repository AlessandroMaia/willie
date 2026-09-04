# Sandbox — the boundary a session runs inside

**F3.** Willie's own description says it launches agent CLIs inside its
distribution **under an OS sandbox**. Today it launches them unconfined.
This design gives the session a boundary, states what that boundary is
for, and makes the two golden rules that talk about it mean something.

## Problem

### The sandbox does not exist

`crates/willie-sess/src/` has no sandbox code: `main`, `pty`, `server`,
`events`, `signals`, `spec`, `screen`, `detach`, `cli`. The harness is
spawned on a PTY with the environment and the working directory the
spec names, and nothing else happens. `willie-core` has no capability
types either, although the repository map lists them among its domain
types. Every session so far has had the reach of the user who started
it: the whole distribution, the Windows drives through the workspace's
own path, and — through the interop interpreter — any Windows
executable.

Two golden rules describe a sandbox that is not there. Rule 3 says an
unreadable policy or a missing backend refuses to start the session;
there is no policy to read. Rule 4 says repository configuration can
only tighten what the project profile allows; neither file exists.

### Nothing says what it defends against

`docs/ARCHITECTURE.md` §3.3 specifies four mechanisms and ten named
capabilities in detail, and no versioned document says what any of it
is for. Without a stated boundary every default is unarguable: network
on by default and the agent's credentials mounted by default are either
right or catastrophic depending on an assumption nobody wrote down.

### The bytes leave the boundary by design

The supervisor copies the harness's output to whoever is attached. That
output is a control language: escape sequences can write the host's
clipboard, ask the emulator a question it answers back into the input
stream, and change the window title. The sandbox cannot help, because
this channel is the session's whole purpose. Willie has two of them,
the Windows Terminal tab and the embedded terminal, and neither
filters anything.

### A denial is invisible

A blocked action reaches the user as the agent's own confused message,
if at all. There is no record that the session tried, so "why did it
fail" has no answer outside the agent's transcript.

## Goals

- A stated boundary, so every default can be argued for or against.
- Sessions run confined: namespaces and mounts, a syscall filter and
  resource limits always; path-based restriction of the process itself
  when the kernel offers it.
- A missing required mechanism refuses the session with a code and a
  remediation. A missing optional one is named everywhere the session
  is shown, never assumed away.
- The policy a session ran under is in its immutable spec; what it was
  denied is in its append-only event log.
- The agent's output cannot drive the host terminal by default.
- Six capabilities, resolved from two configuration layers, editable
  per project.
- Most of the logic is testable on any host, without a kernel.

## Non-goals

- **The repository-level layer** (`.willie/sandbox.toml`) and therefore
  the teeth of rule 4. It is the security-critical half of the merge —
  a case where it can *open* something is a vulnerability, not a bug —
  and it only means anything once the project profile exists. Next
  slice.
- **`home.persistent`, `ssh`, `mnt.all`, `windows.interop`.** Two are
  conveniences and two can void the boundary this design adopts; none
  is needed for a session to work. They exist in the capability type as
  addresses and the daemon refuses them until they are implemented.
- **Fine-grained network egress.** The harness talks to a remote API,
  so the network is open, and shaping it belongs to the corporate
  network slice, not here.
- **A shared cache with a copy-on-write layer.** An overlay would keep
  both the speed and the isolation; it is a fifth mount mechanism in a
  slice that already lands four.
- **A disposable virtual machine per session.** See the boundary below:
  that is the tool for the workload this sandbox declines.
- **Kernel, driver and sandbox-backend escapes.** Outside the boundary
  by definition.

## Design

### The boundary

Willie's sandbox is a **process boundary for a tool the user trusts,
running content the user does not**. The agent is honest. What it reads
is not: a repository file, an issue, a dependency's release notes or a
fetched page can carry instructions, and the agent may act on them. The
sandbox exists so that acting on them stays inside the project.

It is not a malware boundary. Code written to break out — a kernel bug,
a driver, a defect in a sandbox backend — is outside it, and the answer
for that workload is a disposable virtual machine, which Willie does
not provide.

Two consequences follow, and they are why this section comes before the
mechanisms.

**The network cannot be closed, so the defence is what is readable.**
The harness is an API client; without egress there is no session. An
injected agent can therefore send anywhere whatever it can read. Every
capability decision below follows from that: the home directory is a
tmpfs, the Windows drives are absent, the caches are per project,
`extra.paths` starts empty, and each of the four deferred capabilities
widens exactly this.

**The credentials are inside.** `agent.state` mounts the harness's login
so a session can work at all, and anything the agent runs can read it.
Combined with the open network that is a declared residual risk, not an
oversight: a session is as trusted as the harness's own credentials, and
the way to reduce the blast radius is to keep sessions short and
projects separate, not to pretend otherwise.

### Capabilities — `willie-core/src/sandbox.rs`

A capability has a positive name, a default, and one sentence of
consequence that the UI shows verbatim. Six ship:

| Capability      | Default          | Binds                                                | If enabled                                                  |
| --------------- | ---------------- | ---------------------------------------------------- | ----------------------------------------------------------- |
| `project.rw`    | always           | the workspace, read-write, at the same path          | the agent edits the project, which is the point             |
| `agent.state`   | on (Claude Code) | the harness's `agent_state()`: its directory under `~/.willie/agent-state/`, read-write, and the links into it | anything in the session can use the harness's login |
| `tools.ro`      | on               | the managed tool locations, read-only                | the agent runs the tools and cannot alter them              |
| `caches.rw`     | on               | a per-project directory over each package cache path | downloads survive between this project's sessions           |
| `git.identity`  | on               | the user's git configuration, read-only              | commits carry the user's name and address                   |
| `extra.paths`   | empty            | what the project profile lists, read-only or read-write | each entry is reach outside the project, and is recorded  |

`CapabilitySet` is a record of those six, with no I/O, in `willie-core`
beside the session types. The four deferred names exist in the same
enum so the profile format does not change when they arrive; resolving
one refuses the session.

The harness binary itself, `argv[0]`, is bound read-only at its own
path whatever the policy says. With `tools.ro` off the tools are
hidden, but a session that cannot start is not a tighter policy, it is
no session at all.

`caches.rw` binds a **per-project** directory
(`~/.willie/caches/<project_id>/<cache>`) over each cache path
rather than sharing one. A shared writable cache is the single place
where one project's compromise reaches another: a poisoned package in
the shared store installs into the next project that asks for it. The
cost is disk and a cold first install per project, which is the right
trade under the boundary above.

### Resolution — `willied`

Two layers, in increasing authority:

1. **Willie defaults**, per harness, from
   `Harness::default_capabilities()`.
2. **The project profile**, `/var/lib/willie/projects/<id>.toml`,
   edited by the app, which may enable or disable any shipped
   capability.

The **daemon** merges them while handling `session.create` and writes
the result into `spec.json`, which §3.2 already defines as immutable
after creation. The supervisor never reads daemon state: it applies
what the spec carries. That keeps the property which lets a supervisor
outlive its daemon, and it makes the session record say what policy it
ran under, beside the events that say what that policy refused.

An unknown key, a wrong type, or a capability not yet implemented
refuses the session at resolution time, in the daemon, before any
process exists.

### Applying it — `willie-linux/src/sandbox/`, `willie-sess/src/sandbox/`

```
willie-linux/src/sandbox/mod.rs      the plan: policy to mounts, as data
willie-linux/src/sandbox/bwrap.rs    the argument vector, as data
willie-sess/src/sandbox/mod.rs       the required subset, and the report of what applied
willie-sess/src/sandbox/seccomp.rs   the filter program, and the notification loop
willie-sess/src/sandbox/landlock.rs  applied after re-exec, immediately before exec
willie-sess/src/sandbox/rlimits.rs   process, descriptor and core limits
```

The two data files sit in the shared Linux crate rather than in the
supervisor because `sandbox explain` prints the same vector from the
daemon, and the daemon cannot depend on the supervisor binary. The
supervisor applies a plan; the daemon shows one.

**The base, not configurable:** own user, pid, ipc and uts namespaces,
dying with the supervisor; no new privileges; system paths read-only; a
tmpfs home; a private temporary directory; a fresh process filesystem;
a minimal device tree; no interop interpreter, none of its environment
and none of its sockets, so no Windows executable runs — the kernel
holds the interpreter open whatever the namespace contains, so what
stops one is the socket it cannot reach (0016); the Windows
drives unmounted except the project itself; privilege escalation
helpers masked; Willie's own state and runtime directories absent; an
environment allowlist rather than a denylist.

**The syscall filter** denies process tracing and cross-process memory,
the kernel-program interfaces, performance counters, page-fault
handling, the mount family, namespace manipulation, module loading,
kernel replacement, the key management calls, terminal input injection,
and packet and raw sockets. One narrow exception: the netlink route
protocol, which interface enumeration needs. A session whose network is
open learns the same addresses by connecting out, so denying it buys
nothing and breaks ordinary tools.

**The required subset.** Namespaces and mounts, the syscall filter and
the limits are required: if any is unavailable the session does not
start. Path-based restriction of the process is applied when the kernel
offers it, and its absence is reported, because the mounts already do
the primary work and this mechanism is depth on top of them. That is
rule 3 honoured on the side that matters — nothing degrades in silence
— without a kernel Willie does not build turning every session into a
refusal.

The supervisor emits one `sandbox_applied` event naming what it managed
to apply, into the log it already keeps, so a session that ran with
less says so forever.

### The denial log — `willie-sess/src/sandbox/seccomp.rs`

Every syscall the filter refuses is not merely denied, it is
**observed**. Each reaches the supervisor through a notification
descriptor, which appends a `sandbox_denied` event naming the syscall
and answers with a refusal. One class, not two: a filter where some
refusals are logged and others are silent is a distinction to maintain
with nothing to show for it.

Those attempts are rare by construction, so the log does not grow, and
the supervisor answers from a tight loop with a canned refusal, so a
program that probes a denied call repeatedly pays a round trip rather
than a stall.

The mechanism is chosen because it is the only one that produces an
event on this kernel. Mounts deny by absence: an unmounted path is a
missing file and a read-only bind is a read-only filesystem, and only
the child sees the error. Path-based restriction is silent by design,
and kernel-side audit for it exists only in kernels far newer than the
one Willie runs on. So file denials stay visible as the agent's own
error, and the syscall class is the one Willie can name.

If the notification thread dies, the intercepted syscalls fail. That is
the closed direction, and the session records it and continues.

### The terminal filter — `willie-sess/src/vt_filter.rs`

The one channel that crosses the boundary by design gets the one
defence that is not a kernel mechanism. Output travels through a small
parser before reaching whoever is attached. By default it drops the
sequences that act on the host rather than draw on it: writing the
host's clipboard, the queries an emulator answers back into the input
stream, and window-title changes. Everything that draws — colour,
cursor motion, screen regions, the modes a full-screen interface needs
— passes untouched.

It sits beside the PTY relay, not under `sandbox/`, because it is not
confinement: it is sanitising bytes that are supposed to leave.

### Explaining it — `willie-cli`, the app

`willie sandbox explain <project>` asks the daemon and prints the
resolved capabilities, the argument vector, the filter summary and the
path rules, so "what is forbidden" is answerable without starting a
session. In the app the same answer sits beside the capability editor
that layer 2 needs, and a session row shows the applied state and its
denials.

### Errors and edge cases

| Condition                                      | Code                             | Behaviour                                                              |
| ---------------------------------------------- | -------------------------------- | ---------------------------------------------------------------------- |
| a required mechanism is unavailable            | `sandbox_backend_missing`        | the session does not start; the remediation names the mechanism         |
| the kernel offers no path-based restriction    | —                                | the session starts; the applied event says so, and every view shows it  |
| the profile names a deferred capability        | `sandbox_capability_unsupported` | refused in the daemon, before any process exists                        |
| an `extra.paths` entry is not an absolute path | `sandbox_profile_invalid`        | refused in the daemon; the message names the path                       |
| a backend refuses at launch                    | `sandbox_apply_failed`           | the session does not start; the backend's own message is carried        |
| the notification thread dies                   | —                                | intercepted syscalls fail; an event records it; the session continues   |
| `extra.paths` names a path outside the project | —                                | allowed, that is its purpose, and recorded in the spec                  |
| a per-project cache path does not exist yet    | —                                | created before the bind; a session never starts without its caches      |

An unknown key or a wrong type has no row, because it never reaches
resolution: the profile is a section of the project's own record, so
`toml` refuses the whole record and the daemon skips it at start-up —
the project disappears from the Projects screen with no coded error.
Holding the section as a deferred-parse value, so a bad profile fails
by itself while the project still loads, is the named follow-up.

## Testing

Most of the logic never touches a kernel, and that half runs on any
host:

- the merge of the two layers, including every refusal above;
- the argument vector as data, asserted flag by flag against a
  capability set — the base restrictions are the assertions that matter,
  since a missing one is a hole;
- the filter program as data, with its deny list and its one exception;
- the capability-to-bind mapping, including the per-project cache path;
- the terminal filter over byte sequences: the acting ones dropped, a
  full-screen drawing stream unchanged.

The rest needs the distribution, through `cargo xtask test-linux`: a
session that tries to read outside its project, to reach the interop
interpreter, to trace another process, and to write into a read-only
tool path, each asserting the refusal, and the syscall cases asserting
the event in the log. One more asserts that a session starts at all
with the base applied, and that the harness logs in with `agent.state`
and a tmpfs home.

**Spike S3 runs before implementation**, as `docs/ARCHITECTURE.md` §5.4
already schedules: which mechanisms this kernel offers, the
path-restriction interface version, whether an unprivileged user
namespace is available, and that no Windows executable is reachable
from inside. Its outcome is recorded as a decision. The design does not
depend on the answer, because the required subset is specified either
way.

## Rollout / compatibility

The protocol gains the resolved capability set inside the session spec
and two event kinds; both are additive, and a spec written before this
slice resolves to the defaults. `engine.toml` does not change. The
distribution already ships the namespace helper, so the image does not
change either.

Sessions become confined, which is a behaviour change users notice: a
session can no longer reach the Windows drives outside its project, run
a Windows executable, or write into the managed tools. The release note
says so plainly, because someone whose workflow depended on that reach
needs to know why it stopped, and `extra.paths` is where it comes back.

One task, one commit:

1. `CapabilitySet` and the six capabilities in `willie-core`
2. the harness defaults, and the profile's format and reader
3. the daemon's resolution into the spec, with its refusals
4. the argument vector and the mounts, as data; then the launch through
   them — *both landed*
5. the syscall filter, denying only
6. the limits, and the required-subset decision with its report
7. path-based restriction, applied after the re-exec
8. the notification loop and the denial event
9. the terminal filter
10. `sandbox explain`
11. the capability editor, and the session's applied and denied state
12. `docs/ARCHITECTURE.md`, the acceptance checklist, the release note

## Open questions

- Does the per-project cache want a size ceiling? Favoured: no, until a
  project's cache actually hurts. A ceiling needs an eviction policy and
  that is a slice of its own.
- Should the terminal filter be switchable per project? Favoured: not
  yet. Nothing needs the acting sequences, and a switch that restores
  them is a capability that widens the one channel the boundary cannot
  otherwise defend.
- Does `sandbox explain` belong in the session row as well? Favoured:
  the applied state and the denials, yes; the full argument vector, no.
  It is long, and the editor is where somebody comparing policies is
  already looking.
