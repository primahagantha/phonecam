//! Pairing tokens, device records and nonce replay defence (docs Â§4, Â§5).
//!
//! The rule from PRD Â§36 is that `IP + fixed password` is not acceptable. What
//! satisfies it:
//!
//! 1. **First contact** is either a one-time token from the QR code, or a
//!    request a human approves on the PC. The token is 128 bits from a CSPRNG,
//!    expires in 5 minutes, and is consumed on first use.
//! 2. **Afterwards** the phone holds a 256-bit device secret and proves
//!    possession with `HMAC-SHA256(secret, serverNonce || clientNonce)`.
//! 3. **Replay** is defeated by requiring a strictly increasing client nonce per
//!    device; the phone persists its counter, so a restart cannot replay.

use std::collections::HashMap;

use phonecam_protocol::{secret_fingerprint, to_hex, DEVICE_SECRET_BYTES, PAIRING_TOKEN_BYTES};

/// A phone that has completed pairing (PRD Â§37).
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRecord {
    pub device_id: String,
    pub name: String,
    /// 64 lowercase hex chars. Never logged; `secret_fingerprint` is the only
    /// form that may reach a log line (Â§13).
    pub secret_hex: String,
    pub last_seen_ms: i64,
    pub preferred_camera: String,
    pub preferred_resolution: String,
    pub preferred_fps: u32,
    pub preferred_connection: String,
}

impl DeviceRecord {
    pub fn new(device_id: impl Into<String>, secret_hex: impl Into<String>, now_ms: i64) -> Self {
        Self {
            device_id: device_id.into(),
            name: "Android phone".into(),
            secret_hex: secret_hex.into(),
            last_seen_ms: now_ms,
            preferred_camera: "back".into(),
            preferred_resolution: "1920x1080".into(),
            preferred_fps: 30,
            preferred_connection: "auto".into(),
        }
    }

    /// Safe to log: a short correlator, never the secret.
    pub fn fingerprint(&self) -> String {
        secret_fingerprint(&self.secret_hex)
    }
}

/// An armed one-time token.
#[derive(Debug, Clone, PartialEq)]
struct ArmedToken {
    token: String,
    /// Empty for an unbound QR token; the device id for an approval token.
    device_id: String,
    expires_at_ms: i64,
}

/// A phone awaiting a human approval decision (docs Â§4.3).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRequest {
    pub device_id: String,
    pub name: String,
    pub model: String,
    pub requested_at_ms: i64,
}

/// One-time pairing tokens, approval tokens and replay defence.
#[derive(Debug, Default)]
pub struct PairingManager {
    /// The unbound token from the QR code.
    qr_token: Option<ArmedToken>,
    /// Tokens bound to a specific device after the user approved.
    device_tokens: HashMap<String, ArmedToken>,
    pairing_id: Option<String>,
    expires_at_ms: i64,
    /// Highest accepted client nonce per device.
    last_nonce: HashMap<String, u64>,
    pending: HashMap<String, PendingRequest>,
}

