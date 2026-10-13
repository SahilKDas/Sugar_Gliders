# Maple

A small native Rust IM and voice application using tiny-skia, winit, softbuffer,
Tokio WebSockets, CPAL, and Opus. No webview or monolithic UI engine.

This repository is a **local development application**, not a public-service release.
It implements native rendering, text delivery, private membership checks, persistent
and ephemeral small chats, two navigable leagues, and two isolated local voice rooms.
Public deployment requires the work listed under release requirements below.

## Build and run

Requires Rust, a C/C++ compiler, and CMake for bundled libopus. On Windows install
Visual Studio C++ build tools; this workstation needs `CMAKE_GENERATOR="Visual Studio 17 2022"`
because its CMake predates Visual Studio 2026. Linux additionally needs ALSA development
headers and the native window system development packages. Windows is the validated build target.

```powershell
./scripts/package.ps1
./dist/Maple.exe Alice
# Optional second local client, from another terminal:
./dist/Maple.exe Bob
```

The default package uses the optimized dev profile (`opt-level=1`, stripped symbols,
no incremental cache), keeping builds within the size budget. Use
`./scripts/package.ps1 -Profile release` for thin-LTO release packaging. Developers
can use `cargo build --workspace --locked -j 2`; the standalone server remains an
optional test tool and stores state in `data/` when run from the repository root.
The server binds only `127.0.0.1:8787` (text) and `127.0.0.1:8788` (voice).

## Using Maple

- Click the A/T league badges on the left and a text channel to navigate.
- Click the composer or press Tab, then type and press Enter or click Send. Escape
  clears the composer; the mouse wheel scrolls history. F1 opens About and licenses.
- Click Join voice to connect to the selected league's lounge, and Leave voice to disconnect.
  Another local client must join the same lounge to hear you. A speaking ring shows
  your microphone activity. Audio starts only when you click Join voice.
- Chat management uses the composer commands below. Names in commands must not contain
  spaces. Launch a client with that participant name to access its chats.

| Command | Behavior |
| --- | --- |
| `/dm cozy Bob` | Create a persistent two-person chat with Bob |
| `/group friends Bob,Rowan` | Create a persistent chat, including yourself |
| `/ephemeral quick Bob` | Create a chat removed when all members disconnect |
| `/join friends` | Open a chat, including one below the visible sidebar |
| `/invite Rowan` | Invite a participant to the current chat |
| `/leave` | Leave a chat with at least three participants |
| `/close` | Close a chat for all participants |

The server enforces 2–10 distinct members and checks membership on every private
join, message, invitation, leave, close, and delivery. A two-person chat must be closed
instead of reducing membership to one. Any member can invite or close a local chat.
Persistent chat membership and the latest 100 messages per channel survive restarts.
Ephemeral chats and their histories are excluded from disk snapshots.

## Architecture

- `maple-protocol`: typed wire commands/events, league/channel schemas, membership invariants.
- `maple-client`: input/state store, tiny-skia layouts, fontdue system-font rendering,
  softbuffer presentation, bounded asynchronous networking queues.
- `maple-server`: bounded loopback connections, text rooms, membership authorization,
  snapshot storage, and isolated UDP voice relay.
- `maple-audio`: 48 kHz mono Opus, 20 ms frames at 32 kbps, native CPAL capture/playback,
  downmix/upmix for native channel counts, separate decoders per speaker, 60 ms initial
  jitter delay, sequence wrap/reorder handling, loss concealment, bounded mixing queues.

Audio devices must support 48 kHz float samples; unsupported devices produce a visible
error. The custom UDP frame is `MPL1 | room:u32 | stream:u32 | sequence:u16 | Opus payload`,
all integers big endian, at most 1,289 bytes. It is **not RTP**. The loopback relay
rewrites the stream identifier using the sender's bound port and expires idle peers.
Stream teardown stops capture/playback and joins the worker. There is no echo cancellation.

Noto Sans is embedded in the executable, with its OFL license embedded in the About
screen (F1 or click the maple logo). The palette is centralized in `ui::Theme` and
mirrored in `assets/palette.json`; leaf artwork is code-generated geometry. The
Windows build statically links the C runtime and Opus. Normal Windows system DLLs
remain operating-system dependencies.

## Single-executable distribution

`scripts/package.ps1` creates `dist/Maple.exe`; distribute that one file. Fonts,
licenses, rendering, audio codec, and the local server are embedded. Double-click it
to open Maple and start the backend automatically. Another instance connects to the
same backend; if its hosting instance closes, remaining instances retry and one takes
over hosting. Supply a display name with `Maple.exe Alice` when testing multiple users.
This remains a local development identity, pending the account implementation.

The app stores writable chat data in `%LOCALAPPDATA%/Maple`, not inside the executable.
Set `MAPLE_DATA_DIR` to override that directory for tests. No asset directory or
third-party DLLs need to accompany the executable. The optional `maple-server` binary
is retained as a developer tool, but is not included in the distribution.

The composer shows a caret and a Send button. Click it or press Tab to focus it.
Channel navigation and drafts work while offline, and the network task reconnects
automatically. Layout and hit testing use the same logical coordinates on high-DPI
displays. A/T select leagues; the maple logo opens About and licenses.

## Backend and security plan — awaiting review

The next backend should remain a modular Rust server, compiled both as a library
for local hosting and as an optional daemon for a public deployment. The native client
will remain one executable. These security changes are **planned, not implemented**.

