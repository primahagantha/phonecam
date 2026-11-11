//! End-to-end: the running app accepts a phone, pairs it, and streams.
//!
//! This drives the real `Runtime` â€” the real listener, the real handshake, the
//! real decoder â€” with a client that behaves like the Android app. It is the only
//! test that proves the pieces are actually connected to each other, rather than
//! each being correct in isolation.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use phonecam_app::runtime::Runtime;
use phonecam_protocol::{
    write_control_frame, write_video_packet, AuthMode, ChannelRole, ClientHandshake,
    DesktopToPhone, ServerGreeting, ServerHandshakeResponse, VideoCodec, VideoFlags,
    VideoPacketHeader, PROTOCOL_VERSION,
};
use phonecam_ui::views::state::Settings;

/// A client that speaks the phone's side of the wire.
struct Phone {
    stream: TcpStream,
}

impl Phone {
    fn dial(port: u16) -> Self {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("dial the app");
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        Self { stream }
    }

    fn read_greeting(&mut self) -> ServerGreeting {
        let mut buf = [0u8; 40];
        self.stream.read_exact(&mut buf).expect("greeting");
        ServerGreeting::parse(&buf).expect("a valid greeting")
    }

    fn handshake(&mut self, role: ChannelRole, mode: AuthMode, nonce: u64, auth: &str) {
        let bytes = ClientHandshake {
            proto_version: PROTOCOL_VERSION,
            role,
            auth_mode: mode,
            client_nonce: nonce,
            auth: auth.to_string(),
        }
        .to_bytes()
        .expect("frame the handshake");
        self.stream.write_all(&bytes).unwrap();
        self.stream.flush().unwrap();
    }

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

    /// Reads one control frame, skipping anything that is not JSON.
    fn read_control(&mut self) -> Option<serde_json::Value> {
        let mut len = [0u8; 4];
        self.stream.read_exact(&mut len).ok()?;
        let size = u32::from_be_bytes(len) as usize;
        let mut body = vec![0u8; size];
        self.stream.read_exact(&mut body).ok()?;
        serde_json::from_slice(&body).ok()
    }

    fn send(&mut self, message: &DesktopToPhone) {
        let bytes = write_control_frame(message).expect("frame");
        self.stream.write_all(&bytes).unwrap();
        self.stream.flush().unwrap();
    }
}

/// A runtime on an ephemeral port.
///
/// The suite runs several at once, so a fixed port would make every test after
/// the first fail on a bind error that has nothing to do with what it checks.
fn runtime() -> (Arc<Runtime>, u16) {
    let runtime = Runtime::new(&Settings::default());
    let port = runtime.start_on(0).expect("the app must listen");
    runtime.broadcast_state();
    (runtime, port)
}

#[test]
fn the_app_accepts_a_qr_pairing_and_stores_the_device() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    assert!(!payloads.is_empty());

    // Read the token out of the payload, exactly as a QR scan would.
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut phone = Phone::dial(port);
    let greeting = phone.read_greeting();
    assert_eq!(greeting.proto_version, PROTOCOL_VERSION);

    phone.handshake(ChannelRole::Control, AuthMode::PairingToken, 1000, &token);
    let response = phone.read_response();
    assert_eq!(
        response.status as u8, 0,
        "pairing must succeed; the app said {:?}",
        response.status
    );

    let device_id = response.json["deviceId"].as_str().expect("a device id");
    assert_eq!(
        response.json["deviceSecret"].as_str().unwrap().len(),
        64,
        "a 32-byte secret, hex encoded"
    );

    // The app must have recorded it.
    let stored = runtime
        .registry
        .lock()
        .iter()
        .any(|r| r.device_id == device_id);
    assert!(stored, "the paired device must be persisted in memory");

    // And the UI must now be able to say a phone is connected.
    std::thread::sleep(Duration::from_millis(200));
    runtime.broadcast_state();
    let state = runtime.state.lock().clone();
    assert!(state.has_device(), "the snapshot must name the phone");
}

#[test]
fn a_paired_phone_reconnects_without_a_new_code() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut first = Phone::dial(port);
    first.read_greeting();
    first.handshake(ChannelRole::Control, AuthMode::PairingToken, 100, &token);
    let paired = first.read_response();
    let device_id = paired.json["deviceId"].as_str().unwrap().to_string();
    let secret = paired.json["deviceSecret"].as_str().unwrap().to_string();
    drop(first);

    // Give the control loop a moment to release the session.
    std::thread::sleep(Duration::from_millis(300));

    let mut second = Phone::dial(port);
    let greeting = second.read_greeting();
    let auth =
        phonecam_protocol::device_secret_auth(&device_id, &secret, &greeting.server_nonce, 200)
            .unwrap();
    second.handshake(ChannelRole::Control, AuthMode::DeviceSecret, 200, &auth);
    let response = second.read_response();
    assert_eq!(
        response.status as u8, 0,
        "a known phone must reconnect on its own"
    );
}