impl PairingManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arms a fresh unbound token (the QR code). `None` when secure random
    /// generation failed â€” the caller must refuse to pair rather than fall back
    /// to something weaker.
    pub fn arm(&mut self, ttl_ms: i64, now_ms: i64) -> Option<String> {
        let token = random_hex(PAIRING_TOKEN_BYTES)?;
        let pairing_id = random_hex(4)?;
        self.qr_token = Some(ArmedToken {
            token: token.clone(),
            device_id: String::new(),
            expires_at_ms: now_ms + ttl_ms,
        });
        self.pairing_id = Some(pairing_id);
        self.expires_at_ms = now_ms + ttl_ms;
        Some(token)
    }

    pub fn disarm(&mut self) {
        self.qr_token = None;
        self.pairing_id = None;
        self.expires_at_ms = 0;
    }

    /// Whether an unexpired token is armed.
    pub fn is_open(&self, now_ms: i64) -> bool {
        self.qr_token
            .as_ref()
            .map(|t| t.expires_at_ms > now_ms)
            .unwrap_or(false)
    }

    pub fn pairing_id(&self) -> Option<&str> {
        self.pairing_id.as_deref()
    }

    pub fn expires_at_ms(&self) -> i64 {
        self.expires_at_ms
    }

    /// Arms a token bound to one device after the user approves (docs Â§4.3).
    pub fn arm_for_device(&mut self, device_id: &str, ttl_ms: i64, now_ms: i64) -> Option<String> {
        let token = random_hex(PAIRING_TOKEN_BYTES)?;
        self.device_tokens.insert(
            device_id.to_string(),
            ArmedToken {
                token: token.clone(),
                device_id: device_id.to_string(),
                expires_at_ms: now_ms + ttl_ms,
            },
        );
        self.pending.remove(device_id);
        Some(token)
    }

    /// The armed token for `device_id`, without consuming it.
    ///
    /// Used to deliver the token over the approval socket; consumption is left
    /// to the re-dial that redeems it, so the token stays one-time without this
    /// read spending it.
    pub fn approved_token_for(&self, device_id: &str, now_ms: i64) -> Option<String> {
        self.device_tokens
            .get(device_id)
            .filter(|t| t.expires_at_ms > now_ms)
            .map(|t| t.token.clone())
    }

    /// Validates a token **without** consuming it, reporting whose it is.
    ///
    /// Exists so the handshake can answer `busy` *before* spending the token: a
    /// `busy` refusal means "wait and retry", so it must not destroy the
    /// credential the retry needs. Expired entries are ignored, not erased,
    /// because a peek must not mutate state.
    pub fn peek(&self, token: &str, now_ms: i64) -> Option<PeekedToken> {
        for (device_id, armed) in &self.device_tokens {
            if armed.token == token && armed.expires_at_ms > now_ms {
                return Some(PeekedToken {
                    device_id: Some(device_id.clone()),
                });
            }
        }
        match &self.qr_token {
            Some(armed) if armed.token == token && armed.expires_at_ms > now_ms => {
                Some(PeekedToken { device_id: None })
            }
            _ => None,
        }
    }

    /// Consumes a token, whichever kind it is, and reports whose it was.
    ///
    /// A QR token is unbound, so `device_id` is `None` and the caller mints a
    /// new device id. An approval token is bound to the device that asked, and
    /// that id must be reused: the phone already stored it and the secret being
    /// minted is keyed by it.
    pub fn consume(&mut self, token: &str, now_ms: i64) -> Option<PeekedToken> {
        let bound = self
            .device_tokens
            .iter()
            .find(|(_, armed)| armed.token == token)
            .map(|(id, _)| id.clone());

        if let Some(device_id) = bound {
            let expired = self
                .device_tokens
                .get(&device_id)
                .map(|t| t.expires_at_ms <= now_ms)
                .unwrap_or(true);
            // Consumed either way: an expired token must not stay redeemable.
            self.device_tokens.remove(&device_id);
            if expired {
                return None;
            }
            return Some(PeekedToken {
                device_id: Some(device_id),
            });
        }

        match self.qr_token.take() {
            Some(armed) if armed.token == token && armed.expires_at_ms > now_ms => {
                self.pairing_id = None;
                Some(PeekedToken { device_id: None })
            }
            _ => None,
        }
    }

    /// Rejects a replayed client nonce for `device_id` (docs Â§4.4).
    ///
    /// The nonce must strictly increase. Equal is a replay.
    pub fn accept_nonce(&mut self, device_id: &str, nonce: u64) -> bool {
        match self.last_nonce.get(device_id) {
            Some(&last) if nonce <= last => false,
            _ => {
                self.last_nonce.insert(device_id.to_string(), nonce);
                true
            }
        }
    }

    pub fn add_pending_request(&mut self, request: PendingRequest) {
        self.pending.insert(request.device_id.clone(), request);
    }

    pub fn has_pending_request(&self, device_id: &str) -> bool {
        self.pending.contains_key(device_id)
    }

    pub fn take_pending_request(&mut self, device_id: &str) -> Option<PendingRequest> {
        self.pending.remove(device_id)
    }

    pub fn pending_requests(&self) -> impl Iterator<Item = &PendingRequest> {
        self.pending.values()
    }

    pub fn forget_device(&mut self, device_id: &str) {
        self.device_tokens.remove(device_id);
        self.last_nonce.remove(device_id);
        self.pending.remove(device_id);
    }
}

