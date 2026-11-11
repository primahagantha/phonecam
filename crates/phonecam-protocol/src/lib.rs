//! PhoneCam wire protocol v1 â€” the frozen contract.
//!
//! This is the Rust half of `docs/protocol.md`. It is byte-identical to the
//! TypeScript (`packages/shared-types/src/protocol.ts`), Kotlin
//! (`apps/android/.../protocol/Proto.kt`) and C++
//! (`native/core/include/phonecam/protocol.hpp`) implementations.
//!
//! **Changing anything phone-facing here requires bumping `PROTOCOL_VERSION`
//! and updating all four implementations in the same commit.** The loopback
//! channel (Â§9) is not phone-facing: both its endpoints ship in one desktop
//! build, so adding an event there must *not* bump the version. Doing so would
//! force every paired phone to update for a message it never sees.
//!
//! All integers are big-endian on the wire. All JSON is UTF-8.

pub mod framing;
pub mod messages;

pub use framing::{
    write_control_frame, write_loopback, write_server_greeting, write_server_response,
    write_video_packet, ClientHandshake, LoopbackTag, ServerGreeting, ServerHandshakeResponse,
    VideoPacketHeader,
};

pub use messages::{
    AdbDevice, AuthMode, CameraCapability, CameraFacing, CapabilitiesPayload, ChannelRole,
    CommandReply, ConnectionStats, CoreError, CoreState, CoreToDesktop, DesktopBeacon,
    DesktopToCore, DesktopToPhone, DeviceCapabilities, DeviceIdentity, DiscoveryProbe,
    DiscoveryMessage, FpsValue, HandshakeStatus, LogLevel, NearbyPhone, OutputTransform,
    PairingStatus, PhoneBeacon, PhoneState, PhoneStats, PhoneToDesktop, QrPayload, QualityMode,
    ScreenMode, StreamPhase, ThermalState, TransportKind, UsbStatus, VcamState, VcamStatus,
    VideoCodec, VideoFlags,
};

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Wire protocol version. Bumping this is a breaking change for every phone.
pub const PROTOCOL_VERSION: u8 = 1;

/// UDP discovery beacons.
pub const DISCOVERY_PORT: u16 = 48720;
/// Phone â†’ desktop control + video.
pub const SERVICE_PORT: u16 = 48721;
/// Desktop UI â‡„ desktop core, loopback only.
pub const LOOPBACK_IPC_PORT: u16 = 48723;

pub const MAX_CONTROL_JSON: usize = 1024 * 1024;
pub const MAX_VIDEO_PAYLOAD: usize = 8 * 1024 * 1024;

pub const PAIRING_TOKEN_TTL_MS: i64 = 300_000;
pub const PAIRING_TOKEN_BYTES: usize = 16;
pub const DEVICE_SECRET_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 32;
pub const BEACON_INTERVAL_MS: i64 = 1000;

pub const DEFAULT_BITRATE_720P: u32 = 4_000_000;
pub const DEFAULT_BITRATE_1080P: u32 = 8_000_000;

/// `"PCAM"` as a big-endian u32.
pub const MAGIC: u32 = 0x5043_414D;
pub const MAGIC_BYTES: [u8; 4] = [0x50, 0x43, 0x41, 0x4D];

pub const SERVER_GREETING_SIZE: usize = 40;
pub const CLIENT_HANDSHAKE_SIZE: usize = 20;
pub const SERVER_RESPONSE_SIZE: usize = 24;
pub const VIDEO_HEADER_SIZE: usize = 24;

/// Key for the unauthenticated `pairRequest` HMAC (Â§5.4).
///
/// This deliberately does **not** authenticate the device â€” it only proves the
/// requester saw the fresh `serverNonce`, which stops blind spraying. The actual
/// authorisation is the human approval plus one-time token (Â§4.3). Conflating
/// the two would be a security bug, so they stay separate.
pub const PAIR_REQUEST_KEY: &str = "phonecam-pair-request";

/// Errors from parsing the wire format.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("bad magic in frame (expected PCAM)")]
    BadMagic,
    #[error("frame too short: need {need} bytes, have {have}")]
    TooShort { need: usize, have: usize },
    #[error("frame length {len} exceeds limit {limit}")]
    Oversized { len: usize, limit: usize },
    #[error("unsupported protocol version {0}")]
    VersionMismatch(u8),
    #[error("malformed auth payload")]
    MalformedAuth,
    #[error("invalid utf-8 in frame body")]
    InvalidUtf8,
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

