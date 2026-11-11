# PhoneCam Architecture (Rust + GPUI)

This document explains how the pieces fit together and, more importantly, **why**
each boundary sits where it does â€” including the ones that were deliberately
moved from the Electron build and the ones that deliberately were not.

---

## 1. The one-sentence shape

> A phone captures and encodes; one native process decodes once and publishes to
> two places: the Windows virtual camera and its own window.

```
â”Œâ”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”   H.264 over TCP    â”Œâ”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”
â”‚ Android app  â”‚ â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â–º â”‚ phonecam (one process)           â”‚
â”‚ (Kotlin)     â”‚   LAN or USB        â”‚  transport â†’ auth â†’ decode       â”‚
â””â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”˜                     â””â”€â”€â”€â”€â”€â”€â”€â”¬â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”¬â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”˜
                                             â”‚              â”‚
                              NV12 frames    â”‚              â”‚  BGRA
                              (shared mem)   â–¼              â–¼
                                  â”Œâ”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”  â”Œâ”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”
                                  â”‚ PhoneCam Camera  â”‚  â”‚ GPUI window  â”‚
                                  â”‚ (MF DLL, C++)    â”‚  â”‚ (preview)    â”‚
                                  â””â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”˜  â””â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”˜
                                           â–²
                                    Windows Camera API
                                           â–²
                                  OBS / Chrome / Discord
```

---

## 2. What moved, and what did not

### The decode is now single

The Electron build decoded the stream **twice**: the C++ core decoded to NV12 for
the virtual camera, and the renderer decoded the phone's original H.264 again
with WebCodecs for the preview. The reasoning was sound at the time â€” re-encoding
decoded frames to reach a canvas would cost a full encode â€” but it meant two
decoders, two failure modes, and an IPC hop carrying the bitstream.

Here the decoder runs once. Its output goes to the frame bus as NV12 and to the
window as BGRA. The preview cannot disagree with the camera because it is the
same decoded picture, not a second decode of the same bitstream.

### The decoder itself changed

The C++ core did not use the H.264 decoder MFT directly. It muxed each group of
pictures into a fragmented MP4 on disk, then read it back through a Media
Foundation Source Reader, because the direct `IMFTransform` path produced **zero
frames** on some machines (roughly 35 configurations were tried). That worked,
and cost a temp-file write plus a read per group â€” about 24 ms per batch, the
largest single latency term in the pipeline.

`phonecam-media` uses openh264 instead: in process, no container, no temp file,
no second copy, and independent of which MFTs a machine has installed.

### The virtual camera is still C++, on purpose

Windows instantiates software camera sources **itself**, inside the Camera Frame
Server process (`svchost.exe -k Camera`, as `NT AUTHORITY\LocalService`). The DLL
is loaded there, not here, so it cannot share this process's heap and cannot be
replaced by a Rust binary. It stays `PhoneCamVirtualCamera.dll` from the original
tree, unmodified.

That makes the frame bus a **second, independent wire contract**, and the one
place where a mistake is silent: if the writer and the reader disagree, the core
streams, decodes and reports healthy frames while the camera shows
`DEVICE NOT CONNECTED` forever. `phonecam-framebus` therefore pins every byte:

- the 68-byte packed header, with a `const` assertion on its size and a test on
  every field offset;
- `magic = 0x5043414D`, `layout_version = 2`, payload at offset 68;
- a **grow-only** file, because Windows refuses to truncate a file another
  process has mapped â€” which is exactly the state during a resolution step down;
- `capacity`, not the file size, bounds a read;
- `readers` is written **only** by readers, and never reset by the writer.

---

## 3. The three channels

| Channel | Port | Between | Framing |
| --- | --- | --- | --- |
| Discovery | UDP 48720 | phone â‡„ desktop | one JSON datagram per packet |
| Service | TCP 48721 | phone â†’ desktop | `u32`-length JSON, and `u32 totalLen` + 24-byte header for video |
| Frame bus | a file | core â†’ camera DLL | 68-byte header + seqlock + NV12 |

The loopback IPC channel the Electron build needed (TCP 48723) is **gone**: the
UI and the core are the same process, so there is nothing to frame.

---

## 4. Authentication

The rule is that `IP + fixed password` is not acceptable. What satisfies it:

1. **First contact** is a one-time token from the QR code, or a request a human
   approves on the PC. The token is 128 bits from a CSPRNG, expires in five
   minutes, and is consumed on first use.
