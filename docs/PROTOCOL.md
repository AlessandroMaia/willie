# Control protocol

Transport-agnostic ndjson: one JSON-RPC 2.0 object per line, UTF-8, `\n`
terminated. The same messages travel over the engine's stdio pipe to the
daemon, over the daemon's Unix socket to local clients and, later, over
TCP loopback. Types live in `crates/willie-proto`.

## Envelopes
- Request: `{"jsonrpc":"2.0","id":<u64>,"method":"<ns>.<name>","params":<object>}`
- Response: `{"jsonrpc":"2.0","id":<u64>,"result":<any>}` or `{"jsonrpc":"2.0","id":<u64>,"error":{"code":"snake_case","message":"…","remediation":"…"}}`
- Notification (daemon → client): `{"jsonrpc":"2.0","method":"<ns>.<name>","params":<object>}` — no `id`.

## Compatibility rules
- Unknown fields are ignored; new fields have defaults. Additive changes keep `PROTOCOL_VERSION`.
- A client sends `daemon.hello` first. Engine and daemon must be the same Willie version; on mismatch the engine updates the daemon binaries and reconnects.
- The daemon exits when its stdin reaches EOF.
- Non-JSON lines on the daemon's stdout are logged by the client and ignored.

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
| `protocol_version_mismatch` | client speaks another `PROTOCOL_VERSION` |
| `internal_error` | handler failure; message says what, remediation says what to do |