/// `serverNonce || clientNonce(big-endian u64)`, the bytes an HMAC is taken over.
///
/// The order is fixed by Â§4.4 and the phone's `Auth.kt`: server nonce first,
/// then the client's 8-byte monotonic counter.
pub fn hmac_message(server_nonce: &[u8], client_nonce: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(server_nonce.len() + 8);
    out.extend_from_slice(server_nonce);
    out.extend_from_slice(&client_nonce.to_be_bytes());
    out
}

fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

/// `deviceId:hexHmac` â€” auth mode 2, proving possession of the device secret.
pub fn device_secret_auth(
    device_id: &str,
    device_secret_hex: &str,
    server_nonce: &[u8],
    client_nonce: u64,
) -> Result<String> {
    let key = hex::decode(device_secret_hex).map_err(|_| ProtocolError::MalformedAuth)?;
    let mac = hmac_sha256_hex(&key, &hmac_message(server_nonce, client_nonce));
    Ok(format!("{device_id}:{mac}"))
}

/// `deviceId:request:hexHmac` â€” auth mode 3, raising a desktop approval prompt.
///
/// With no stored secret the phone signs with the fixed `PAIR_REQUEST_KEY`; the
/// server accepts either.
pub fn pair_request_auth(
    device_id: &str,
    device_secret_hex: Option<&str>,
    server_nonce: &[u8],
    client_nonce: u64,
) -> Result<String> {
    let message = hmac_message(server_nonce, client_nonce);
    let mac = match device_secret_hex {
        Some(secret) => {
            let key = hex::decode(secret).map_err(|_| ProtocolError::MalformedAuth)?;
            hmac_sha256_hex(&key, &message)
        }
        None => hmac_sha256_hex(PAIR_REQUEST_KEY.as_bytes(), &message),
    };
    Ok(format!("{device_id}:request:{mac}"))
}

/// Verifies either accepted pair-request signature. Constant-time comparison.
pub fn verify_pair_request(
    device_id: &str,
    device_secret_hex: Option<&str>,
    server_nonce: &[u8],
    client_nonce: u64,
    presented: &str,
) -> bool {
    let expected_without = pair_request_auth(device_id, None, server_nonce, client_nonce);
    let expected_with = device_secret_hex
        .and_then(|s| pair_request_auth(device_id, Some(s), server_nonce, client_nonce).ok());

    compare_constant_time(expected_without.as_deref().unwrap_or("").as_bytes(), presented.as_bytes())
        || expected_with
            .as_deref()
            .map(|e| compare_constant_time(e.as_bytes(), presented.as_bytes()))
            .unwrap_or(false)
}

/// Verifies an auth-mode-2 device-secret signature. Constant-time.
pub fn verify_device_secret(
    device_id: &str,
    device_secret_hex: &str,
    server_nonce: &[u8],
    client_nonce: u64,
    presented_mac: &str,
) -> bool {
    match device_secret_auth(device_id, device_secret_hex, server_nonce, client_nonce) {
        Ok(expected) => {
            // `presented` is the mac alone, which is what follows the colon on
            // the wire. Comparing it against the whole `deviceId:mac` string
            // fails for every phone, because the id is never in the digest.
            let expected_mac =
                expected.rsplit_once(':').map(|(_, mac)| mac).unwrap_or(&expected);
            compare_constant_time(expected_mac.as_bytes(), presented_mac.as_bytes())
        }
        Err(_) => false,
    }
}

