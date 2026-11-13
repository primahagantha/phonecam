---
name: phonecam-protocol
description: Deep guide for binary framing, HMAC-SHA256 crypto, JSON payloads, replay guard, and diagnostics.
---

# PhoneCam Wire Protocol & Cryptography Skill Guide

This skill guide provides an exhaustive specification of the frozen PhoneCam Wire Protocol v1, detailing binary packet framing, cryptographic handshake authentication, replay protection mechanisms, JSON message models, and network debugging procedures.

---

## 1. Protocol Architecture & Channel Topology

The PhoneCam protocol contract is strictly frozen (`PROTOCOL_VERSION = 1`). It is byte-identical across all implementations:
- **Rust Desktop Core**: `crates/phonecam-protocol`
- **Android Kotlin**: `apps/android/.../protocol/Proto.kt`
- **C++ Engine**: `native/core/include/phonecam/protocol.hpp`
- **TypeScript**: `packages/shared-types/src/protocol.ts`

```
┌────────────────────────────────────────────────────────┐
│                      Android Client                    │
└──────┬───────────────────┬───────────────────┬─────────┘
       │ UDP Port 48720    │ TCP Port 48721    │ TCP Port 48721
       │ (Datagram)        │ (Stream / JSON)   │ (Framed Binary)
       ▼                   ▼                   ▼
┌──────────────┐    ┌──────────────┐    ┌────────────────┐
│  Discovery   │    │   Control    │    │     Video      │
│   Channel    │    │   Channel    │    │    Channel     │
└──────────────┘    └──────────────┘    └────────────────┘
```

### Channel Summary
| Channel | Transport | Port | Framing Structure | Purpose |
| :--- | :--- | :--- | :--- | :--- |
| **Discovery** | UDP | `48720` | Single JSON payload per UDP packet | Local network peer announcement |
| **Service (Control)** | TCP | `48721` | Big-endian `u32` length + UTF-8 JSON | State exchange, commands, telemetry |
| **Service (Video)** | TCP | `48721` | Big-endian `u32` totalLen + 24B Header + Payload | H.264 Annex-B video stream |

> **Endianness Rule**: All integers on the wire are encoded in **Big-Endian** (network byte order).

---

## 2. Binary Framing Specification

### 2.1 Protocol Constants
- `MAGIC`: `0x5043414D` (`"PCAM"` in ASCII, `[0x50, 0x43, 0x41, 0x4D]`).
- `PROTOCOL_VERSION`: `1`.
- `MAX_CONTROL_JSON`: `1,048,576` bytes (1 MB).
- `MAX_VIDEO_PAYLOAD`: `8,388,608` bytes (8 MB).

---

### 2.2 Server Greeting (40 Bytes)
Immediately upon accepting a TCP connection on port `48721`, the desktop transmits the Server Greeting before reading any bytes from the client:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Magic (0x5043414D)                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| Proto Version |     Status    |            Reserved           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
+                                                               +
|                    Server Nonce (32 Bytes)                    |
+                 CSPRNG Cryptographic Randomness               +
|                                                               |
+                                                               +
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- `Magic`: 4 bytes `0x50 0x43 0x41 0x4D`.
- `Proto Version`: 1 byte (`0x01`).
- `Status`: 1 byte (`0 = Ok`, `2 = Busy`).
- `Reserved`: 2 bytes (`0x0000`).
- `Server Nonce`: 32 bytes generated from system CSPRNG (`rand::thread_rng()`).

---

### 2.3 Client Handshake (20 Bytes + Variable Auth String)
In response, the client transmits its handshake:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Magic (0x5043414D)                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| Proto Version |  Channel Role |   Auth Mode   |     Flags     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
+                   Client Nonce (8 Bytes, u64)                 +
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|          Auth Length          |            Reserved           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Auth String (Variable)                    |
|                        (UTF-8 Encoded)                        |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- `Channel Role`: `1 = Control`, `2 = Video`.
- `Auth Mode`:
  - `1 = PairToken`: First contact via 128-bit pairing token.
  - `2 = DeviceSecret`: Regular session via HMAC proof of shared secret.
  - `3 = PairRequest`: Soliciting manual approval on desktop UI.
- `Client Nonce`: Strictly monotonic 64-bit integer persisted in phone storage.
- `Auth Length`: Big-endian `u16` length of following auth string.
- `Auth String`:
  - Mode 1: `<32-char-hex-token>`
  - Mode 2: `<deviceId>:<64-char-hex-hmac>`
  - Mode 3: `<deviceId>:request:<64-char-hex-hmac>`

---

