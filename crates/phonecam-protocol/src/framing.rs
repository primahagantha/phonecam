//! Byte-level framing for all three channels (docs/protocol.md §5–§7, §9).
//!
//! Every struct here is `#[repr]`-free on purpose: the wire format is defined by
//! explicit offsets rather than by Rust's layout rules, so adding a field to a
//! struct can never silently change what goes on the socket.

use crate::{
    AuthMode, ChannelRole, HandshakeStatus, ProtocolError, Result, VideoCodec, VideoFlags,
    CLIENT_HANDSHAKE_SIZE, MAGIC, MAGIC_BYTES, MAX_CONTROL_JSON, MAX_VIDEO_PAYLOAD,
    PROTOCOL_VERSION, SERVER_GREETING_SIZE, SERVER_RESPONSE_SIZE, VIDEO_HEADER_SIZE,
};

use serde::Serialize;
use serde_json::Value;

/* ------------------------------------------------------------------ */
/* Server greeting (§5.1) — 40 bytes, sent before reading anything     */
/* ------------------------------------------------------------------ */

/// The desktop's first bytes on `accept()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerGreeting {
    pub proto_version: u8,
    pub status: HandshakeStatus,
    /// 32 bytes from a CSPRNG.
    pub server_nonce: Vec<u8>,
}

/// Writes the 40-byte greeting: `magic, version, status, u16 reserved, nonce`.
pub fn write_server_greeting(status: HandshakeStatus, server_nonce: &[u8]) -> Result<Vec<u8>> {
    if server_nonce.len() != 32 {
        return Err(ProtocolError::TooShort {
            need: 32,
            have: server_nonce.len(),
        });
    }
    let mut buf = Vec::with_capacity(SERVER_GREETING_SIZE);
    buf.extend_from_slice(&MAGIC_BYTES);
    buf.push(PROTOCOL_VERSION);
    buf.push(status as u8);
    buf.extend_from_slice(&0u16.to_be_bytes()); // reserved
    buf.extend_from_slice(server_nonce);
    debug_assert_eq!(buf.len(), SERVER_GREETING_SIZE);
    Ok(buf)
}

impl ServerGreeting {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < SERVER_GREETING_SIZE {
            return Err(ProtocolError::TooShort {
                need: SERVER_GREETING_SIZE,
                have: bytes.len(),
            });
        }
        require_magic(bytes)?;
        Ok(Self {
            proto_version: bytes[4],
            status: HandshakeStatus::from_wire(bytes[5]),
            server_nonce: bytes[8..8 + 32].to_vec(),
        })
    }
}

/* ------------------------------------------------------------------ */
/* Client handshake (§5.2) — 20 bytes + auth                           */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientHandshake {
    pub proto_version: u8,
    pub role: ChannelRole,
    pub auth_mode: AuthMode,
    /// Monotonic u64; the server rejects a value <= the last accepted one.
    pub client_nonce: u64,
    /// ASCII, format depends on `auth_mode`.
    pub auth: String,
}

impl ClientHandshake {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let auth = self.auth.as_bytes();
        if auth.len() > u16::MAX as usize {
            return Err(ProtocolError::Oversized {
                len: auth.len(),
                limit: u16::MAX as usize,
            });
        }
        let mut buf = Vec::with_capacity(CLIENT_HANDSHAKE_SIZE + auth.len());
        buf.extend_from_slice(&MAGIC_BYTES);
        buf.push(self.proto_version);
        buf.push(self.role as u8);
        buf.push(self.auth_mode as u8);
        buf.push(0); // flags
        buf.extend_from_slice(&self.client_nonce.to_be_bytes());
        buf.extend_from_slice(&(auth.len() as u16).to_be_bytes());
        buf.extend_from_slice(&0u16.to_be_bytes()); // reserved
        buf.extend_from_slice(auth);
        debug_assert_eq!(buf.len(), CLIENT_HANDSHAKE_SIZE + auth.len());
        Ok(buf)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < CLIENT_HANDSHAKE_SIZE {
            return Err(ProtocolError::TooShort {
                need: CLIENT_HANDSHAKE_SIZE,
                have: bytes.len(),
            });
        }
        require_magic(bytes)?;
        let auth_len = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
        if bytes.len() < CLIENT_HANDSHAKE_SIZE + auth_len {
            return Err(ProtocolError::TooShort {
                need: CLIENT_HANDSHAKE_SIZE + auth_len,
                have: bytes.len(),
            });
        }
        let auth =
            std::str::from_utf8(&bytes[CLIENT_HANDSHAKE_SIZE..CLIENT_HANDSHAKE_SIZE + auth_len])
                .map_err(|_| ProtocolError::InvalidUtf8)?;
        Ok(Self {
            proto_version: bytes[4],
            role: ChannelRole::from_wire(bytes[5]),
            auth_mode: AuthMode::from_wire(bytes[6]),
            client_nonce: u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
            auth: auth.to_string(),
        })
    }
}

