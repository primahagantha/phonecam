use phonecam_core::session::{
    CameraRetryTracker, SessionState, CAMERA_RETRY_DELAYS_MS, MAX_CAMERA_START_FAILURES,
};
use phonecam_protocol::StreamPhase;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[test]
fn test_stream_wanted_lifecycle() {
    let session = SessionState::default();
    assert!(!session.stream_wanted.load(Ordering::Relaxed));

    // User starts stream: stream_wanted becomes true
    session.note_start_stream();
    assert!(session.stream_wanted.load(Ordering::Relaxed));
    assert_eq!(session.phase(), StreamPhase::Connecting);

    // Control channel drops: stream_wanted remains true so reconnect can auto-resume
    session.note_control_closed();
    assert!(session.stream_wanted.load(Ordering::Relaxed));
    assert_eq!(session.phase(), StreamPhase::Reconnecting);

    // User stops stream: stream_wanted is cleared
    session.note_stop_stream();
    assert!(!session.stream_wanted.load(Ordering::Relaxed));
}

#[test]
fn test_live_frame_seen_promotes_phase_after_restart() {
    let session = SessionState::default();

    // 1. First stream attempt
    session.note_start_stream();
    assert_eq!(session.phase(), StreamPhase::Connecting);
    assert!(!session.live_frame_seen.load(Ordering::Relaxed));

    // First frame decodes: promotes from connecting to streaming
    session.note_decoded_frame();
    assert_eq!(session.phase(), StreamPhase::Streaming);
    assert!(session.live_frame_seen.load(Ordering::Relaxed));

    // 2. Stream restart / reconfigure on same channel
    // Phone pauses or reconfigures, moving phase to Connected
    session.note_phone_state(false);
    assert_eq!(session.phase(), StreamPhase::Connected);

    session.note_start_stream();
    // Clearing live_frame_seen ensures the next decoded frame does the promotion
    assert!(!session.live_frame_seen.load(Ordering::Relaxed));
    assert_eq!(session.phase(), StreamPhase::Connecting);

    // 3. Next decoded frame promotes back to streaming
    session.note_decoded_frame();
    assert_eq!(session.phase(), StreamPhase::Streaming);
    assert!(session.live_frame_seen.load(Ordering::Relaxed));
}

#[test]
fn test_stepped_camera_retry_delays_and_exhaustion() {
    let mut tracker = CameraRetryTracker::new();
    assert_eq!(tracker.failures(), 0);
    assert_eq!(tracker.max_failures(), MAX_CAMERA_START_FAILURES);

    let expected_delays = [
        Duration::from_millis(CAMERA_RETRY_DELAYS_MS[0]),
        Duration::from_millis(CAMERA_RETRY_DELAYS_MS[1]),
        Duration::from_millis(CAMERA_RETRY_DELAYS_MS[2]),
        Duration::from_millis(CAMERA_RETRY_DELAYS_MS[3]),
    ];

    for (i, expected) in expected_delays.iter().enumerate() {
        assert!(tracker.can_retry());
        let delay = tracker.record_failure();
        assert_eq!(delay, Some(*expected), "mismatch at attempt {i}");
        assert_eq!(tracker.failures(), i + 1);
    }

    // After 4 failures, cannot retry
    assert!(!tracker.can_retry());
    assert_eq!(tracker.record_failure(), None);
    assert_eq!(tracker.failures(), MAX_CAMERA_START_FAILURES);

    // Successful stream reset
    tracker.reset();
    assert_eq!(tracker.failures(), 0);
    assert!(tracker.can_retry());
}