### 2.4 Server Handshake Response (24 Bytes + JSON Body)
```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Magic (0x5043414D)                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
| Proto Version |     Status    |  Channel Role |     Flags     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Reserved (8 Bytes)                      |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                          JSON Length                          |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Reserved (4 Bytes)                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                      JSON Payload (UTF-8)                     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- `Status`: `0 = Ok`, `1 = BadAuth`, `2 = Busy`, `3 = VersionMismatch`.
- `Role`: Echo of the client's role.
- `JSON Length`: Big-endian `u32` byte count of following JSON payload.
- `JSON Payload`: On successful Auth Mode 1 pairing, returns:
  ```json
  {
    "deviceId": "phone-a1b2c3d4",
    "deviceSecret": "9f83... (64 hex characters)",
    "serverId": "desktop-x9y8z7",
    "name": "User Desktop"
  }
  ```

---

### 2.5 Control Channel Frame
All subsequent control messages use length-prefixed JSON:
```
[ 4-byte u32 Big-Endian Length ] [ UTF-8 JSON Payload ]
```

---

### 2.6 Video Packet Framing (24-Byte Header + NAL Payload)
Every video frame packet begins with a 4-byte total length prefix (`total_len = 24 + payload_len`), followed by the 24-byte binary video header and raw H.264 Annex-B NAL units:

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                 Total Length Prefix (u32, BE)                 |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Frame ID (u32, Monotonic)                 |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
+           Presentation Timestamp in Microseconds (i64)        +
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|  Video Flags  |     Codec     |           Reserved            |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                          Payload Length                       |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Reserved (4 Bytes)                      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                  H.264 Annex-B Payload Bytes                  |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

- `Video Flags`:
  - Bit 0 (`0x01`): `KEYFRAME` (IDR slice).
  - Bit 1 (`0x02`): `CONFIG` (Contains SPS / PPS parameter sets).
  - Bit 2 (`0x04`): `DISCONTINUITY` (Encoder reconfigured / resolution changed).
- `Codec`: `1 = H.264`.

---

## 3. Cryptographic Authentication & Replay Guard

### 3.1 HMAC-SHA256 Authentication Computation
In Auth Mode 2, authentication proves knowledge of the 256-bit `deviceSecret` without exposing the secret across the wire:

```
message = serverNonce (32 bytes) || clientNonce (8 bytes, big-endian u64)
digest  = HMAC-SHA256(key = deviceSecret, message = message)
auth    = format!("{deviceId}:{hex_encode(digest)}")
```

### 3.2 Monotonic Replay Protection
- An attacker recording a past handshake cannot replay it because the server issues a unique 32-byte CSPRNG `serverNonce` on every connection.
- A compromised intermediate network proxy cannot replay past frames because the server tracks `last_accepted_client_nonce[deviceId]`.
- If `client_nonce <= last_accepted_nonce`, the server immediately rejects the handshake with `HandshakeStatus::BadAuth`.

### 3.3 Timing Attack Immunity
Digest verification must strictly execute in constant time:
```rust
pub fn compare_constant_time(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
```

### 3.4 Unauthenticated Anti-Spraying (`pairRequest`)
When an unpaired device requests authorization (Auth Mode 3), it signs using the public constant key `phonecam-pair-request`:
```rust
pub const PAIR_REQUEST_KEY: &str = "phonecam-pair-request";
```
This guarantees the requester observed the fresh `serverNonce`, preventing blind amplification attacks while leaving the authorization decision to explicit human approval on the desktop UI.

---

## 4. JSON Control Message Schemas

### 4.1 Phone -> Desktop: `sessionState`
Sent by the phone whenever streaming parameters change:
```json
{
  "type": "sessionState",
  "streaming": true,
  "width": 1920,
  "height": 1080,
  "fps": 60,
  "bitrate": 8000000,
  "facing": "back",
  "battery": 88,
  "thermal": "nominal"
}
```

### 4.2 Desktop -> Phone: `setVideo`
Dispatched by adaptive bitrate ladder or manual UI settings:
```json
{
  "type": "setVideo",
  "width": 1280,
  "height": 720,
  "fps": 30,
  "bitrate": 4000000
}
```

### 4.3 Ping / Pong Round-Trip Telemetry
Calculates round-trip network latency (`rttUs`):
- Desktop sends `{"type": "ping", "timestamp": 1731512400123}`
- Phone immediately responds with `{"type": "pong", "timestamp": 1731512400123}`

---

## 5. Network Sniffing & Wire Diagnostics

### 5.1 Verifying TCP Handshake via Raw Bytes
When inspecting Wireshark or socket dumps:
```
50 43 41 4D  # Magic: "PCAM"
01           # Proto Version: 1
00           # Status: 0 (Ok)
00 00        # Reserved
4A E1 89 ... # 32-byte Server Nonce
```

### 5.2 Common Handshake Errors
| Error Code / Symptom | Root Cause | Resolution |
| :--- | :--- | :--- |
| `Status 1 (BadAuth)` | Stale `clientNonce` or mismatched secret | Clear paired device in settings and re-pair with QR |
| `Status 2 (Busy)` | Desktop already in active session | Verify no other phone is connected or close orphan session |
| `Status 3 (VersionMismatch)` | Client and desktop versions incompatible | Upgrade both desktop and Android clients |
| `BadMagic` | Connection to wrong port or protocol desync | Verify port is 48721 and peer sends "PCAM" prefix |