/* ------------------------------------------------------------------ */
/* Server handshake response (§5.3) — 24 bytes + JSON                  */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, PartialEq)]
pub struct ServerHandshakeResponse {
    pub proto_version: u8,
    pub status: HandshakeStatus,
    /// Echo of the client's role.
    pub role: ChannelRole,
    pub json: Value,
}

/// Writes the response. On `ok` with auth mode 1 the body carries
/// `deviceId` / `deviceSecret` / `serverId` / `name`; otherwise it is `{}`.
pub fn write_server_response(
    status: HandshakeStatus,
    role: ChannelRole,
    body: &impl Serialize,
) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(body)?;
    if json.len() > MAX_CONTROL_JSON {
        return Err(ProtocolError::Oversized {
            len: json.len(),
            limit: MAX_CONTROL_JSON,
        });
    }
    let mut buf = Vec::with_capacity(SERVER_RESPONSE_SIZE + json.len());
    buf.extend_from_slice(&MAGIC_BYTES);
    buf.push(PROTOCOL_VERSION);
    buf.push(status as u8);
    buf.push(role as u8);
    buf.push(0); // flags
    buf.extend_from_slice(&0u64.to_be_bytes()); // reserved
    buf.extend_from_slice(&(json.len() as u32).to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes()); // reserved
    buf.extend_from_slice(&json);
    debug_assert_eq!(buf.len(), SERVER_RESPONSE_SIZE + json.len());
    Ok(buf)
}

impl ServerHandshakeResponse {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < SERVER_RESPONSE_SIZE {
            return Err(ProtocolError::TooShort {
                need: SERVER_RESPONSE_SIZE,
                have: bytes.len(),
            });
        }
        require_magic(bytes)?;
        let json_len = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as usize;
        if json_len > MAX_CONTROL_JSON {
            return Err(ProtocolError::Oversized {
                len: json_len,
                limit: MAX_CONTROL_JSON,
            });
        }
        if bytes.len() < SERVER_RESPONSE_SIZE + json_len {
            return Err(ProtocolError::TooShort {
                need: SERVER_RESPONSE_SIZE + json_len,
                have: bytes.len(),
            });
        }
        let body = &bytes[SERVER_RESPONSE_SIZE..SERVER_RESPONSE_SIZE + json_len];
        let json = if body.is_empty() {
            Value::Object(Default::default())
        } else {
            serde_json::from_slice(body)?
        };
        Ok(Self {
            proto_version: bytes[4],
            status: HandshakeStatus::from_wire(bytes[5]),
            role: ChannelRole::from_wire(bytes[6]),
            json,
        })
    }
}

/* ------------------------------------------------------------------ */
/* Control channel (§6) — u32 length + JSON                            */
/* ------------------------------------------------------------------ */

pub fn write_control_frame(message: &impl Serialize) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(message)?;
    if json.len() > MAX_CONTROL_JSON {
        return Err(ProtocolError::Oversized {
            len: json.len(),
            limit: MAX_CONTROL_JSON,
        });
    }
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&(json.len() as u32).to_be_bytes());
    buf.extend_from_slice(&json);
    Ok(buf)
}