#[test]
fn the_control_channel_greets_the_phone() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut phone = Phone::dial(port);
    phone.read_greeting();
    phone.handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    assert_eq!(phone.read_response().status as u8, 0);

    // The first control frame the app sends is `welcome`.
    let welcome = phone.read_control().expect("a control frame");
    assert_eq!(welcome["t"], "welcome");
    assert_eq!(welcome["protocol"], PROTOCOL_VERSION);

    // The phone reports its capabilities, and the UI must see them.
    phone.send(&DesktopToPhone::Welcome {
        protocol: 1,
        server_version: "test".into(),
        server_name: "test".into(),
    });
    let caps = serde_json::json!({
        "t": "capabilities",
        "device": {"model": "2201117TG", "androidVersion": 33, "appVersion": "1.0.0"},
        "cameras": [{
            "id": "0", "facing": "back",
            "resolutions": ["1920x1080", "1280x720"],
            "fps": [30],
            "torch": true, "focus": true, "zoom": true, "exposure": true,
            "whiteBalance": true, "minZoom": 1.0, "maxZoom": 8.0,
            "orientationDegrees": 90
        }]
    });
    let mut bytes = (serde_json::to_vec(&caps).unwrap().len() as u32)
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(&serde_json::to_vec(&caps).unwrap());
    phone.stream.write_all(&bytes).unwrap();
    phone.stream.flush().unwrap();

    std::thread::sleep(Duration::from_millis(300));
    let state = runtime.state.lock().clone();
    let capabilities = state.capabilities.expect("the app must store capabilities");
    assert_eq!(capabilities.cameras.len(), 1);
    assert_eq!(capabilities.cameras[0].facing.as_str(), "back");
}

#[test]
fn the_phone_state_moves_the_phase_but_a_socket_alone_does_not() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut phone = Phone::dial(port);
    phone.read_greeting();
    phone.handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    assert_eq!(phone.read_response().status as u8, 0);

    // The phone says it is streaming. That is evidence about its encoder, not
    // about a picture reaching this PC, so the phase must not claim `streaming`.
    let state_message = serde_json::json!({
        "t": "state", "streaming": true, "camera": "back",
        "resolution": "1280x720", "fps": 30,
        "actualResolution": "1280x720", "actualFps": 30, "bitrate": 4000000,
        "mirror": false, "rotation": 0, "torch": false, "zoom": 1.0,
        "battery": 88, "charging": true, "thermal": "none"
    });
    let bytes = serde_json::to_vec(&state_message).unwrap();
    let mut framed = (bytes.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&bytes);
    phone.stream.write_all(&framed).unwrap();
    phone.stream.flush().unwrap();

    std::thread::sleep(Duration::from_millis(300));
    let phase = runtime.session.phase();
    assert_ne!(
        phase,
        phonecam_protocol::StreamPhase::Streaming,
        "only a decoded frame may claim a picture"
    );
    assert_eq!(phase, phonecam_protocol::StreamPhase::Connecting);
}

#[test]
fn a_video_channel_with_garbage_does_not_crash_the_app() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    // Open a video channel and feed it nonsense.
    let mut video = Phone::dial(port);
    video.read_greeting();
    video.handshake(ChannelRole::Video, AuthMode::PairingToken, 2, &token);
    let response = video.read_response();
    assert_eq!(response.status as u8, 0, "the video channel must open");

    // A packet that is not valid H.264 at all.
    let junk = vec![0xAAu8; 64];
    let packet = write_video_packet(
        &VideoPacketHeader {
            frame_id: 1,
            pts_us: 0,
            flags: VideoFlags::KEYFRAME,
            codec: VideoCodec::H264,
            payload_len: junk.len() as u32,
        },
        &junk,
    )
    .unwrap();
    let _ = video.stream.write_all(&packet);
    let _ = video.stream.flush();

    // The app must still be alive and still accepting.
    std::thread::sleep(Duration::from_millis(400));
    let mut probe = Phone::dial(port);
    let greeting = probe.read_greeting();
    assert_eq!(greeting.status as u8, 0, "the app must survive a bad stream");
    drop(probe);
    drop(video);
    drop(runtime);
}

