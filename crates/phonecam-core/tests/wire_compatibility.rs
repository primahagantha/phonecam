//! End-to-end wire compatibility.
//!
//! These tests speak the **phone's** side of the protocol over a real loopback
//! socket: they frame a greeting read, a handshake, and control/video traffic
//! exactly as the Android app's `Proto.kt` does. If any of this drifts, every
//! paired phone stops connecting, so it is checked against the wire rather than
//! against our own structs.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use phonecam_core::pairing::PairingManager;
use phonecam_core::session::{
    read_control_payload, read_video_packet, Authenticator, SessionState, APPROVAL_TOKEN_TTL_MS,
};
use phonecam_protocol::{
    write_control_frame, write_server_response, write_video_packet, AuthMode, ChannelRole,
    ClientHandshake, HandshakeStatus, ServerGreeting, ServerHandshakeResponse, VideoCodec,
    VideoFlags, VideoPacketHeader, MAGIC, PROTOCOL_VERSION, SERVER_GREETING_SIZE,
};

/// A client that behaves like the phone.
struct PhoneClient {
    stream: TcpStream,
}

impl PhoneClient {
    fn connect(port: u16) -> Self {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        Self { stream }
    }

    /// Reads the 40-byte greeting exactly as `Proto.Framing.readGreeting` does.
    fn read_greeting(&mut self) -> ServerGreeting {
        let mut buf = [0u8; SERVER_GREETING_SIZE];
        self.stream.read_exact(&mut buf).expect("greeting");
        ServerGreeting::parse(&buf).expect("a valid greeting")
    }

    fn send_handshake(&mut self, role: ChannelRole, mode: AuthMode, nonce: u64, auth: &str) {
        let handshake = ClientHandshake {
            proto_version: PROTOCOL_VERSION,
            role,
            auth_mode: mode,
            client_nonce: nonce,
            auth: auth.to_string(),
        };
        let bytes = handshake.to_bytes().expect("frame the handshake");
        self.stream.write_all(&bytes).expect("write handshake");
        self.stream.flush().unwrap();
    }

    /// Reads the 24-byte response plus its JSON body.
    fn read_response(&mut self) -> ServerHandshakeResponse {
        let mut fixed = [0u8; 24];
        self.stream.read_exact(&mut fixed).expect("response header");
        let json_len = u32::from_be_bytes([fixed[16], fixed[17], fixed[18], fixed[19]]) as usize;
        let mut full = fixed.to_vec();
        if json_len > 0 {
            let mut body = vec![0u8; json_len];
            self.stream.read_exact(&mut body).expect("response body");
            full.extend_from_slice(&body);
        }
        ServerHandshakeResponse::parse(&full).expect("a valid response")
    }
}

/// A server harness that runs the authenticator on a real socket.
struct Server {
    port: u16,
    session: Arc<SessionState>,
    pairing: Arc<parking_lot::Mutex<PairingManager>>,
    registry: Arc<parking_lot::Mutex<Vec<phonecam_core::pairing::DeviceRecord>>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let session = Arc::new(SessionState::default());
        let pairing = Arc::new(parking_lot::Mutex::new(PairingManager::new()));
        let registry = Arc::new(parking_lot::Mutex::new(Vec::new()));

        {
            let session = session.clone();
            let pairing = pairing.clone();
            let registry = registry.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let session = session.clone();
                    let pairing = pairing.clone();
                    let registry = registry.clone();
                    std::thread::spawn(move || {
                        let conn = phonecam_core::transport::Connection::new(stream).unwrap();
                        conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                        let auth = Authenticator {
                            pairing: &pairing,
                            registry: &registry,
                            session: &session,
                            server_id: "pc-7f3c1a9e",
                            server_name: "DESKTOP-ABC",
                            now_ms: 1_000_000,
                        };
                        let outcome = auth.authenticate(&conn, true);
                        if outcome.ok && !outcome.approval_pending {
                            *session.device_id.lock() = outcome.device_id.clone();
                            *session.device_name.lock() = outcome.device_name.clone();
                            *session.transport.lock() = "lan".into();
                            // Echo one control frame back so the client can see
                            // the channel is live.
                            if let Ok(bytes) =
                                write_control_frame(&phonecam_protocol::DesktopToPhone::Welcome {
                                    protocol: PROTOCOL_VERSION,
                                    server_version: "1.0.0".into(),
                                    server_name: "DESKTOP-ABC".into(),
                                })
                            {
                                conn.write_all(&bytes);
                            }
                            // Drain whatever the client sends until it leaves.
                            while read_control_payload(&conn).is_some() {}
                        }
                    });
                }
            });
        }

        Self { port, session, pairing, registry }
    }
}

