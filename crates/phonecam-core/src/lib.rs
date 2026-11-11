//! PhoneCam core: transport, authentication, decode, the frame bus, the
//! adaptive ladder and adb â€” everything except the UI and the virtual camera
//! DLL.
//!
//! The desktop is the server and the phone is the client (docs Â§2). Electron
//! used to be a separate process talking to a C++ service over a loopback
//! socket; here the core is a library inside the app, so there is one process,
//! one decode, and no IPC hop between "the thing that decodes" and "the thing
//! that draws".

pub mod adaptive_quality;
pub mod pairing;
pub mod session;
pub mod transport;

pub use adaptive_quality::{
    AdaptiveQuality, QualityAction, QualityDecision, QualityRung, QualitySample,
    BAD_DROP_PERCENT, CLEAN_DROP_PERCENT, CLEAN_RTT_US, SLOW_RTT_US, STEP_DOWN_DWELL_MS,
    STEP_UP_DWELL_MS,
};
pub use pairing::{DeviceRecord, PairingManager, PendingRequest};
pub use session::{
    Authenticator, HandshakeOutcome, SessionState, StatsWindow, APPROVAL_TOKEN_TTL_MS,
    APPROVAL_WAIT_MS,
};
pub use transport::{Connection, DiscoverySocket, Listener, ReadResult, TransportError};