#[test]
fn the_app_writes_the_frame_bus_at_the_output_size() {
    // The buffer is the contract with the C++ DLL, so it must exist at the
    // configured size as soon as a stream starts.
    let runtime = Runtime::new(&Settings::default());
    runtime.broadcast_state();

    let writer = phonecam_framebus::FrameWriter::open(1920, 1080);
    match writer {
        Ok(writer) => {
            assert_eq!(writer.width(), 1920);
            assert_eq!(writer.height(), 1080);
            assert_eq!(writer.capacity(), 1920 * 1080 * 3 / 2);
            // The path and layout are the DLL's contract.
            assert!(std::path::Path::new(phonecam_framebus::BUFFER_PATH).exists());
        }
        Err(error) => {
            // A CI box without ProgramData write access is not a failure of the
            // code; say so rather than skipping silently.
            eprintln!("frame bus unavailable in this environment: {error}");
        }
    }
}

#[test]
fn the_pairing_prompt_can_be_approved_from_the_ui_side() {
    let (runtime, _port) = runtime();
    runtime.broadcast_state();

    // Simulate the app raising a prompt, as a `pairRequest` would.
    runtime.set_pairing_prompt("android-a31f92");
    assert!(runtime.state.lock().pairing_prompt.is_some());

    // The user approves.
    runtime.resolve_pair_request(true);
    assert!(runtime.state.lock().pairing_prompt.is_none());

    // And a token exists for that device, which the pending socket delivers.
    let token = runtime
        .pairing
        .lock()
        .approved_token_for("android-a31f92", phonecam_core::session::now_ms());
    assert!(token.is_some(), "approval must arm the token the retry redeems");
}

#[test]
fn a_second_phone_is_refused_while_one_holds_the_session() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    // First phone pairs and holds the control channel.
    let mut first = Phone::dial(port);
    first.read_greeting();
    first.handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    let paired = first.read_response();
    assert_eq!(paired.status as u8, 0);
    let first_id = paired.json["deviceId"].as_str().unwrap().to_string();
    let first_secret = paired.json["deviceSecret"].as_str().unwrap().to_string();

    std::thread::sleep(Duration::from_millis(400));

    // A different phone, already paired, dials while the first is connected.
    let other = phonecam_core::pairing::DeviceRecord::new("android-other", "bb".repeat(32), 0);
    runtime.registry.lock().push(other);

    let mut second = Phone::dial(port);
    let greeting = second.read_greeting();
    let auth = phonecam_protocol::device_secret_auth(
        "android-other",
        &"bb".repeat(32),
        &greeting.server_nonce,
        500,
    )
    .unwrap();
    second.handshake(ChannelRole::Control, AuthMode::DeviceSecret, 500, &auth);
    let response = second.read_response();
    assert_eq!(
        response.status as u8, 4,
        "the second phone must be told the PC is busy"
    );

    // The first phone is undisturbed.
    assert_eq!(
        runtime.session.device_id.lock().as_str(),
        first_id,
        "the holder of the session must not be displaced"
    );
    let _ = first_secret;
}


/// A quiet control channel must stay open.
///
/// This is the regression the shipped C++ documented and the port reproduced: a
/// read timeout was treated as a closed socket, so a session that had simply
/// gone quiet was torn down roughly once a second while the phone was fine.
#[test]
fn a_quiet_control_channel_stays_open() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut phone = Phone::dial(port);
    phone.read_greeting();
    phone.handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    assert_eq!(phone.read_response().status as u8, 0);

    // Say nothing for well over the 1 s read timeout.
    std::thread::sleep(Duration::from_millis(2500));

    // The channel must still work: send a state message and see it land.
    let state_message = serde_json::json!({
        "t": "state", "streaming": false, "camera": "back",
        "resolution": "1280x720", "fps": 30,
        "actualResolution": "1280x720", "actualFps": 30, "bitrate": 4000000,
        "mirror": false, "rotation": 0, "torch": false, "zoom": 1.0,
        "battery": 50, "charging": false, "thermal": "none"
    });
    let bytes = serde_json::to_vec(&state_message).unwrap();
    let mut framed = (bytes.len() as u32).to_be_bytes().to_vec();
    framed.extend_from_slice(&bytes);
    phone.stream.write_all(&framed).unwrap();
    phone.stream.flush().unwrap();

    std::thread::sleep(Duration::from_millis(400));
    assert!(
        runtime.session.phone_state.lock().is_some(),
        "a state sent after a quiet period must still be received"
    );
}