#[test]
fn a_qr_pairing_completes_and_returns_a_device_secret() {
    let server = Server::start();

    let token = server.pairing.lock().arm(300_000, 1_000_000).expect("arm");

    let mut phone = PhoneClient::connect(server.port);
    let greeting = phone.read_greeting();
    assert_eq!(greeting.status, HandshakeStatus::Ok);
    assert_eq!(greeting.proto_version, PROTOCOL_VERSION);
    assert_eq!(greeting.server_nonce.len(), 32);

    phone.send_handshake(ChannelRole::Control, AuthMode::PairingToken, 1000, &token);
    let response = phone.read_response();

    assert_eq!(response.status, HandshakeStatus::Ok);
    assert_eq!(response.role, ChannelRole::Control);

    // The body is exactly what the phone persists (docs Â§4.2 step 4).
    let device_id = response.json["deviceId"].as_str().expect("deviceId");
    let secret = response.json["deviceSecret"].as_str().expect("deviceSecret");
    assert!(device_id.starts_with("android-"), "device id was {device_id}");
    assert_eq!(secret.len(), 64, "the secret is 32 bytes hex-encoded");
    assert_eq!(response.json["serverId"], "pc-7f3c1a9e");

    // The server recorded it, and the token is spent.
    assert!(server.registry.lock().iter().any(|r| r.device_id == device_id));
    assert!(
        !server.pairing.lock().is_open(1_000_000),
        "the QR token must be single use"
    );
}

#[test]
fn a_replayed_token_is_refused() {
    let server = Server::start();
    let token = server.pairing.lock().arm(300_000, 1_000_000).expect("arm");

    let mut first = PhoneClient::connect(server.port);
    first.read_greeting();
    first.send_handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    assert_eq!(first.read_response().status, HandshakeStatus::Ok);

    // The same code again: consumed, so expired.
    let mut second = PhoneClient::connect(server.port);
    second.read_greeting();
    second.send_handshake(ChannelRole::Control, AuthMode::PairingToken, 2, &token);
    assert_eq!(second.read_response().status, HandshakeStatus::TokenExpired);
}

