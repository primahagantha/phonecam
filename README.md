# PhoneCam (Rust + GPUI)

Turn an old Android phone into a real webcam. This is the native rewrite of the
Electron desktop app: same wire protocol, same virtual camera, one process.

```
Android phone â”€â”€H.264 over LAN or USBâ”€â”€â–º PhoneCam (one process) â”€â”€â–º PhoneCam Camera
                                          â”œ transport + auth          (MF DLL, C++)
                                          â”œ decode (openh264)         â–²
                                          â”œ frame bus â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”˜
                                          â”” GPUI window (preview)
```

The Android app is unchanged. The virtual camera DLL is unchanged. What changed
is everything between them.

---

## What this rewrite removes

| | Electron build | This build |
| --- | --- | --- |
| Processes | 3 (renderer, main, C++ core) | **1** |
| Decodes per stream | **2** (WebCodecs for preview, MF for camera) | **1** |
| Preview path | H.264 â†’ loopback socket â†’ WebCodecs â†’ canvas | decoded frame â†’ texture |
| Decoder | MF mux-to-MP4-then-read-back (~24 ms/batch) | openh264 in-process |
| Runtime to install | Chromium (~150 MB) | a single exe |
| Idle RAM | ~180 MB | ~40 MB |

The C++ core muxed every group of pictures into a fragmented MP4 on disk and read
it back through a Media Foundation Source Reader, because the bare H.264 decoder
MFT produced zero frames on some machines. That cost a temp-file write plus a
read per batch and was the largest latency term in the pipeline. openh264
decodes in process with no container and no second copy, on any machine,
regardless of which MFTs happen to be installed.

### What deliberately did not change

- **The wire protocol is frozen.** `PROTOCOL_VERSION = 1`, byte-identical to
  `docs/protocol.md` in the original tree. Every existing paired phone still
  works, and the crypto is checked against the Kotlin implementation's inputs.
- **The desktop is still the server.** The phone dials, so LAN and USB share one
  code path (`adb reverse` makes USB look like loopback).
- **The virtual camera is still a C++ Media Foundation DLL.** Windows loads
  software camera sources into the Camera Frame Server process
  (`svchost.exe -k Camera`, running as `LocalService`), so a Rust binary cannot
  take its place. The Rust core writes the same
  `C:\ProgramData\PhoneCam\framebuffer-v2.bin` with the same 68-byte header.
- **The frame bus layout is byte-identical**, including the seqlock and the
  rule that `readers` is written only by readers.

---

## Crates

| Crate | Owns |
| --- | --- |
| `phonecam-protocol` | The frozen wire contract: framing, messages, HMAC auth, redaction |
| `phonecam-framebus` | The 68-byte shared-memory header, the seqlock writer, NV12 scaling |
| `phonecam-media` | H.264 decode (openh264), SPS parsing, NV12 and BGRA conversion |
| `phonecam-core` | Transport, pairing, session/handshake, the adaptive ladder |
| `phonecam-ui` | Design tokens, the pure display rules, every panel, GPUI rendering |
| `phonecam-app` | The binary: settings, tray, and the one process that runs it all |

`phonecam-ui` depends on `phonecam-core` only for the protocol types, so the
window never touches a socket.

---

## Building

Requires Rust 1.85+ and, for the virtual camera, Visual Studio 2022 Build Tools
with the Windows SDK.

```powershell
cargo build --release            # -> target/release/phonecam.exe
cargo test --workspace           # 212 tests
```

The virtual camera DLL is built separately from the original C++ tree
(`native/virtual-camera`) and ships beside the exe; it is not rebuilt here.

To make the camera appear in Windows (once, from an **administrator** prompt):

```powershell
phonecam-vcam-register.exe --register
```

Requires Windows 11 build 22621 or newer: `MFCreateVirtualCamera` does not exist
earlier.

---

## Kept in step with the Electron build

The original app at `D:\production\phonecam` was still being fixed while this
port was written. Six of its fixes are behavioural, and each was a real defect
that this tree had reproduced. They are ported, not deferred:

| Upstream fix | What this tree does |
| --- | --- |
| `fix(core): one quality change at a time` | `AdaptiveQuality::pending` holds the ladder until the phone reports the rung it was asked for. Without it one period of trouble walked two rungs, because the statistics in between still described the old stream. |
| `fix(vcam): verify registration instead of assuming it` | `vcam::Registrar::status()` runs `--status`, which exits non-zero unless the camera is actually enumerable, and `registered` reports that rather than a hardcoded `true`. |
| `fix(desktop): write a desktop.log` | `settings::DesktopLog` appends a dated line per event beside the settings, so "it froze" can be dated. |
| `fix(core): let a phone re-pair when its secret no longer matches` | Already matched: `verify_pair_request` accepts a signature from **either** key, because the human approval is what authorises a pairing. |
| `fix(core): resume the stream when the phone comes back` | Not ported. This tree has no automatic resume: the phone reconnects and the desktop asks again, which is the same observable behaviour the original had before that fix, and it is the one item below that still needs doing. |
| `fix(android): encode at a constant bitrate` | Android-side; nothing to port. |

One gap the port had and this pass closed: **`want_stream` was written and never
read.** The Start button logged "start requested" and the phone was never told
anything, because nothing drained the flag. `Runtime::start_stream` now sends the
configuration batch and then `start_stream`, in that order, and
`pressing_start_asks_the_phone_to_stream` fails without it.
## Verified against a real phone

Not just an emulating client: a **Xiaomi 2201117TG** (Redmi Note 11) over Wi-Fi
on `192.168.100.16`, running `com.phonecam.camera.debug`.

    PAIRED: Android phone            transport : Lan
    frame size   : 1280x720          stats.fps    : 29.8
    frames drawn : 59                stats.bitrate: 519474
    phone actual : 1280x720 @ 30 fps stats.quality: excellent

The frame bus was read while that stream was live:

    magic      = 0x5043414d   (PCAM)      state      = 1  (Live)
    layout     = 2                        frameIndex = 314
    width      = 1920  height = 1080      seq        = 628  (even, so no write in flight)

That is the contract the C++ DLL reads: right magic, right layout version, right
size, a live state, a frame index that advances.

Three defects this test found that no emulated client had:

1. **Paired devices were never persisted.** The C++ core keeps them in
   `%APPDATA%\PhoneCam\devices.json`; this tree kept them in memory only, so
   every restart answered a reconnecting phone "unknown device" and demanded a
   new QR scan. Now loaded and saved, in **the original's format** — an earlier
   attempt here wrote a bare array with Rust field names and failed to parse the
   real file at all.
2. **The statistics were never computed.** `stats_window_start` returned the
   current time, so the elapsed check was `now - now >= 1s` — never true. The
   stream ran and every rate read zero, which is exactly the "reads as a
   measurement that was never taken" failure the design rules exist to prevent.
   `StatsWindow` now exposes `elapsed_us`.
3. **Start never reached the phone.** Already fixed in the previous pass, and
   this session confirmed it against hardware: the phone reports
   `1280x720 @ 30 fps, streaming: true`.

One caveat: the installed Electron app at `C:\Program Files\PhoneCam` keeps
restarting its own `phonecam-core.exe` on 48721 and 48720, so this test ran on
48791 via `PHONECAM_PORT`, with the phone dialling the PC's LAN address
directly. Discovery (UDP 48720) was unavailable for the same reason, and the app
said so rather than degrading silently.
## What is verified, and what is not

Verified by test (232 passing, `cargo test --workspace`):

- **Wire compatibility.** A client that speaks the phone's side of the protocol
  pairs by QR token, reconnects with its device secret, is refused on a replayed
  nonce, is refused a wrong secret, raises an approval prompt, and has video
  packets survive the socket byte for byte.
- **The running app.** `crates/phonecam-app/tests/end_to_end.rs` drives the real
  `Runtime` — real listener, real handshake, real decoder — over real sockets: it
  accepts a pairing, stores the device, greets the phone, receives capabilities,
  refuses a second phone while one holds the session, survives a garbage video
  channel, keeps a quiet control channel open, and delivers a command queued from
  another thread.
- **The frame bus** is written at the configured output size, with the 68-byte
  header pinned field by field.
- **The design rules**, each of which encodes a defect that shipped.

Also verified against the running binary, not just in tests:

- **The tray installs.** `tray icon installed` appears in the log at startup, on
  a message-only window the app owns, so a click still arrives after the visible
  window has been hidden.
- **Close-to-tray hides rather than refuses.** A `WM_CLOSE` leaves the process
  alive, the window not visible, and port 48721 still accepting. Hiding is done
  through `ShowWindow(SW_HIDE)` because GPUI exposes `hide` on its platform
  window but not on `Window`; a close hook that merely returned `false` left an X
  button that did nothing, which is worse than having no close-to-tray.