2. **Afterwards** the phone holds a 256-bit device secret and proves possession
   with `HMAC-SHA256(secret, serverNonce || clientNonce)`.
3. **Replay** is defeated by requiring a strictly increasing client nonce per
   device; the phone persists its counter, so a restart cannot replay.

Two subtleties are load-bearing:

**`busy` must not spend the token.** The handshake peeks at the token *without
consuming it* before answering `busy`, because `busy` means "wait and retry" and
the retry needs the credential. Consuming it there turns a temporary refusal into
a permanent one.

**The `pairRequest` HMAC is not authentication.** It proves only that the
requester saw the fresh server nonce, which stops blind spraying. The actual
authorisation is the human approval plus the one-time token. Conflating the two
would be a security bug, so they are separate in the code and named separately.

---

## 5. The session, and what a phase means

`state.phase` is a claim about whether a picture is flowing, so it is set only
when that is true.

| Phase | Means |
| --- | --- |
| `idle` | No phone. |
| `connecting` | A phone is connected and the stream is being set up. |
| `connected` | The phone is connected and the transport is up, but nothing is streaming. |
| `streaming` | A decoded frame has been published for the current stream. |
| `reconnecting` | The phone is gone and the core is waiting for it. |
| `error` | The last attempt failed; the error says why. |

A video socket being accepted is **not** `streaming`: the socket says a channel
exists, a decoded frame says there is a picture. The phone's own `streaming` flag
is **not** sufficient either â€” it is set from the phone's encoder starting,
before this PC has a socket or has decoded anything, so it is evidence about the
phone's encoder rather than about a picture reaching this PC. It cannot promote
the phase; only a decoded frame can.

A `streaming: false` from the phone *does* demote, because it means the phone has
stopped encoding and no claim of live video can hold.

A reconfigure opens a second video socket while the first picture is still on
screen, so opening one never demotes a live stream. Doing so made a working
stream look like it had failed.

---

## 6. Adaptive quality

The ladder lives in the core, not the phone, because the evidence is split across
both ends: the phone owns the encoder and the thermal sensor, the desktop owns
the round trip and the decode path. A single decision has to be made where both
are visible.

```
1080p60 -> 1080p30 -> 720p30 -> 480p30
```

Quick to drop (10 s dwell), slow to climb (30 s of clean stats). Critical thermal
skips the dwell entirely: the platform is about to clamp the encoder anyway, so a
controlled step keeps the stream alive instead of letting it stall.

Two rules that look like details and are not:

- **An unmeasured round trip is not a perfect link.** `rttUs == 0` means "never
  measured", so it counts as neither clean nor struggling. Reading it as clean
  made an unmeasured stream climb the ladder.
- **The announcement follows the phone's confirmation, not the request.** The
  controller's `set_video` is a request; the phone substitutes rather than fails
  when a rung is unavailable, so the change is announced only once the phone's own
  state reports the new rung. Announcing at the request would name a rung the
  phone is not using.

The ceiling is the user's own capture setting, not the camera's maximum. Without
that, a phone asked for 720p climbed back to 1080p within thirty seconds of clean
stream and silently overrode the choice â€” measured on a Redmi Note 11, that took
the app from 25% to 38% CPU and the camera HAL from 109% to 129%.

---

## 7. The UI

Three regions, each with one job:

- **status bar** â€” one sentence: is the picture flowing.
- **stage** â€” the picture, or the pairing code, or the next step.
- **rail** â€” four tabs in the order the job happens: Setup, Capture, Publish,
  Diagnostics.

The preview is never a tab. It is always visible, because an answer you have to
navigate to is not an answer.

### The preview surface

The picture is drawn from one image handle that lives for the lifetime of the
window, and the window pulls it on an animation frame rather than being pushed a
new texture per decoded frame. This is the GPUI equivalent of the shipped
"exactly one canvas, never remounted" rule, and it exists for the same reason: a
preview that swaps its target paints into a detached surface and shows nothing
while the core decodes happily.

One conversion detail is easy to get wrong and invisible in a grey test: gpui's
`RenderImage` expects **BGRA, premultiplied**. Emitting RGBA produces a picture
with red and blue swapped, which reads as a broken camera rather than a broken
conversion. `DecodedFrame::write_bgra` is pinned by a coloured test.

### The tray