/// The result of validating a token.
#[derive(Debug, Clone, PartialEq)]
pub struct PeekedToken {
    /// `Some` for an approval token bound to a device, `None` for a QR token.
    pub device_id: Option<String>,
}

/// 128 bits of hex from the OS CSPRNG.
///
/// `None` rather than a weak fallback: a predictable pairing token is worse than
/// a pairing that fails.
pub fn random_hex(bytes: usize) -> Option<String> {
    use rand::RngExt as _;
    let mut buffer = vec![0u8; bytes];
    rand::rng().fill(&mut buffer[..]);
    Some(to_hex(&buffer))
}

/// A fresh 32-byte device secret, hex-encoded.
pub fn new_device_secret() -> Option<String> {
    random_hex(DEVICE_SECRET_BYTES)
}

/// A fresh device id, e.g. `android-a31f92`.
pub fn new_device_id() -> Option<String> {
    random_hex(4).map(|suffix| format!("android-{suffix}"))
}

/// A fresh server id, e.g. `pc-7f3c1a9e`.
pub fn new_server_id() -> Option<String> {
    random_hex(4).map(|suffix| format!("pc-{suffix}"))
}

/// A 32-byte nonce for the greeting.
pub fn new_server_nonce() -> Option<Vec<u8>> {
    use rand::RngExt as _;
    let mut nonce = vec![0u8; 32];
    rand::rng().fill(&mut nonce[..]);
    Some(nonce)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: i64 = 300_000;

    #[test]
    fn a_token_is_32_lowercase_hex_chars() {
        let token = random_hex(PAIRING_TOKEN_BYTES).unwrap();
        assert_eq!(token.len(), 32);
        assert!(token
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn tokens_do_not_repeat() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(random_hex(16).unwrap()));
        }
    }

    #[test]
    fn a_device_secret_is_64_hex_chars() {
        let secret = new_device_secret().unwrap();
        assert_eq!(secret.len(), 64);
        assert_eq!(secret_fingerprint(&secret).len(), 8);
    }

    #[test]
    fn a_qr_token_pairs_once_then_is_spent() {
        let mut pairing = PairingManager::new();
        let token = pairing.arm(TTL, 1_000).unwrap();
        assert!(pairing.is_open(1_000));

        let peeked = pairing.peek(&token, 1_000).unwrap();
        assert_eq!(peeked.device_id, None, "a QR token is unbound");

        let consumed = pairing.consume(&token, 1_000).unwrap();
        assert_eq!(consumed.device_id, None);

        // Single use: the second attempt fails.
        assert!(pairing.consume(&token, 1_000).is_none());
        assert!(!pairing.is_open(1_000));
    }

    #[test]
    fn a_token_expires() {
        let mut pairing = PairingManager::new();
        let token = pairing.arm(TTL, 0).unwrap();
        assert!(!pairing.is_open(TTL + 1));
        assert!(pairing.peek(&token, TTL + 1).is_none());
        assert!(pairing.consume(&token, TTL + 1).is_none());
    }

    #[test]
    fn peeking_does_not_spend_the_token() {
        let mut pairing = PairingManager::new();
        let token = pairing.arm(TTL, 0).unwrap();
        // This is the `busy` path: peek, refuse, let the phone retry.
        for _ in 0..5 {
            assert!(pairing.peek(&token, 100).is_some());
        }
        assert!(
            pairing.consume(&token, 100).is_some(),
            "the token must survive peeking"
        );
    }

    #[test]
    fn an_approval_token_is_bound_to_its_device() {
        let mut pairing = PairingManager::new();
        pairing.add_pending_request(PendingRequest {
            device_id: "android-a31f92".into(),
            name: "Android phone".into(),
            model: "Android".into(),
            requested_at_ms: 0,
        });
        let token = pairing
            .arm_for_device("android-a31f92", 120_000, 0)
            .unwrap();

        // Arming clears the pending request: the decision has been made.
        assert!(!pairing.has_pending_request("android-a31f92"));

        let peeked = pairing.peek(&token, 0).unwrap();
        assert_eq!(peeked.device_id.as_deref(), Some("android-a31f92"));

        let consumed = pairing.consume(&token, 0).unwrap();
        assert_eq!(
            consumed.device_id.as_deref(),
            Some("android-a31f92"),
            "the phone already stored this id and the secret is keyed by it"
        );
    }

    #[test]
    fn approved_token_for_does_not_consume() {
        let mut pairing = PairingManager::new();
        let token = pairing.arm_for_device("d1", 120_000, 0).unwrap();
        assert_eq!(pairing.approved_token_for("d1", 10), Some(token.clone()));
        // Still redeemable by the re-dial.
        assert!(pairing.consume(&token, 10).is_some());
    }

    #[test]
    fn an_expired_approval_token_is_not_redeemable() {
        let mut pairing = PairingManager::new();
        let token = pairing.arm_for_device("d1", 120_000, 0).unwrap();
        assert!(pairing.approved_token_for("d1", 120_001).is_none());
        assert!(pairing.consume(&token, 120_001).is_none());
    }

    #[test]
    fn an_unknown_token_is_refused() {
        let mut pairing = PairingManager::new();
        pairing.arm(TTL, 0).unwrap();
        assert!(pairing
            .consume("00000000000000000000000000000000", 0)
            .is_none());
        assert!(pairing.peek("not-a-token", 0).is_none());
    }

    #[test]
    fn nonces_must_strictly_increase() {
        let mut pairing = PairingManager::new();
        assert!(pairing.accept_nonce("d1", 100));
        assert!(pairing.accept_nonce("d1", 101));
        // Equal is a replay.
        assert!(!pairing.accept_nonce("d1", 101));
        // Lower is a replay.
        assert!(!pairing.accept_nonce("d1", 50));
        // A different device has its own counter.
        assert!(pairing.accept_nonce("d2", 1));
    }

    #[test]
    fn forgetting_a_device_clears_its_nonce_so_it_can_pair_again() {
        let mut pairing = PairingManager::new();
        assert!(pairing.accept_nonce("d1", 1000));
        pairing.forget_device("d1");
        // Without clearing the nonce a re-paired phone would be refused as a
        // replay for using a lower counter.
        assert!(pairing.accept_nonce("d1", 1));
    }

    #[test]
    fn disarming_closes_the_qr_but_leaves_device_tokens_alone() {
        let mut pairing = PairingManager::new();
        let qr = pairing.arm(TTL, 0).unwrap();
        let bound = pairing.arm_for_device("d1", TTL, 0).unwrap();
        pairing.disarm();
        assert!(!pairing.is_open(0));
        assert!(pairing.consume(&qr, 0).is_none());
        // An approval already granted must still be redeemable.
        assert!(pairing.consume(&bound, 0).is_some());
    }

    #[test]
    fn ids_have_the_expected_shape() {
        assert!(new_device_id().unwrap().starts_with("android-"));
        assert!(new_server_id().unwrap().starts_with("pc-"));
        assert_eq!(new_server_nonce().unwrap().len(), 32);
    }
}