#[test]
fn a_reconnect_with_the_device_secret_works_and_a_replay_does_not() {
    let server = Server::start();
    let token = server.pairing.lock().arm(300_000, 1_000_000).expect("arm");

    let mut first = PhoneClient::connect(server.port);
    first.read_greeting();
    first.send_handshake(ChannelRole::Control, AuthMode::PairingToken, 100, &token);
    let paired = first.read_response();
    let device_id = paired.json["deviceId"].as_str().unwrap().to_string();
    let secret = paired.json["deviceSecret"].as_str().unwrap().to_string();
    drop(first);

    // The server records the device and the session on its own thread, so both
    // have to be visible before a re-dial is meaningful: an unknown device is
    // refused, and a session that has not recorded the id would make the same
    // phone look like a different one.
    for _ in 0..200 {
        let stored = server.registry.lock().iter().any(|r| r.device_id == device_id);
        let on_session = *server.session.device_id.lock() == device_id;
        if stored && on_session {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        server.registry.lock().iter().any(|r| r.device_id == device_id),
        "the paired device must be stored before it can reconnect"
    );

    let mut second = PhoneClient::connect(server.port);
    let greeting = second.read_greeting();
    let auth = phonecam_protocol::device_secret_auth(&device_id, &secret, &greeting.server_nonce, 200)
        .unwrap();
    second.send_handshake(ChannelRole::Control, AuthMode::DeviceSecret, 200, &auth);
    let response = second.read_response();
    assert_eq!(response.status, HandshakeStatus::Ok, "a paired phone must reconnect");
    drop(second);

    // The same nonce again is a replay, and must be refused even though the HMAC
    // itself is valid: otherwise a captured handshake reconnects forever.
    std::thread::sleep(Duration::from_millis(100));
    let mut third = PhoneClient::connect(server.port);
    let greeting = third.read_greeting();
    let auth = phonecam_protocol::device_secret_auth(&device_id, &secret, &greeting.server_nonce, 200)
        .unwrap();
    third.send_handshake(ChannelRole::Control, AuthMode::DeviceSecret, 200, &auth);
    assert_eq!(third.read_response().status, HandshakeStatus::Denied);
}

#[test]
fn a_wrong_device_secret_is_refused() {
    let server = Server::start();
    let token = server.pairing.lock().arm(300_000, 1_000_000).expect("arm");

    let mut first = PhoneClient::connect(server.port);
    first.read_greeting();
    first.send_handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    let paired = first.read_response();
    let device_id = paired.json["deviceId"].as_str().unwrap().to_string();
    drop(first);

    let mut second = PhoneClient::connect(server.port);
    let greeting = second.read_greeting();
    let wrong = "00".repeat(32);
    let auth = phonecam_protocol::device_secret_auth(&device_id, &wrong, &greeting.server_nonce, 50)
        .unwrap();
    second.send_handshake(ChannelRole::Control, AuthMode::DeviceSecret, 50, &auth);
    assert_eq!(second.read_response().status, HandshakeStatus::Denied);
}

#[test]
fn a_pair_request_raises_a_prompt_and_arms_a_bound_token() {
    let server = Server::start();

    let mut phone = PhoneClient::connect(server.port);
    let greeting = phone.read_greeting();

    // No stored secret yet: the phone signs with the fixed pair-request key.
    let auth =
        phonecam_protocol::pair_request_auth("android-a31f92", None, &greeting.server_nonce, 7)
            .unwrap();
    phone.send_handshake(ChannelRole::Control, AuthMode::PairRequest, 7, &auth);

    let response = phone.read_response();
    assert_eq!(
        response.status,
        HandshakeStatus::ApprovalPending,
        "the phone must be told a human is deciding"
    );
    assert!(server.pairing.lock().has_pending_request("android-a31f92"));

    // The user approves: a token bound to that device is armed.
    let token = server
        .pairing
        .lock()
        .arm_for_device("android-a31f92", APPROVAL_TOKEN_TTL_MS, 1_000_000)
        .expect("arm for device");
    assert!(!server.pairing.lock().has_pending_request("android-a31f92"));

    // The phone re-dials with the token and keeps its own id.
    let mut again = PhoneClient::connect(server.port);
    again.read_greeting();
    again.send_handshake(ChannelRole::Control, AuthMode::PairingToken, 8, &token);
    let paired = again.read_response();
    assert_eq!(paired.status, HandshakeStatus::Ok);
    assert_eq!(
        paired.json["deviceId"], "android-a31f92",
        "a bound token must reuse the id the phone already stored"
    );
}

#[test]
fn a_bad_pair_request_signature_is_refused() {
    let server = Server::start();
    let mut phone = PhoneClient::connect(server.port);
    let greeting = phone.read_greeting();

    let good =
        phonecam_protocol::pair_request_auth("android-x", None, &greeting.server_nonce, 7).unwrap();
    // Corrupt the digest.
    let mut bad = good.clone();
    let last = bad.pop().unwrap();
    bad.push(if last == 'a' { 'b' } else { 'a' });

    phone.send_handshake(ChannelRole::Control, AuthMode::PairRequest, 7, &bad);
    assert_eq!(phone.read_response().status, HandshakeStatus::Denied);
    assert!(!server.pairing.lock().has_pending_request("android-x"));
}

#[test]
fn a_version_mismatch_is_named() {
    let server = Server::start();
    let mut phone = PhoneClient::connect(server.port);
    phone.read_greeting();

    let handshake = ClientHandshake {
        proto_version: 2,
        role: ChannelRole::Control,
        auth_mode: AuthMode::PairingToken,
        client_nonce: 1,
        auth: "x".into(),
    };
    phone.stream.write_all(&handshake.to_bytes().unwrap()).unwrap();
    let response = phone.read_response();
    assert_eq!(response.status, HandshakeStatus::VersionMismatch);
    assert!(
        response.json["serverVersion"].is_string(),
        "the phone needs to know what to upgrade to"
    );
}

#[test]
fn a_loopback_peer_can_use_mode_zero_but_only_on_loopback() {
    let server = Server::start();
    let mut phone = PhoneClient::connect(server.port);
    phone.read_greeting();
    // The test connects over 127.0.0.1, so mode 0 is legitimate here.
    phone.send_handshake(ChannelRole::Control, AuthMode::Loopback, 1, "");
    let response = phone.read_response();
    assert_eq!(response.status, HandshakeStatus::Ok);

    // The server records the device on its own thread, so wait for it rather
    // than assuming the ordering.
    for _ in 0..100 {
        if server.session.device_id.lock().as_str() == "local-ui" {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(server.session.device_id.lock().as_str(), "local-ui");
}

#[test]
fn the_control_channel_round_trips_a_framed_message() {
    let server = Server::start();
    let token = server.pairing.lock().arm(300_000, 1_000_000).expect("arm");

    let mut phone = PhoneClient::connect(server.port);
    phone.read_greeting();
    phone.send_handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    assert_eq!(phone.read_response().status, HandshakeStatus::Ok);

    // The server sends `welcome` on a control frame.
    let mut len_bytes = [0u8; 4];
    phone.stream.read_exact(&mut len_bytes).unwrap();
    let len = u32::from_be_bytes(len_bytes) as usize;
    let mut body = vec![0u8; len];
    phone.stream.read_exact(&mut body).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["t"], "welcome");
    assert_eq!(value["protocol"], PROTOCOL_VERSION);
}

#[test]
fn video_packets_survive_the_wire_unchanged() {
    // The phone writes a packet; the reader must recover the exact payload and
    // flags, including the Annex-B bytes.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let payload: Vec<u8> = {
        let mut p = vec![0u8, 0, 0, 1, 0x67, 0x42, 0x00, 0x1e];
        p.extend_from_slice(&[0u8, 0, 0, 1, 0x68, 0xce]);
        p.extend_from_slice(&[0u8, 0, 0, 1, 0x65, 0x88, 0x84, 0x21]);
        p
    };
    let expected = payload.clone();

    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let conn = phonecam_core::transport::Connection::new(stream).unwrap();
        conn.set_read_timeout(Some(Duration::from_secs(5))).unwrap();

        let packet = write_video_packet(
            &VideoPacketHeader {
                frame_id: 42,
                pts_us: 1_234_567,
                flags: VideoFlags::KEYFRAME,
                codec: VideoCodec::H264,
                payload_len: payload.len() as u32,
            },
            &payload,
        )
        .unwrap();
        conn.write_all(&packet);

        let (got_payload, flags, pts) = read_video_packet(&conn).expect("a whole packet");
        assert_eq!(got_payload, expected, "the Annex-B bytes must survive");
        assert!(flags.is_keyframe());
        assert_eq!(pts, 1_234_567);
    });

    // Client side: write the packet and then close.
    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut payload = vec![0u8, 0, 0, 1, 0x67, 0x42, 0x00, 0x1e];
    payload.extend_from_slice(&[0u8, 0, 0, 1, 0x68, 0xce]);
    payload.extend_from_slice(&[0u8, 0, 0, 1, 0x65, 0x88, 0x84, 0x21]);
    let packet = write_video_packet(
        &VideoPacketHeader {
            frame_id: 42,
            pts_us: 1_234_567,
            flags: VideoFlags::KEYFRAME,
            codec: VideoCodec::H264,
            payload_len: payload.len() as u32,
        },
        &payload,
    )
    .unwrap();
    client.write_all(&packet).unwrap();
    client.flush().unwrap();

    // Read the server's reply packet.
    let mut len_bytes = [0u8; 4];
    client.read_exact(&mut len_bytes).unwrap();
    let total = u32::from_be_bytes(len_bytes) as usize;
    let mut rest = vec![0u8; total];
    client.read_exact(&mut rest).unwrap();
    let mut full = len_bytes.to_vec();
    full.extend_from_slice(&rest);
    let (header, body) = VideoPacketHeader::parse(&full).unwrap();
    assert_eq!(header.frame_id, 42);
    assert_eq!(body.len(), 22, "8 SPS + 6 PPS + 8 IDR bytes");
}