/* ------------------------------------------------------------------ */
/* Video channel (§7.1) — u32 totalLen + 24-byte header + payload      */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoPacketHeader {
    /// Monotonic per stream, wraps at 2^32.
    pub frame_id: u32,
    /// Presentation timestamp in microseconds, phone clock.
    pub pts_us: i64,
    pub flags: VideoFlags,
    pub codec: VideoCodec,
    pub payload_len: u32,
}

impl VideoPacketHeader {
    pub const fn size() -> usize {
        VIDEO_HEADER_SIZE
    }

    /// Parses a complete framed packet (length prefix included) and returns the
    /// header plus the payload slice, without copying the payload.
    pub fn parse(frame: &[u8]) -> Result<(Self, &[u8])> {
        if frame.len() < 4 {
            return Err(ProtocolError::TooShort {
                need: 4,
                have: frame.len(),
            });
        }
        let total_len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        if total_len > MAX_VIDEO_PAYLOAD + VIDEO_HEADER_SIZE {
            return Err(ProtocolError::Oversized {
                len: total_len,
                limit: MAX_VIDEO_PAYLOAD + VIDEO_HEADER_SIZE,
            });
        }
        if frame.len() < 4 + total_len {
            return Err(ProtocolError::TooShort {
                need: 4 + total_len,
                have: frame.len(),
            });
        }
        let h = &frame[4..];
        if h.len() < VIDEO_HEADER_SIZE {
            return Err(ProtocolError::TooShort {
                need: VIDEO_HEADER_SIZE,
                have: h.len(),
            });
        }
        let payload_len = u32::from_be_bytes([h[16], h[17], h[18], h[19]]);
        if payload_len as usize + VIDEO_HEADER_SIZE != total_len {
            // A mismatch means the framing and the header disagree; trusting
            // either would read past the buffer.
            return Err(ProtocolError::MalformedAuth);
        }
        let header = Self {
            frame_id: u32::from_be_bytes([h[0], h[1], h[2], h[3]]),
            pts_us: i64::from_be_bytes(h[4..12].try_into().unwrap()),
            flags: VideoFlags(h[12]),
            codec: VideoCodec::from_wire(h[13]),
            payload_len,
        };
        let payload = &h[VIDEO_HEADER_SIZE..VIDEO_HEADER_SIZE + payload_len as usize];
        Ok((header, payload))
    }
}

pub fn write_video_packet(header: &VideoPacketHeader, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() != header.payload_len as usize {
        return Err(ProtocolError::MalformedAuth);
    }
    if payload.len() > MAX_VIDEO_PAYLOAD {
        return Err(ProtocolError::Oversized {
            len: payload.len(),
            limit: MAX_VIDEO_PAYLOAD,
        });
    }
    let total = VIDEO_HEADER_SIZE + payload.len();
    let mut buf = Vec::with_capacity(4 + total);
    buf.extend_from_slice(&(total as u32).to_be_bytes());
    buf.extend_from_slice(&header.frame_id.to_be_bytes());
    buf.extend_from_slice(&header.pts_us.to_be_bytes());
    buf.push(header.flags.0);
    buf.push(header.codec as u8);
    buf.extend_from_slice(&0u16.to_be_bytes()); // reserved
    buf.extend_from_slice(&header.payload_len.to_be_bytes());
    buf.extend_from_slice(&0u32.to_be_bytes()); // reserved
    buf.extend_from_slice(payload);
    debug_assert_eq!(buf.len(), 4 + total);
    Ok(buf)
}

/* ------------------------------------------------------------------ */
/* Loopback multiplex (§9) — one tag byte, then the tagged payload     */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LoopbackTag {
    /// `u32 jsonLen` + JSON — a command or reply.
    Control = 0x01,
    /// 24-byte video header + payload, no outer length.
    Video = 0x02,
    /// `u32 jsonLen` + JSON — an unsolicited event.
    Event = 0x03,
}

impl LoopbackTag {
    pub fn from_wire(value: u8) -> Option<Self> {
        match value {
            0x01 => Some(Self::Control),
            0x02 => Some(Self::Video),
            0x03 => Some(Self::Event),
            _ => None,
        }
    }
}

/// Frames one loopback message: tag byte, then the tagged body.
pub fn write_loopback(tag: LoopbackTag, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + body.len());
    buf.push(tag as u8);
    buf.extend_from_slice(body);
    buf
}

