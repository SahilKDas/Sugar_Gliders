# Maple

A maple-themed native Rust chat and voice app: tiny-skia + winit + softbuffer, CPAL,
Opus, Tokio, certificate-verified rustls TLS 1.3 QUIC, Argon2id accounts, and SQLite.
The client, local backend, codec, font, and licenses ship in **one Windows executable**.
No webview, asset folder, or third-party runtime DLL is required.

## Run

Double-click `dist/Maple.exe`. The embedded backend starts automatically. Choose
**New here? Create an account**, enter a username and password, and click **Create
account**. Subsequent launches use **Sign in**. An optional command-line username,
such as `Maple.exe alice`, prefills the username; it does not authenticate the user.
Never supply a password or session token on the command line.

Usernames use 3–32 lowercase ASCII letters, digits, or underscores. Registration
requires at least 12 password characters, with a maximum of 256 UTF-8 bytes. Password
fields are masked. Tab selects the next field, Enter submits, and F1 opens About.

Multiple local instances can sign in to separate accounts on the same backend. All
instances share the persisted server identity and database. If the hosting instance
closes, another instance takes over and clients reconnect with a rotated in-memory
session. An account can have multiple concurrent sessions. Use distinct accounts for
a two-client chat/voice test.

## Native polish and sounds

Rounded parchment cards, avatar initials, selected-channel accents, hover feedback,
blinking carets, and a gently pulsing speaking ring keep the native interface clear.
Windows text uses the system DirectWrite software renderer and Segoe UI, including
real ClearType RGB/BGR subpixel coverage, font shaping, fallback, and monitor rendering
parameters. tiny-skia still draws every UI surface; the portable fallback uses the
embedded Noto Sans font. Text is drawn at physical DPI, never enlarged after rendering.
Cached native layouts supply matching caret positions and wrapping without repeated
prefix shaping. ClearType blends directly into opaque backgrounds and selections.

The editor supports cursor placement/drag selection in chat and sign-in fields,
Shift selection, Ctrl+arrows, Home/End, Ctrl+A, and Windows Ctrl+C/X/V. Double-click
selects a word; triple-click selects the draft. Ctrl+Backspace/Delete removes words;
Ctrl+Z and Ctrl+Y (or Ctrl+Shift+Z) undo/redo drafts with bounded, coalesced history.
Passwords cannot be copied/cut and are excluded from undo history. IME preedit is
underlined, its candidate window follows the caret, and password composition stays
masked. Windows caret width and blink settings are respected; typing resets the blink,
focus loss hides the caret, and navigation is Unicode grapheme aware.

UI animation pauses when the window loses focus. Warm synthesized chimes distinguish
sending/receiving messages, sign-in/out, navigation, voice join/leave, errors, and
sound toggles; typing adds a quiet short wooden tick. Sounds use output-only CPAL,
precomputed samples, a bounded queue, and a 75 ms event throttle. Missing or unsupported
output devices leave the app usable and silent. No sound assets or DLLs are shipped.

Click **SFX** at the bottom left or press **F3** to mute/unmute all interface sounds.
This preference persists locally and does not mute your voice conversation.

## Security behavior

- Text/control uses reliable QUIC streams; voice uses unreliable QUIC DATAGRAM
  frames. **Both are encrypted and authenticated by TLS 1.3.** The plaintext WebSocket
  and raw UDP audio endpoints have been removed. The backend uses UDP port 8787 only.
- rustls verifies the server certificate and hostname. The native client trusts only
  the persistent certificate provisioned by its local backend in the user's data
  directory, pins it in memory for the app's lifetime, and uses hostname `localhost`.
  Certificates received from the network are never automatically trusted. QUIC 0-RTT
  is disabled. Verification failure never causes a plaintext fallback.
- Registration/login occurs only over the verified encrypted connection. Passwords
  use unique random salts and Argon2id (64 MiB, three iterations, one lane). Hashing
  runs in blocking workers with a two-worker semaphore, not on audio or UI threads.
- Account IDs are randomly generated immutable 128-bit identifiers. Membership and
  stored message authors reference these IDs; usernames are resolved for display.
  An unverified display name can no longer claim a private chat.
- Session tokens use 256 bits of OS randomness. Only SHA-256 token digests are stored
  in SQLite. Sessions expire after one hour, are checked on commands and periodically,
  and are revoked on logout. Resume atomically rotates the token and closes its old
  connection. Up to eight persisted sessions per account are retained.
- Tokens are held in memory on the client, never saved to ordinary files. Secrets
  are redacted in Debug output and zeroized on drop, including serialized transport
  buffers. Remembering sessions across app restarts is intentionally not enabled.
- Authentication is throttled per address (20 attempts/minute) and per account
  (eight attempts/minute). Unknown-user login checks still verify a dummy hash and
  return the same error as an incorrect password. Connections, frames, queues, account
  count, chat histories, and media rate are bounded.
- Voice requires an authenticated session and an authorized room join on the control
  stream. The server ignores claimed sender IDs, rewrites them from the authenticated
  connection, isolates rooms, caps room occupancy at ten, and stops delivery after
  leave/logout/revocation. QUIC supplies packet authentication and replay protection.
- Sign out is visible in the header and also available with F2 or `/logout`. It stops
  audio and clears cached private messages/drafts. Switching accounts clears the
  previous account's chat state.

This is **transport encryption**, not end-to-end encryption: the server reads messages
and decrypts/relays media. The chat database is not encrypted at rest. The current
launcher is a local deployment, bound to `127.0.0.1`; internet hosting, domain
certificate provisioning, recovery/MFA, moderation, and signed installers are separate
deployment/product work. Never expose a development certificate/key as a public service.