| Concern | Proposed choice | Reason |
| --- | --- | --- |
| Concurrency | Tokio with bounded queues and blocking-worker limits | Fits the existing workspace and keeps audio/UI work separate |
| Secure text/control | Quinn QUIC bidirectional streams, rustls TLS 1.3 | Reliable ordered control on an encrypted connection |
| Secure voice | Opus in QUIC DATAGRAM frames on that authenticated connection | Encrypted UDP media without retransmitting old audio |
| Accounts | RustCrypto Argon2id with unique random salts | Store password hashes; keep passwords out of logs and command-line arguments |
| Storage | SQLite via rusqlite, WAL mode, versioned migrations | Self-contained local/single-node backend without a database service |
| Sessions | Random opaque tokens; only token hashes stored server-side | Explicit expiry, rotation, revocation, and logout |
| Client secrets | Windows Credential Manager for remembered sessions | Avoid plaintext tokens in files; keep sessions in memory by default |
| Deployment | One server process with a persistent data directory | Add PostgreSQL only when multi-node deployment requires it |

Quinn exposes reliable streams and unreliable datagrams; its datagram size must be
queried per connection rather than assuming a fixed packet size. References:
[Quinn Connection](https://docs.rs/quinn/latest/quinn/struct.Connection.html),
[RustCrypto Argon2](https://docs.rs/argon2/0.5.3/argon2/),
[SQLite WAL](https://www.sqlite.org/wal.html).

Account schema: immutable user IDs, normalized unique handles, display names, Argon2id
PHC hashes, creation timestamps, and account status. Session schema: token digest,
user ID, device label, expiry, and revocation timestamp. Chat memberships must reference
immutable account IDs rather than the current unverified display names. Existing demo
labels must not automatically grant ownership of old private chats to newly registered
accounts; import requires explicit local-owner mapping.

The native sign-in/register screen will send credentials only after server certificate
verification succeeds. Public servers use certificates issued for their domain; local
development uses an explicitly trusted development certificate. Never disable certificate
or hostname verification. Disable QUIC 0-RTT for login, registration, and chat mutations.
Registration/login attempts need per-address and per-account throttles, uniform login
errors, and bounded Argon2 workers so hashing cannot stall the async runtime.

The server binds each connection to a validated session and resolves the sender's user
ID itself. A voice join is authorized over the reliable control stream; datagrams are
accepted only from currently authenticated members of that voice room. Ignore claimed
sender IDs in media payloads. Revocation/logout removes voice access and closes affected
connections. TLS/QUIC protects traffic between client and server; the server can still
read messages and relay decrypted audio. End-to-end encryption would be a separate design.

Implementation order:

1. Add account/session storage, migrations, and a native sign-in/register flow.
2. Introduce a shared secure transport module with verified TLS, bounded framing,
   protocol version negotiation, reconnect, and session resumption.
3. Replace the plaintext WebSocket text endpoint and raw UDP voice relay together;
   fail closed on old insecure endpoints. Keep CPAL/Opus/jitter processing behind a
   transport abstraction. Preserve one-file packaging.
4. Bind memberships and voice permissions to account IDs; implement logout, token
   expiry/revocation, and remembered-session storage.
5. Test invalid passwords, duplicate accounts, restart persistence, rate limits,
   untrusted/expired/wrong-host certificates, unauthorized voice, revocation, and
   packet tampering/replay rejection by QUIC. Check that packet captures contain no
   plaintext text/Opus payloads and that missing certificates never cause downgrade.

A later verified WSS fallback can support networks that block UDP; it must not downgrade
to plaintext. Public certificate issuance, hosting, account recovery email, and deployments
remain outside this local change. Keep TLS cryptography in audited libraries; do not add
custom media encryption. Use minimal crate features and retain the 750 MB pruning threshold
and strict 1 GB workspace limit throughout the implementation.

## Verification and size budget

```powershell
./scripts/build.ps1 -Action test
./scripts/build.ps1 -Action clippy
./scripts/footprint.ps1
# Headless output from the actual UI renderer:
./target/debug/maple-client.exe --render assets/preview.png
```

The integration test starts its own server; ports 8787/8788 must be free. It checks
private delivery, outsider rejection, history replay, membership limits, disk snapshot
creation, and UDP forwarding. Unit tests cover membership, bounded history, packet
validation, sequence wrap/loss/reordering, and a real Opus encode/decode round trip.
Microphone/speaker hardware and perceptual audio quality require a manual two-client test.

All profiles strip debug information. Release uses thin LTO, one codegen unit, and abort
on panic. `.gitignore` excludes targets, runtime data, caches, raw audio, and dumps.
The build helper measures every 500 ms, stops compilation above 750 MB, prunes only
verified intermediate target paths, and checks the strict 1,000,000,000-byte ceiling.
It includes hidden `.git` files. Cargo's shared registry is outside this workspace;
do not vendor it here. Use the helper for subsequent builds to keep this budget monitored.

## Release requirements

Before public use: authenticated identities and invitation consent; TLS for control
traffic and authenticated encrypted media; reconnect/retry acknowledgments and stable
message identifiers; transactional async storage rather than synchronous JSON snapshots;
device hot-plug/resampling and echo cancellation; accessibility, full IME/text editing,
clipboard handling and broader DPI/accessibility testing; user-managed league/channel lifecycle and
roles/moderation; crash recovery, fuzzing/load tests, cross-platform hardware tests,
packaging/signing and operational monitoring. Local display names are development
identities and must not be treated as secure accounts. Messages are plaintext at rest.