/// Length-independent constant-time comparison.
///
/// Compares the HMAC digests, not the secrets, so an early length mismatch only
/// reveals that the input was malformed â€” which the caller already knows.
pub fn compare_constant_time(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// `sha256(secret)[0:8]` â€” the only form of a secret that may reach a log (Â§13).
pub fn secret_fingerprint(device_secret_hex: &str) -> String {
    let digest = Sha256::digest(device_secret_hex.as_bytes());
    hex::encode(digest)[..8].to_string()
}

/// Lowercase hex encoding, matching the phone's `ByteArray.toHex()`.
pub fn to_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

/// Decodes lowercase hex; `None` for odd length or non-hex input (never panics).
pub fn from_hex(value: &str) -> Option<Vec<u8>> {
    hex::decode(value).ok()
}

/// Redacts anything shaped like a 32- or 64-char lowercase hex run (PRD Â§63).
///
/// Applied to every log line the core writes, so a secret cannot leak through a
/// line that forgot to declare itself sensitive.
pub fn redact(message: &str) -> String {
    redact_runs(message)
}

fn redact_runs(message: &str) -> String {
    let bytes = message.as_bytes();
    let mut out = String::with_capacity(message.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let run = count_hex_run(bytes, i);
        // `>= 32`, not `== 32 || == 64`: a run longer than a token must be
        // swallowed whole. Matching only the exact lengths redacted the first
        // 32 chars of a 33-char run and emitted the last one, which leaks the
        // tail of a secret rather than hiding it. Over-redacting prose costs a
        // word; under-redacting a secret costs the secret.
        if run >= 32 {
            out.push_str(&"[redacted]");
            i += run;
        } else {
            // Copy one UTF-8 character, not one byte, so non-ASCII survives.
            let ch_len = message[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            out.push_str(&message[i..i + ch_len]);
            i += ch_len;
        }
    }
    out
}

fn count_hex_run(bytes: &[u8], start: usize) -> usize {
    let mut n = 0;
    while start + n < bytes.len() {
        match bytes[start + n] {
            b'0'..=b'9' | b'a'..=b'f' => n += 1,
            _ => break,
        }
    }
    n
}

/// Parses `WxH` into numeric dimensions; `None` when malformed.
pub fn parse_resolution(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.split_once('x')?;
    let w = w.parse::<u32>().ok()?;
    let h = h.parse::<u32>().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// Human label for a resolution, e.g. `1920x1080` â†’ `1080p`.
///
/// Compares by the *shorter* side, which is what makes `1920x1080` and
/// `1080x1920` the same picture: a sensor mounted sideways reports the other
/// orientation for the identical frame (see `resolution.ts`).
pub fn resolution_label(value: &str) -> String {
    match parse_resolution(value) {
        Some((w, h)) => format!("{}p", w.min(h)),
        None => value.to_string(),
    }
}

/// Splits an H.264 Annex-B access unit into NAL units, without copying.
///
/// Both the wire format and MediaCodec's output are Annex-B (start codes
/// `00 00 01` / `00 00 00 01`), so this only locates boundaries â€” it is used for
/// diagnostics and parameter-set extraction, never for conversion.
pub fn split_annex_b(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0usize;
    while i + 2 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i);
            i += 3;
        } else if i + 3 < data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            starts.push(i);
            i += 4;
        } else {
            i += 1;
        }
    }

    starts
        .windows(2)
        .map(|w| &data[w[0]..w[1]])
        .chain(starts.last().map(|&s| &data[s..]))
        .collect()
}

/// The NAL unit type, from the first byte after the start code.
pub fn nal_unit_type(nal: &[u8]) -> Option<u8> {
    // Skip the start code.
    let offset = if nal.len() >= 4 && nal[0] == 0 && nal[1] == 0 && nal[2] == 0 && nal[3] == 1 {
        4
    } else if nal.len() >= 3 && nal[0] == 0 && nal[1] == 0 && nal[2] == 1 {
        3
    } else {
        return None;
    };
    nal.get(offset).map(|b| b & 0x1f)
}