## Data and compatibility

Runtime state is under `%LOCALAPPDATA%/Maple/secure-v2` on Windows. Set `MAPLE_DATA_DIR`
to choose a different parent directory; `secure-v2` is appended. Files include
`accounts.sqlite` (accounts, sessions, bounded chat snapshots), `server.der` (public
certificate), and `server.key` (private PKCS#8 identity). These are writable application
data, not distribution assets. Protect and back up this directory as private user data.

The earlier `state.json` demo remains untouched. It is not imported automatically:
demo usernames were not verified, so assigning their old memberships to new accounts
would grant private-chat access without proof of ownership. Do not replace a server
identity while clients are running; they will correctly reject a different identity.
Account recovery and credential migration are not currently exposed in the UI.

The schema is versioned at v1. SQLite uses WAL, busy timeouts, transactions, secure
deletion, bounded snapshots, and WAL checkpoints. Persistent chats retain the latest
100 messages per channel across restarts. Ephemeral chats are omitted from snapshots
and removed when their last online member disconnects. Application mutations serialize
through the state store; SQLite work runs on blocking workers, and socket writes do
not hold the chat-state lock. This single-node implementation stores chat snapshots;
normalized message tables and PostgreSQL can replace that storage if future scale
requires them.

## Chat and voice

Click A/T league badges and text channels. Click the composer or press Tab, type, and
press Enter or click Send. Escape clears the composer; the mouse wheel scrolls history.
Navigation and per-channel drafts work during brief disconnections. Unknown/private
channels are still authorized by the server, independently of client selection.

| Command | Action |
| --- | --- |
| `/dm cozy bob` | Persistent two-person chat with registered account `bob` |
| `/group friends bob,rowan` | Persistent group including yourself |
| `/ephemeral quick bob` | Chat removed when all members disconnect |
| `/join friends` | Open a chat, including one below the visible sidebar |
| `/invite rowan` | Invite an existing account to the current chat |
| `/leave` | Leave a chat with at least three members |
| `/close` | Close the current chat for every member |
| `/logout` | Revoke this session and sign out |

Chats require 2–10 distinct accounts. Close a two-person chat instead of reducing it
to one. Any member can invite or close a chat. The two current leagues are public to
signed-in accounts; each has an isolated voice lounge. Click Join voice, with another
signed-in instance in the same lounge, to talk. Capture starts only after the server
accepts that join. A speaking ring reflects local microphone activity.

Audio uses 48 kHz mono Opus, 20 ms frames at 32 kbps, downmix/upmix for supported
native channel counts, separate per-speaker decoders, a 60 ms initial jitter delay,
sequence wrap/reorder handling, loss concealment, and bounded mixing queues. Devices
must support 48 kHz float samples; unsupported devices show an error. There is no echo
cancellation or device hot-plug UI yet. Headphone/two-device audio testing remains
necessary for perceptual quality and real device compatibility.

## Workspace and build

- `maple-protocol`: typed commands/events, redacted secret values, membership invariants.
- `maple-transport`: verified TLS/QUIC configuration, bounded framing, certificate identity.
- `maple-server`: SQLite accounts/sessions, authentication, authorization, text/voice relay.
- `maple-audio`: CPAL capture/playback and Opus/jitter/mixing over authorized QUIC.
- `maple-client`: native sign-in form, input/state, tiny-skia raster UI and presentation.

Requires Rust 1.98 or later, CMake, and a native C/C++ compiler. Windows is the validated
target. Visual Studio 2022 C++ tools work with this workstation's older CMake. Linux
additionally needs ALSA and native window-system development packages.

```powershell
./scripts/package.ps1             # optimized, stripped, static-CRT native package
./scripts/build.ps1 -Action test
./scripts/build.ps1 -Action clippy
./scripts/footprint.ps1
./dist/Maple.exe alice
```

Use `package.ps1 -Profile release` for thin-LTO release packaging. Defaults use the
optimized dev profile (`opt-level=1`, stripped symbols, no incremental cache). The
Windows C runtime, Opus, SQLite, and TLS crypto are statically linked. The packaging
script audits PE imports and rejects non-system runtime DLL dependencies. A repeated
packaging check can verify an identical running executable without overwriting it;
close the app to replace a changed executable.

The size helper includes hidden `.git`, assets, build artifacts and caches. Builds are
monitored every 500 ms, stopped above 750 MB, and verified intermediate directories
are pruned. The hard ceiling is 1,000,000,000 bytes. Completed MSVC PDB files can also
be pruned after linking; they are not required by the packaged app. Cargo's shared
registry is outside the workspace; do not vendor it here. Native fonts, palettes,
leaf geometry, and licenses are embedded. No asset folder accompanies `Maple.exe`.

## Verification

The suite exercises registration, incorrect-password and duplicate-account rejection,
immutable memberships, private chat delivery/history, restart persistence, session
rotation/revocation/expiry, anonymous voice rejection, room isolation, voice leave,
sender-ID rewriting, untrusted/wrong-host/expired certificates, and a UDP wire proxy
that corrupts encrypted packets. It checks actual captured packets for plaintext
password/text/voice markers and verifies that tampered media is rejected. Unit tests
also cover secret redaction, native credential handling, offline drafts/hit testing,
bounded text/history/membership, jitter ordering/loss/wrap, and real Opus round trips.
Tests use isolated directories and ephemeral ports, not the running user's accounts.

Relevant library references: [Quinn](https://docs.rs/quinn/latest/quinn/),
[rustls](https://docs.rs/rustls/latest/rustls/),
[Argon2](https://docs.rs/argon2/0.5.3/argon2/),
[SQLite WAL](https://www.sqlite.org/wal.html).