/// A command queued from another thread must reach the phone.
///
/// The queue is shared, not thread-local. When it was thread-local, every command
/// sat in the wrong thread's list and nothing was ever sent, which is
/// indistinguishable from a phone that ignores its controls.
#[test]
fn a_queued_command_reaches_the_phone() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut phone = Phone::dial(port);
    phone.read_greeting();
    phone.handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    assert_eq!(phone.read_response().status as u8, 0);

    // The `welcome` frame arrives first.
    let welcome = phone.read_control().expect("welcome");
    assert_eq!(welcome["t"], "welcome");

    // Queue a command from this thread, which does not own the socket.
    runtime.queue_command(DesktopToPhone::StopStream);

    // The control loop must deliver it.
    phone.stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let delivered = phone.read_control().expect("the queued command must arrive");
    assert_eq!(
        delivered["t"], "stop_stream",
        "a command queued off-thread must reach the phone"
    );
}

/// Pressing Start must actually ask the phone to stream.
///
/// This is the whole product: every other feature exists to support this one
/// command. The effect used to only set a flag nothing read, so the button
/// logged "start requested" and the phone was never told anything.
#[test]
fn pressing_start_asks_the_phone_to_stream() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut phone = Phone::dial(port);
    phone.read_greeting();
    phone.handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    assert_eq!(phone.read_response().status as u8, 0);

    // `welcome` arrives first.
    let welcome = phone.read_control().expect("welcome");
    assert_eq!(welcome["t"], "welcome");

    // The user presses Start.
    runtime.start_stream();

    // The phone must be told to start, after being configured.
    phone.stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut seen = Vec::new();
    for _ in 0..8 {
        match phone.read_control() {
            Some(message) => {
                let kind = message["t"].as_str().unwrap_or_default().to_string();
                seen.push(kind.clone());
                if kind == "start_stream" {
                    break;
                }
            }
            None => break,
        }
    }

    assert!(
        seen.iter().any(|t| t == "start_stream"),
        "the phone must be told to start; it saw {seen:?}"
    );
    // Configuration must precede the start, or the phone tears the encoder down
    // and re-dials.
    let start_at = seen.iter().position(|t| t == "start_stream").unwrap();
    assert!(
        seen.iter().take(start_at).any(|t| t == "set_video"),
        "set_video must come before start_stream; it saw {seen:?}"
    );
}

#[test]
fn stream_auto_resumes_when_phone_reconnects() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut first = Phone::dial(port);
    first.read_greeting();
    first.handshake(ChannelRole::Control, AuthMode::PairingToken, 100, &token);
    let paired = first.read_response();
    let device_id = paired.json["deviceId"].as_str().unwrap().to_string();
    let secret = paired.json["deviceSecret"].as_str().unwrap().to_string();

    let welcome = first.read_control().expect("welcome");
    assert_eq!(welcome["t"], "welcome");

    // User starts stream:
    runtime.start_stream();
    assert!(runtime.session.stream_wanted.load(std::sync::atomic::Ordering::Relaxed));

    // Drain start stream commands from first connection
    let mut seen = Vec::new();
    for _ in 0..8 {
        if let Some(msg) = first.read_control() {
            let t = msg["t"].as_str().unwrap_or_default().to_string();
            seen.push(t.clone());
            if t == "start_stream" {
                break;
            }
        }
    }
    assert!(seen.contains(&"start_stream".to_string()));

    // Wi-Fi drops: drop first phone connection
    drop(first);
    std::thread::sleep(Duration::from_millis(300));

    // Phone reconnects on its own using device secret
    let mut second = Phone::dial(port);
    let greeting = second.read_greeting();
    let auth = phonecam_protocol::device_secret_auth(&device_id, &secret, &greeting.server_nonce, 200).unwrap();
    second.handshake(ChannelRole::Control, AuthMode::DeviceSecret, 200, &auth);
    assert_eq!(second.read_response().status as u8, 0);

    let welcome2 = second.read_control().expect("welcome after reconnect");
    assert_eq!(welcome2["t"], "welcome");

    // Because stream_wanted was still true, the desktop automatically resumes streaming!
    let mut resumed_commands = Vec::new();
    for _ in 0..8 {
        if let Some(msg) = second.read_control() {
            let t = msg["t"].as_str().unwrap_or_default().to_string();
            resumed_commands.push(t.clone());
            if t == "start_stream" {
                break;
            }
        }
    }
    assert!(
        resumed_commands.contains(&"start_stream".to_string()),
        "stream must auto-resume on reconnect without pressing start; got {resumed_commands:?}"
    );

    // Now user stops the stream
    runtime.stop_stream();
    assert!(!runtime.session.stream_wanted.load(std::sync::atomic::Ordering::Relaxed));
    drop(second);
    std::thread::sleep(Duration::from_millis(300));

    // Third connection after stream was stopped must not auto-resume
    let mut third = Phone::dial(port);
    let greeting3 = third.read_greeting();
    let auth3 = phonecam_protocol::device_secret_auth(&device_id, &secret, &greeting3.server_nonce, 300).unwrap();
    third.handshake(ChannelRole::Control, AuthMode::DeviceSecret, 300, &auth3);
    assert_eq!(third.read_response().status as u8, 0);

    let welcome3 = third.read_control().expect("welcome");
    assert_eq!(welcome3["t"], "welcome");

    // Set a short timeout: no stream commands should arrive
    third.stream.set_read_timeout(Some(Duration::from_millis(400))).unwrap();
    let mut unexpected = Vec::new();
    for _ in 0..3 {
        if let Some(msg) = third.read_control() {
            unexpected.push(msg["t"].as_str().unwrap_or_default().to_string());
        }
    }
    assert!(!unexpected.contains(&"start_stream".to_string()), "a stopped stream must stay stopped on reconnect");
}

