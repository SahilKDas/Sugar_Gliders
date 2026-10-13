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
./scripts/build.ps1 -Action build
./target/release/maple-server.exe
# In two other terminals, with distinct names:
./target/release/maple-client.exe Alice
./target/release/maple-client.exe Bob
```

For faster iteration, `cargo build --workspace --locked -j 2` uses the optimized dev
profile (`opt-level=1`, stripped symbols, no incremental cache). Those executables
are under `target/debug`. Use the footprint script between direct Cargo commands.
Run the server from the repository root so its bounded state stays in `data/`.
The server binds only `127.0.0.1:8787` (text) and `127.0.0.1:8788` (voice).

## Using Maple

- Click the A/T league badges on the left and a text channel to navigate.
- Type a message and press Enter. Escape clears the composer; the mouse wheel scrolls history.
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

Fonts are loaded from the operating system. Set `MAPLE_FONT` to a TrueType font path
if automatic discovery fails. The palette is centralized in `ui::Theme` and mirrored
in `assets/palette.json`; leaf artwork is lightweight SVG/code-generated geometry.

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
clipboard handling and DPI-scaled layouts; user-managed league/channel lifecycle and
roles/moderation; crash recovery, fuzzing/load tests, cross-platform hardware tests,
packaging/signing and operational monitoring. Local display names are development
identities and must not be treated as secure accounts. Messages are plaintext at rest.