- **adb is located** at `ANDROID_HOME/platform-tools/adb.exe` and reported, with
  `adbAvailable` kept separate from "a phone is on the cable".

Not verified here, and worth knowing:

- **A real phone, on this exact build, is now confirmed** — see above. What is
  still missing is a **real consumer of the virtual camera**: no OBS or browser
  was opened to confirm the DLL turns those frames into a picture.
- **The virtual camera DLL.** It is the original C++ binary and was not rebuilt;
  the Rust side writes the buffer it reads, but the two were not exercised
  together on this machine.
- **The tray menu's click handling.** The icon installs and the menu is built
  from state, but simulating a right-click on a notification icon needs input
  injection that this session cannot do. The command path it feeds is tested.
- **Screenshots.** This session's capture path returns a blank surface for a
  DirectX-composited window, so the UI is verified by construction (it renders,
  the fonts resolve, the tokens pass contrast checks) rather than by eye.
## Requirements

- **Windows 11 build 22621+** for the virtual camera.
- **Android 8.0 (API 26)+** with an H.264 encoder, running the original
  PhoneCam app.

---

## The design rules, and why they exist

The window is an instrument panel, not a dashboard. It answers one question â€”
"is the picture flowing?" â€” and offers one action at a time. Three rules carry
most of that, and each replaced a specific failure:

**One status headline.** The old strip showed "Camera ready" next to "No phone".
Both were true, of *different subsystems*, and the result was a screen that
argued with itself. The priority order now lives in one tested function
(`theme::rules::summarise_status`): service unreachable > error > no phone >
connecting > reconnecting > streaming > ready.

**One accent, and it means "act", not "healthy".** `#3ba3ff` marks what is
active, selected or actionable. Green is reserved for health and appears only
when a live picture is actually flowing â€” a registered virtual camera with
nothing behind it is not a success, and colouring it green made the loudest
element on screen mean the least.

**One action per state**, in the place the user is already looking. The preview
stage owns start and pairing, because that is where the code appears and where
the eye already is. Every other surface describes state.

Two consequences look like omissions and are deliberate: the pairing QR renders
at 264px in the stage rather than as a sidebar thumbnail, because it is the only
asset a second device has to *read*; and the output size is stated only once a
phone is present, because on an unpaired screen it asserted a stream that did
not exist.

### Rules the tests enforce

The pure rules are unit tested because each one encodes a defect that shipped:

- `summarise_status` â€” one headline, priority ordered.
- `usb_readiness` â€” `adbAvailable` and `available` are two facts; absence of
  evidence is `Unknown`, never "tools missing".
- `vcam_chip_label` â€” a registered-but-off camera is "Off", not "Not installed".
- `throughput_readout` â€” a rate is a measurement only while streaming, and
  `latencyMs: -1` is "not measured", never minus one millisecond.
- `compare_resolution` â€” compares the *label*, so a sideways sensor reporting
  `1080x1920` for a request of `1920x1080` is not a downgrade.
- `reported_device_facts` â€” a field the phone did not report is absent, never
  blank and never "Android 0".
- `stale_notice_keys` â€” a notice whose fact is over is retracted.
- `can_start_stream` â€” a 2500 ms cooldown covers the phase lag, so two starts
  cannot race into two video dials.
- Contrast â€” every text pair is checked against the 4.5:1 floor at test time, so
  a palette edit cannot silently fail AA.

---

## Repository

```
crates/phonecam-protocol/   The frozen wire contract
crates/phonecam-framebus/   Shared memory, seqlock, NV12
crates/phonecam-media/      H.264 decode, SPS, pixel conversion
crates/phonecam-core/       Transport, pairing, session, adaptive quality
crates/phonecam-ui/         Design system, rules, panels, GPUI
crates/phonecam-app/        The binary
docs/architecture.md        Why the boundaries sit where they do
```

---

## Privacy

Video is captured, encoded, transported and decoded entirely on your own
devices. There is no account, no cloud service, and no internet dependency for
streaming. Pairing uses a one-time token exchanged by QR code, and every later
connection is authenticated with a device secret over a fresh nonce. Logs never
contain tokens, secrets or video data â€” the redactor swallows any 32- or 64-char
lowercase hex run, because a line that forgets to declare itself sensitive must
still not leak.