/* ------------------------------------------------------------------ */

fn require_magic(bytes: &[u8]) -> Result<()> {
    let magic = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if magic != MAGIC {
        return Err(ProtocolError::BadMagic);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_offsets_are_exact() {
        let nonce = (0u8..32).collect::<Vec<_>>();
        let b = write_server_greeting(HandshakeStatus::Busy, &nonce).unwrap();
        assert_eq!(&b[0..4], b"PCAM");
        assert_eq!(b[4], 1);
        assert_eq!(b[5], HandshakeStatus::Busy as u8);
        assert_eq!(&b[6..8], &[0, 0]);
        assert_eq!(&b[8..40], nonce.as_slice());
        assert_eq!(b.len(), 40);
    }

    #[test]
    fn handshake_offsets_are_exact() {
        let h = ClientHandshake {
            proto_version: 1,
            role: ChannelRole::Video,
            auth_mode: AuthMode::PairingToken,
            client_nonce: 1,
            auth: "abc".into(),
        };
        let b = h.to_bytes().unwrap();
        assert_eq!(&b[0..4], b"PCAM");
        assert_eq!(b[4], 1);
        assert_eq!(b[5], 2); // video
        assert_eq!(b[6], 1); // pairing token
        assert_eq!(b[7], 0); // flags
        assert_eq!(&b[8..16], &1u64.to_be_bytes());
        assert_eq!(&b[16..18], &3u16.to_be_bytes()); // authLen
        assert_eq!(&b[18..20], &[0, 0]);
        assert_eq!(&b[20..], b"abc");
    }

    #[test]
    fn video_header_offsets_are_exact() {
        let h = VideoPacketHeader {
            frame_id: 0xDEAD_BEEF,
            pts_us: -1,
            flags: VideoFlags::CODEC_CONFIG,
            codec: VideoCodec::H264,
            payload_len: 2,
        };
        let b = write_video_packet(&h, &[0x11, 0x22]).unwrap();
        assert_eq!(&b[0..4], &26u32.to_be_bytes()); // totalLen = 24 + 2
        assert_eq!(&b[4..8], &0xDEAD_BEEFu32.to_be_bytes()); // frameId
        assert_eq!(&b[8..16], &(-1i64).to_be_bytes()); // ptsUs
        assert_eq!(b[16], 0x02); // flags: codecConfig
        assert_eq!(b[17], 1); // codec: H.264
        assert_eq!(&b[18..20], &[0, 0]);
        assert_eq!(&b[20..24], &2u32.to_be_bytes()); // payloadLen
        assert_eq!(&b[24..28], &[0, 0, 0, 0]);
        assert_eq!(&b[28..], &[0x11, 0x22]);
    }

    #[test]
    fn video_parse_rejects_length_mismatch() {
        // totalLen says 24+4 but payloadLen says 99: the two disagree.
        let mut b = write_video_packet(
            &VideoPacketHeader {
                frame_id: 0,
                pts_us: 0,
                flags: VideoFlags::NONE,
                codec: VideoCodec::H264,
                payload_len: 4,
            },
            &[1, 2, 3, 4],
        )
        .unwrap();
        b[20] = 99;
        assert!(matches!(
            VideoPacketHeader::parse(&b),
            Err(ProtocolError::MalformedAuth)
        ));
    }

    #[test]
    fn loopback_tag_bytes_match_spec() {
        assert_eq!(LoopbackTag::Control as u8, 0x01);
        assert_eq!(LoopbackTag::Video as u8, 0x02);
        assert_eq!(LoopbackTag::Event as u8, 0x03);
        assert_eq!(LoopbackTag::from_wire(0x04), None);
    }

    #[test]
    fn unknown_status_and_role_fall_back_safely() {
        // A newer peer's unknown code must not panic; it maps to the closest
        // safe interpretation so the connection can be closed cleanly.
        assert_eq!(HandshakeStatus::from_wire(200), HandshakeStatus::BadRequest);
        assert_eq!(ChannelRole::from_wire(99), ChannelRole::Control);
        assert_eq!(AuthMode::from_wire(99), AuthMode::Loopback);
    }
}