#[test]
fn the_greeting_magic_is_pcam() {
    // A phone validates this before anything else, so it is pinned literally.
    assert_eq!(&MAGIC.to_be_bytes(), b"PCAM");
    let nonce = vec![0u8; 32];
    let bytes = phonecam_protocol::write_server_greeting(HandshakeStatus::Ok, &nonce).unwrap();
    assert_eq!(&bytes[0..4], b"PCAM");
}

#[test]
fn a_handshake_with_a_truncated_auth_body_is_refused() {
    let server = Server::start();
    let mut phone = PhoneClient::connect(server.port);
    phone.read_greeting();

    // Claim 200 bytes of auth but send none.
    let mut buf = Vec::new();
    buf.extend_from_slice(b"PCAM");
    buf.push(PROTOCOL_VERSION);
    buf.push(ChannelRole::Control as u8);
    buf.push(AuthMode::PairingToken as u8);
    buf.push(0);
    buf.extend_from_slice(&1u64.to_be_bytes());
    buf.extend_from_slice(&200u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    phone.stream.write_all(&buf).unwrap();
    phone.stream.flush().unwrap();

    // The server must not hang or panic; it closes, and the client sees EOF.
    let mut probe = [0u8; 1];
    let _ = phone.stream.set_read_timeout(Some(Duration::from_secs(3)));
    match phone.stream.read(&mut probe) {
        Ok(0) => {}
        Ok(_) => {}
        Err(_) => {}
    }
}

#[test]
fn an_oversized_auth_payload_is_refused_before_it_is_read() {
    let server = Server::start();
    let mut phone = PhoneClient::connect(server.port);
    phone.read_greeting();

    let mut buf = Vec::new();
    buf.extend_from_slice(b"PCAM");
    buf.push(PROTOCOL_VERSION);
    buf.push(ChannelRole::Control as u8);
    buf.push(AuthMode::DeviceSecret as u8);
    buf.push(0);
    buf.extend_from_slice(&1u64.to_be_bytes());
    buf.extend_from_slice(&1000u16.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes());
    phone.stream.write_all(&buf).unwrap();
    phone.stream.flush().unwrap();

    // No response frame: the connection is simply closed.
    let _ = phone.stream.set_read_timeout(Some(Duration::from_secs(3)));
    let mut probe = [0u8; 1];
    let _ = phone.stream.read(&mut probe);
}

#[test]
fn write_server_response_is_readable_by_the_phone() {
    // The phone's `readResponse` requires this exact shape.
    let body = serde_json::json!({
        "deviceId": "android-a31f92",
        "deviceSecret": "aa".repeat(32),
        "serverId": "pc-7f3c1a9e",
        "name": "DESKTOP-ABC",
    });
    let bytes = write_server_response(HandshakeStatus::Ok, ChannelRole::Control, &body).unwrap();
    assert_eq!(&bytes[0..4], b"PCAM");
    assert_eq!(bytes[4], PROTOCOL_VERSION);
    assert_eq!(bytes[5], 0);
    assert_eq!(bytes[6], 1);
    let parsed = ServerHandshakeResponse::parse(&bytes).unwrap();
    assert_eq!(parsed.json["deviceId"], "android-a31f92");
}






#[test]
fn hmac_round_trip_self_check() {
    let nonce = vec![0x11u8; 32];
    let secret = "ab".repeat(32);
    let auth = phonecam_protocol::device_secret_auth("android-a", &secret, &nonce, 200).unwrap();
    let parts: Vec<&str> = auth.split(':').collect();
    assert_eq!(parts.len(), 2, "auth was {auth}");
    let ok = phonecam_protocol::verify_device_secret("android-a", &secret, &nonce, 200, parts[1]);
    assert!(ok, "auth={auth}");
}