pub const NAL_SPS: u8 = 7;
pub const NAL_PPS: u8 = 8;
pub const NAL_IDR: u8 = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_round_trip() {
        let nonce = vec![0xABu8; 32];
        let bytes = write_server_greeting(HandshakeStatus::Ok, &nonce).unwrap();
        assert_eq!(bytes.len(), SERVER_GREETING_SIZE);
        let parsed = ServerGreeting::parse(&bytes).unwrap();
        assert_eq!(parsed.proto_version, PROTOCOL_VERSION);
        assert_eq!(parsed.status, HandshakeStatus::Ok);
        assert_eq!(parsed.server_nonce, nonce);
    }

    #[test]
    fn greeting_rejects_bad_magic() {
        let mut bytes = write_server_greeting(HandshakeStatus::Ok, &[0u8; 32]).unwrap();
        bytes[0] = 0x00;
        assert!(matches!(
            ServerGreeting::parse(&bytes),
            Err(ProtocolError::BadMagic)
        ));
    }

    #[test]
    fn handshake_round_trip() {
        let h = ClientHandshake {
            proto_version: 1,
            role: ChannelRole::Control,
            auth_mode: AuthMode::DeviceSecret,
            client_nonce: 0x0102_0304_0506_0708,
            auth: "android-a31f92:deadbeef".to_string(),
        };
        let bytes = h.to_bytes().unwrap();
        assert_eq!(bytes.len(), CLIENT_HANDSHAKE_SIZE + h.auth.len());
        let parsed = ClientHandshake::parse(&bytes).unwrap();
        assert_eq!(parsed.role, ChannelRole::Control);
        assert_eq!(parsed.auth_mode, AuthMode::DeviceSecret);
        assert_eq!(parsed.client_nonce, 0x0102_0304_0506_0708);
        assert_eq!(parsed.auth, h.auth);
    }

    #[test]
    fn handshake_nonce_is_big_endian() {
        let h = ClientHandshake {
            proto_version: 1,
            role: ChannelRole::Video,
            auth_mode: AuthMode::Loopback,
            client_nonce: 0x0000_0000_0000_00ff,
            auth: String::new(),
        };
        let bytes = h.to_bytes().unwrap();
        // clientNonce at offset 8, 8 bytes big-endian.
        assert_eq!(&bytes[8..16], &[0, 0, 0, 0, 0, 0, 0, 0xff]);
    }

    #[test]
    fn response_round_trip() {
        let body = serde_json::json!({"deviceId": "android-a31f92", "deviceSecret": "aa"});
        let bytes =
            write_server_response(HandshakeStatus::Ok, ChannelRole::Control, &body).unwrap();
        let parsed = ServerHandshakeResponse::parse(&bytes).unwrap();
        assert_eq!(parsed.status, HandshakeStatus::Ok);
        assert_eq!(parsed.role, ChannelRole::Control);
        assert_eq!(parsed.json, body);
    }

    #[test]
    fn video_packet_round_trip() {
        let payload = vec![0u8, 0, 0, 1, 0x65, 0x11, 0x22];
        let header = VideoPacketHeader {
            frame_id: 7,
            pts_us: 1_234_567,
            flags: VideoFlags::KEYFRAME,
            codec: VideoCodec::H264,
            payload_len: payload.len() as u32,
        };
        let bytes = write_video_packet(&header, &payload).unwrap();
        // totalLen counts the header + payload, not itself.
        assert_eq!(&bytes[0..4], &((VIDEO_HEADER_SIZE + payload.len()) as u32).to_be_bytes());
        let (parsed, body) = VideoPacketHeader::parse(&bytes).unwrap();
        assert_eq!(parsed.frame_id, 7);
        assert_eq!(parsed.pts_us, 1_234_567);
        assert_eq!(parsed.flags, VideoFlags::KEYFRAME);
        assert_eq!(body, payload);
    }

    /// The auth bytes are pinned against the Kotlin and TypeScript
    /// implementations: the same inputs must produce the same string on all
    /// four ends, or every paired phone stops authenticating.
    #[test]
    fn device_secret_auth_is_compatible() {
        let server_nonce = [0x11u8; 32];
        let secret = "0f".repeat(32);
        let auth = device_secret_auth("android-a31f92", &secret, &server_nonce, 1700000000000)
            .unwrap();
        assert!(auth.starts_with("android-a31f92:"));
        let (id, mac) = auth.split_once(':').unwrap();
        assert_eq!(id, "android-a31f92");
        assert_eq!(mac.len(), 64, "hmac is 32 bytes hex-encoded");
        assert!(mac.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn pair_request_without_secret_uses_fixed_key() {
        let server_nonce = [0x22u8; 32];
        let a = pair_request_auth("d1", None, &server_nonce, 42).unwrap();
        assert!(a.starts_with("d1:request:"));
        assert!(verify_pair_request("d1", None, &server_nonce, 42, &a));
        // A different nonce must not verify.
        assert!(!verify_pair_request("d1", None, &[0x33u8; 32], 42, &a));
    }

    #[test]
    fn pair_request_with_secret_also_verifies() {
        let server_nonce = [0x44u8; 32];
        let secret = "ab".repeat(32);
        let a = pair_request_auth("d2", Some(&secret), &server_nonce, 9).unwrap();
        assert!(verify_pair_request("d2", Some(&secret), &server_nonce, 9, &a));
        // The secret-less form must NOT verify when the phone signed with a secret.
        assert!(!verify_pair_request("d2", None, &server_nonce, 9, &a));
    }

    #[test]
    fn verify_device_secret_rejects_tampering() {
        let server_nonce = [0x55u8; 32];
        let secret = "cd".repeat(32);
        let good = device_secret_auth("d3", &secret, &server_nonce, 5).unwrap();
        let mac = good.rsplit_once(':').unwrap().1;
        assert!(verify_device_secret("d3", &secret, &server_nonce, 5, mac));
        let mut bad = mac.to_string();
        bad.pop();
        bad.push(if bad.ends_with('a') { 'b' } else { 'a' });
        assert!(!verify_device_secret("d3", &secret, &server_nonce, 5, &bad));
    }

    #[test]
    fn hmac_message_is_server_nonce_then_client_nonce() {
        let m = hmac_message(&[0xAAu8; 32], 0x0102_0304_0506_0708);
        assert_eq!(m.len(), 40);
        assert_eq!(&m[0..32], &[0xAAu8; 32][..]);
        assert_eq!(&m[32..40], &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
    }

    #[test]
    fn redaction_catches_secret_shaped_runs() {
        let secret = "a".repeat(64);
        let token = "b".repeat(32);
        let line = format!("paired device {} with token {}", secret, token);
        let out = redact(&line);
        assert!(!out.contains(&secret));
        assert!(!out.contains(&token));
        assert_eq!(out.matches("[redacted]").count(), 2);
    }

    #[test]
    fn redaction_leaves_short_hex_and_prose_alone() {
        assert_eq!(redact("frame 0x00ff00 arrived"), "frame 0x00ff00 arrived");
        assert_eq!(redact("no device"), "no device");
        // A 31-char run is shorter than any credential and must survive intact.
        let run31 = "c".repeat(31);
        assert_eq!(redact(&run31), run31);
    }

    /// A run longer than a token must be swallowed whole.
    ///
    /// Matching only the exact 32/64 lengths left the last character of a
    /// 33-char run in the clear, which is a partial leak of the very thing this
    /// function exists to remove.
    #[test]
    fn redaction_never_partially_leaks_a_long_run() {
        for len in [33, 40, 63, 64, 65, 100] {
            let run = "a".repeat(len);
            let out = redact(&run);
            assert_eq!(out, "[redacted]", "len {len} should be fully redacted");
        }
    }

    #[test]
    fn redaction_handles_a_secret_inside_a_sentence() {
        let out = redact(&format!("token={} ok", "f".repeat(32)));
        assert_eq!(out, "token=[redacted] ok");
    }

    #[test]
    fn redaction_is_utf8_safe() {
        assert_eq!(redact("kamera âœ“ 0123456789abcdef0123456789abcdef"), "kamera âœ“ [redacted]");
    }

    #[test]
    fn secret_fingerprint_is_eight_chars() {
        let fp = secret_fingerprint(&"e".repeat(64));
        assert_eq!(fp.len(), 8);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn resolution_label_uses_shorter_side() {
        assert_eq!(resolution_label("1920x1080"), "1080p");
        assert_eq!(resolution_label("1080x1920"), "1080p");
        assert_eq!(resolution_label("1280x720"), "720p");
        assert_eq!(resolution_label("auto"), "auto");
    }

    #[test]
    fn annex_b_splitting_finds_nal_units() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0, 0, 0, 1, 0x67, 0x11]); // SPS
        data.extend_from_slice(&[0, 0, 0, 1, 0x68, 0x22]); // PPS
        data.extend_from_slice(&[0, 0, 1, 0x65, 0x33]); // IDR (3-byte start code)
        let nals = split_annex_b(&data);
        assert_eq!(nals.len(), 3);
        assert_eq!(nal_unit_type(nals[0]), Some(NAL_SPS));
        assert_eq!(nal_unit_type(nals[1]), Some(NAL_PPS));
        assert_eq!(nal_unit_type(nals[2]), Some(NAL_IDR));
    }

    #[test]
    fn control_frame_is_length_prefixed() {
        let msg = serde_json::json!({"t": "start_stream"});
        let bytes = write_control_frame(&msg).unwrap();
        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        assert_eq!(len, bytes.len() - 4);
        assert_eq!(&bytes[4..], serde_json::to_vec(&msg).unwrap().as_slice());
    }

    #[test]
    fn oversized_control_frame_is_refused() {
        let big = serde_json::json!({"blob": "x".repeat(MAX_CONTROL_JSON + 10)});
        assert!(matches!(
            write_control_frame(&big),
            Err(ProtocolError::Oversized { .. })
        ));
    }
}