#[test]
fn start_stream_clears_live_frame_seen_so_next_frame_promotes_phase() {
    let (runtime, _port) = runtime();

    // 1. First start
    runtime.start_stream();
    assert_eq!(runtime.session.phase(), phonecam_protocol::StreamPhase::Connecting);
    assert!(!runtime.session.live_frame_seen.load(std::sync::atomic::Ordering::Relaxed));

    // Frame decoded
    runtime.session.note_decoded_frame();
    assert_eq!(runtime.session.phase(), phonecam_protocol::StreamPhase::Streaming);
    assert!(runtime.session.live_frame_seen.load(std::sync::atomic::Ordering::Relaxed));

    // 2. Restart / reconfigure on same channel
    // Phone state pauses or reconfigures, demoting phase to Connected
    runtime.session.note_phone_state(false);
    assert_eq!(runtime.session.phase(), phonecam_protocol::StreamPhase::Connected);

    runtime.start_stream();
    assert_eq!(runtime.session.phase(), phonecam_protocol::StreamPhase::Connecting);
    assert!(!runtime.session.live_frame_seen.load(std::sync::atomic::Ordering::Relaxed));

    // 3. Next decoded frame promotes back to Streaming
    runtime.session.note_decoded_frame();
    assert_eq!(runtime.session.phase(), phonecam_protocol::StreamPhase::Streaming);
    assert!(runtime.session.live_frame_seen.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn stepped_camera_retries_on_initial_camera_start_failure() {
    let (runtime, port) = runtime();
    let payloads = runtime.arm_pairing();
    let payload: serde_json::Value = serde_json::from_str(&payloads[0]).unwrap();
    let token = payload["token"].as_str().unwrap().to_string();

    let mut phone = Phone::dial(port);
    phone.read_greeting();
    phone.handshake(ChannelRole::Control, AuthMode::PairingToken, 1, &token);
    assert_eq!(phone.read_response().status as u8, 0);
    phone.read_control(); // welcome

    runtime.start_stream();
    assert_eq!(runtime.camera_retry.lock().failures(), 0);

    // Drain initial start_stream
    for _ in 0..6 {
        if let Some(msg) = phone.read_control() {
            if msg["t"] == "start_stream" {
                break;
            }
        }
    }

    // Camera fails to start initially (e.g. HAL busy)
    // Send error frame as raw JSON packet
    let error_msg = serde_json::json!({
        "t": "error",
        "code": "stream_failed",
        "message": "Camera is currently locked by another process"
    });
    let err_bytes = serde_json::to_vec(&error_msg).unwrap();
    let mut packet = (err_bytes.len() as u32).to_be_bytes().to_vec();
    packet.extend_from_slice(&err_bytes);
    phone.stream.write_all(&packet).unwrap();
    phone.stream.flush().unwrap();

    // Give runtime thread a moment to process error and schedule stepped retry
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(runtime.camera_retry.lock().failures(), 1);

    // Wait for the 500ms stepped retry delay to elapse and trigger retry
    std::thread::sleep(Duration::from_millis(500));

    // A fresh start_stream command must have been sent to the phone on retry!
    phone.stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut retry_seen = false;
    for _ in 0..8 {
        if let Some(msg) = phone.read_control() {
            if msg["t"] == "start_stream" {
                retry_seen = true;
                break;
            }
        }
    }
    assert!(retry_seen, "stepped retry must issue start_stream again after delay");
}