GPUI 0.2.2 ships a status item on macOS only, so `phonecam-app` builds the
Windows tray directly on `Shell_NotifyIcon`. It exists for two reasons that still
hold: closing the window must keep the virtual camera publishing, because other
apps hold the device; and the camera toggle has to be reachable without
restoring the window.

---

## 8. Recovery

The single most important reliability property is that **the camera device must
not disappear**. When the link drops:

```
frames stop
   â†“ (immediately)
virtual camera keeps publishing the last frame
   â†“ (after 3 s)
core publishes a black NV12 frame with state = Disconnected
   â†“
phone re-dials on its own, with exponential backoff
```

The phone owns reconnection. The core keeps publishing the placeholder and
resumes the moment a phone arrives, which is what stops OBS and browsers ever
losing the device. The state is expressed **in the frame**, so a consumer sees
`PHONECAM / DEVICE NOT CONNECTED` rather than a frozen image.

Publishing black pixels alongside the state â€” rather than only flipping the state
field â€” is deliberate: a reader that ignores the header would otherwise keep
showing the last frame of a phone that is gone.

---

## 9. USB is not a second code path

The phone always dials `127.0.0.1:48721`. On Wi-Fi that address is this PC; on a
cable, `adb reverse tcp:48721 tcp:48721` makes it this PC through the phone's own
loopback. That is the entire USB implementation, and it exists so the most
failure-prone part of the product — the connection handshake — has exactly one
code path instead of two.

Two facts are reported separately and must never be collapsed:

| Field | Means |
| --- | --- |
| `usb.adbAvailable` | the platform tools were found and can be run |
| `usb.available` | a phone is actually on the cable |

The shipped app read the second as the first, so a PC on Wi-Fi with no cable
plugged in told the user to install a tool it already had. There is a third
state that matters just as much: **before `locate` has run at all**, neither is
known, and the honest answer is "not checked yet" rather than "missing". The
runtime tracks `adb_checked` for exactly that reason, and the notice only fires
on a recorded failure.

## 10. The tray

GPUI 0.2.2 ships a status item on macOS only, so the Windows tray is built on
`Shell_NotifyIcon` with a **message-only window** the app owns.

Owning that window is not incidental. The icon posts its clicks to a window
handle, and the GPUI window is created and destroyed by the framework — and
hiding it on close is precisely the case where a click still has to arrive. A
tray attached to the visible window would go deaf the moment it was most needed.

Closing hides rather than quits, because the virtual camera keeps publishing and
other apps hold the device: a closed window that quit would silently kill every
consumer's camera. That means the close must actually *hide* — a close hook that
merely refuses leaves an X button that does nothing, which is a worse lie than
having no close-to-tray. `ShowWindow(SW_HIDE)` does it, because GPUI exposes
`hide` on its platform window but not on `Window`.

Quit lives in the tray, which is the only place it can be deliberate.
## 11. Logging

Never log pairing tokens, device secrets, or HMAC values. Log
`sha256(secret)[0:8]` when a correlator is needed. Never log video payload bytes;
log lengths and timestamps only.

`redact()` is applied to every line and swallows any run of 32 or more lowercase
hex characters, because a line that forgot to declare itself sensitive must still
not leak. Over-redacting prose costs a word; under-redacting a secret costs the
secret â€” and matching only the exact 32/64 lengths once left the tail of a
33-character run in the clear.

---

## 12. Running beside the original

`PHONECAM_PORT` overrides the service port. That is not a test convenience: on
this machine the installed Electron build holds 48721 through its own
`phonecam-core.exe`, so the Rust build could not start at all and every failure
read as "another copy of this app is running", which was not what happened. The
port is published on the runtime so the QR payload and the beacon advertise the
same number the listener bound, instead of assuming the protocol default.

Discovery still uses UDP 48720 and will fail while the original core holds it;
the app says so in its log and pairs by QR code instead, which is the documented
fallback rather than a silent degradation.
## 13. Platform roadmap

Only the virtual camera layer is platform-specific.

| Platform | Mechanism | Notes |
| --- | --- | --- |
| Windows | Media Foundation virtual camera (`MFCreateVirtualCamera`) | needs build 22621+ |
| macOS 12.3+ | Core Media I/O camera extension | would replace the DLL |
| Linux | `v4l2loopback` | needs a distro/kernel matrix |

Transport, decode and the frame bus are already portable; the frame bus would
become a shared memory segment instead of a file.


