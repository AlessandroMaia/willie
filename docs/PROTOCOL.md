# Control protocol

Transport-agnostic ndjson: one JSON-RPC 2.0 object per line, UTF-8, `\n`
terminated. The same messages travel over the engine's stdio pipe to the
daemon, over the daemon's Unix socket to local clients and, later, over
TCP loopback. Types live in `crates/willie-proto`.

## Envelopes

`id` is a `u64`, `method` is `<ns>.<name>`, `params` is an object and
`result` is any JSON value.

Request:

```json
{ "jsonrpc": "2.0", "id": 1, "method": "daemon.health", "params": {} }
```

Response, either a result or an error:

```json
{ "jsonrpc": "2.0", "id": 1, "result": { "pid": 42 } }
```

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": "snake_case",
    "message": "one sentence",
    "remediation": "what to do"
  }
}
```

Notification (daemon → client), recognised by having no `id`:

```json
{ "jsonrpc": "2.0", "method": "state.event", "params": {} }
```

## Compatibility rules
- Unknown fields are ignored; new fields have defaults. Additive
  changes keep `PROTOCOL_VERSION`.
- A client sends `daemon.hello` first. Engine and daemon must be the
  same Willie version. F0 reports a mismatch as `version_mismatch` with
  the remediation to reinstall the distribution; updating the daemon
  binaries in place is a later slice.
- The daemon exits when its stdin reaches EOF.
- Lines on the daemon's stdout that are no envelope are ignored. The
  engine keeps the last few of them: when `wsl.exe` itself refuses to
  start the daemon, its message arrives there and is the only account
  of the failure.

## `daemon.*`
| Method | Params | Result |
| --- | --- | --- |
| `daemon.hello` | `Hello { client, willie_version, protocol_version }` | `HelloReply { willie_version, protocol_version, distro_image_version? }` |
| `daemon.health` | `{}` | `Health { pid, uptime_secs, willie_version }` |
| `daemon.doctor` | `{}` | `DoctorReport { checks: [ { name, status: ok|fail|skip, detail, remediation?, required } ] }` |
| `daemon.shutdown` | `{}` | `null` — the daemon replies, then exits 0 |

## Error codes (daemon)
| Code | Meaning |
| --- | --- |
| `method_not_found` | unknown method |
| `invalid_params` | params did not deserialise |
| `invalid_request` | the line was not a JSON-RPC request (malformed JSON or missing fields); the daemon answers with id 0 and keeps serving |
| `protocol_version_mismatch` | client speaks another `PROTOCOL_VERSION` |
| `internal_error` | handler failure; message says what, remediation says what to do |

## Engine problem codes

These are produced on the Windows side, not by the daemon. They cross
the app's IPC boundary as `Problem { code, message, remediation }` and
the UI keys behaviour on them, so they are stable once released.

| Code | When | When not | Remediation |
| --- | --- | --- | --- |
| `wsl_not_installed` | `wsl.exe` could not be spawned at all | it ran and answered something unexpected — that is `wsl_unparseable_output` | enable WSL 2.4.4 or newer (administrator) and retry |
| `wsl_command_failed` | a `wsl.exe` management command exited non-zero | the command worked and only its output was surprising | run the same command in PowerShell, then click Run doctor again |
| `wsl_unparseable_output` | `wsl.exe` answered, but not with what the reading needs (no version in `--version`) | the command failed — that is `wsl_command_failed` | run `wsl --version` in PowerShell, update WSL if it is old, then click Run doctor |
| `wsl_io` | the pipe to `wsl.exe` broke while the command was running | the child process died — that is `daemon_exited` | click Run doctor (it restarts the daemon) |
| `daemon_error` | the daemon answered with a well-formed RPC error: it is alive and refused | the daemon is silent, dead or off-protocol | the daemon's own remediation when it sent one, otherwise click Run doctor to retry |
| `daemon_timeout` | no reply within the call budget (60 s for `hello`, 10 s after) | the call was answered with an error — that is `daemon_error` | click Run doctor (it restarts the daemon) |
| `daemon_transport` | writing to or reading from the daemon's pipe failed | the child exited and the exit explains it — that is `daemon_exited` | click Run doctor (it restarts the daemon) |
| `daemon_exited` | the child process is gone; the detail is its stderr, or `wsl.exe`'s own stdout message when stderr is silent | the daemon is alive but slow or wrong | by detail: the service-logon right, Install distribution for a missing one, otherwise one more Run doctor |
| `protocol_violation` | a line was an envelope but not usable: closed stdout, or a result the method cannot hold | the line was no envelope at all (those are ignored) | click Run doctor (it restarts the daemon); if it repeats, click Install distribution |
| `version_mismatch` | `hello` reported a Willie version other than the app's | the protocol version differs — the daemon answers `protocol_version_mismatch` itself | click Install distribution: it reinstalls the distribution with binaries matching this app |
| `image_invalid` | the image beside the app is missing, empty, or its sha256 does not match the `.sha256` sidecar | there is no image at all — that is `image_not_found` | rebuild the image with `just distro-build`, then click Install distribution again |
| `image_not_found` | no image in any candidate location (`WILLIE_ROOTFS`, the resource dir, `target/distro`) | one was found and rejected — that is `image_invalid` | reinstall Willie; in development run `just distro-build` |
| `daemon_not_running` | a call was made while no daemon is live | the daemon was live and died — that is `daemon_exited` | click Run doctor (it starts the daemon) |
| `distro_not_registered` | the pre-flight before a start found no `willie` distribution | the distribution exists and its daemon failed — that is `daemon_exited` | click Install distribution |
