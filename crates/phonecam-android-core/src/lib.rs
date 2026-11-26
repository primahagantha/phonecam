//! JNI bridge exposing PhoneCam's Rust protocol, framing, and QR/discovery
//! parsing to the Android Kotlin app and JVM unit tests.

use std::ptr;

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jbyteArray, jint, jintArray, jlong, jlongArray, jstring};
use jni::JNIEnv;
use phonecam_core::pairing::random_hex as core_random_hex;
use phonecam_core::session::MAX_HANDSHAKE_AUTH;
use phonecam_protocol::{
    device_secret_auth, pair_request_auth, secret_fingerprint, DesktopBeacon, HandshakeStatus,
    QrPayload, ServerGreeting, CLIENT_HANDSHAKE_SIZE, MAGIC, MAGIC_BYTES, MAX_CONTROL_JSON,
    MAX_VIDEO_PAYLOAD, PROTOCOL_VERSION, SERVER_GREETING_SIZE, SERVER_RESPONSE_SIZE, SERVICE_PORT,
    VIDEO_HEADER_SIZE,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const NATIVE_VERSION: &str = concat!(
    "phonecam-android-core/",
    env!("CARGO_PKG_VERSION"),
    " (protocol v1)"
);

/// Typed failure mapped onto the exact Java exception expected by callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeError {
    Io(String),
    IllegalArgument(String),
    IndexOutOfBounds(String),
}

impl NativeError {
    fn throw(self, env: &mut JNIEnv<'_>) {
        let (class, msg) = match self {
            Self::Io(m) => ("java/io/IOException", m),
            Self::IllegalArgument(m) => ("java/lang/IllegalArgumentException", m),
            Self::IndexOutOfBounds(m) => ("java/lang/IndexOutOfBoundsException", m),
        };
        if !env.exception_check().unwrap_or(false) {
            let _ = env.throw_new(class, msg);
        }
    }
}

/* ------------------------------------------------------------------ */
/* Pure Rust core helpers (unit-tested without JNI)                    */
/* ------------------------------------------------------------------ */

pub fn generate_random_hex(byte_len: i32) -> Result<String, NativeError> {
    if byte_len < 0 {
        return Err(NativeError::IllegalArgument(format!(
            "byteLen must be non-negative, got {byte_len}"
        )));
    }
    core_random_hex(byte_len as usize)
        .ok_or_else(|| NativeError::IllegalArgument("random generation failed".into()))
}

pub fn compute_device_secret_auth(
    device_id: &str,
    secret_hex: &str,
    server_nonce: &[u8],
    client_nonce: i64,
) -> Result<String, NativeError> {
    device_secret_auth(device_id, secret_hex, server_nonce, client_nonce as u64)
        .map_err(|_| NativeError::IllegalArgument("invalid device secret hex".into()))
}

pub fn compute_pair_request_auth(
    device_id: &str,
    secret_hex: Option<&str>,
    server_nonce: &[u8],
    client_nonce: i64,
) -> Result<String, NativeError> {
    pair_request_auth(device_id, secret_hex, server_nonce, client_nonce as u64)
        .map_err(|_| NativeError::IllegalArgument("invalid device secret hex".into()))
}

/// Decodes a 40-byte server greeting into `[version, status, serverNonce[0..32]]`.
pub fn decode_server_greeting(bytes: &[u8]) -> Result<[u8; 34], NativeError> {
    if bytes.len() != SERVER_GREETING_SIZE {
        return Err(NativeError::Io(format!(
            "not a PhoneCam peer (greeting length={}, expected {})",
            bytes.len(),
            SERVER_GREETING_SIZE
        )));
    }
    let greeting = ServerGreeting::parse(bytes).map_err(|_| {
        let magic = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        NativeError::Io(format!("not a PhoneCam peer (magic=0x{magic:08X})"))
    })?;
    let mut out = [0u8; 34];
    out[0] = greeting.proto_version;
    out[1] = greeting.status as u8;
    out[2..34].copy_from_slice(&greeting.server_nonce);
    Ok(out)
}

/// Encodes a 20-byte big-endian client handshake header followed by `auth_bytes`.
pub fn encode_client_handshake(
    role: i32,
    auth_mode: i32,
    client_nonce: i64,
    auth_bytes: &[u8],
) -> Result<Vec<u8>, NativeError> {
    if auth_bytes.len() > MAX_HANDSHAKE_AUTH {
        return Err(NativeError::IllegalArgument(format!(
            "auth payload too long: {} > {}",
            auth_bytes.len(),
            MAX_HANDSHAKE_AUTH
        )));
    }
    let role_byte = u8::try_from(role)
        .map_err(|_| NativeError::IllegalArgument(format!("role out of range: {role}")))?;
    let auth_mode_byte = u8::try_from(auth_mode)
        .map_err(|_| NativeError::IllegalArgument(format!("authMode out of range: {auth_mode}")))?;

    let mut out = Vec::with_capacity(CLIENT_HANDSHAKE_SIZE + auth_bytes.len());
    out.extend_from_slice(&MAGIC_BYTES);
    out.push(PROTOCOL_VERSION);
    out.push(role_byte);
    out.push(auth_mode_byte);
    out.push(0); // flags
    out.extend_from_slice(&(client_nonce as u64).to_be_bytes());
    out.extend_from_slice(&(auth_bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // reserved
    out.extend_from_slice(auth_bytes);
    Ok(out)
}

/// Decodes a 24-byte server response header into `[version, status, role, jsonLen]`.
pub fn decode_server_response_header(header24: &[u8]) -> Result<[i32; 4], NativeError> {
    if header24.len() != SERVER_RESPONSE_SIZE {
        return Err(NativeError::Io(format!(
            "server response header must be {} bytes, got {}",
            SERVER_RESPONSE_SIZE,
            header24.len()
        )));
    }
    let magic = u32::from_be_bytes([header24[0], header24[1], header24[2], header24[3]]);
    if magic != MAGIC {
        return Err(NativeError::Io(format!(
            "bad magic in server response (0x{magic:08X})"
        )));
    }
    let version = header24[4] as i32;
    let status = HandshakeStatus::from_wire(header24[5]) as i32;
    let role = header24[6] as i32;
    let json_len_signed =
        i32::from_be_bytes([header24[16], header24[17], header24[18], header24[19]]);
    if json_len_signed < 0 || (json_len_signed as usize) > MAX_CONTROL_JSON {
        return Err(NativeError::Io(format!(
            "server response json length out of range: {json_len_signed}"
        )));
    }
    Ok([version, status, role, json_len_signed])
}

fn build_video_header_frame(
    frame_id: i64,
    pts_us: i64,
    flags: i32,
    codec: i32,
    payload_len: i32,
) -> Result<[u8; 4 + VIDEO_HEADER_SIZE], NativeError> {
    if payload_len < 0 || (payload_len as usize) > MAX_VIDEO_PAYLOAD {
        return Err(NativeError::Io(format!(
            "video payload length out of range: {payload_len}"
        )));
    }
    let total_len = (VIDEO_HEADER_SIZE + payload_len as usize) as u32;
    let mut out = [0u8; 4 + VIDEO_HEADER_SIZE];
    out[0..4].copy_from_slice(&total_len.to_be_bytes());
    out[4..8].copy_from_slice(&(frame_id as u32).to_be_bytes());
    out[8..16].copy_from_slice(&pts_us.to_be_bytes());
    out[16] = flags as u8;
    out[17] = codec as u8;
    // out[18..20] reserved = 0
    out[20..24].copy_from_slice(&(payload_len as u32).to_be_bytes());
    // out[24..28] reserved = 0
    Ok(out)
}

pub fn encode_video_packet_header(
    frame_id: i64,
    pts_us: i64,
    flags: i32,
    codec: i32,
    payload_len: i32,
) -> Result<[u8; 28], NativeError> {
    build_video_header_frame(frame_id, pts_us, flags, codec, payload_len)
}

/// Writes the 28-byte video packet prefix (`u32 totalLen` + 24-byte header) directly into
/// `dst[offset..offset + 28]` without allocating.
pub fn encode_video_packet_header_into(
    dst: &mut [u8],
    offset: i32,
    frame_id: i64,
    pts_us: i64,
    flags: i32,
    codec: i32,
    payload_len: i32,
) -> Result<(), NativeError> {
    const PREFIX_LEN: usize = 4 + VIDEO_HEADER_SIZE;
    if offset < 0 {
        return Err(NativeError::IndexOutOfBounds(format!(
            "negative destination offset: {offset}"
        )));
    }
    let start = offset as usize;
    let end = start
        .checked_add(PREFIX_LEN)
        .ok_or_else(|| NativeError::IndexOutOfBounds("destination offset overflow".into()))?;
    if end > dst.len() {
        return Err(NativeError::IndexOutOfBounds(format!(
            "destination slice {start}..{end} exceeds length {}",
            dst.len()
        )));
    }
    let header = build_video_header_frame(frame_id, pts_us, flags, codec, payload_len)?;
    dst[start..end].copy_from_slice(&header);
    Ok(())
}

pub fn encode_video_packet(
    frame_id: i64,
    pts_us: i64,
    flags: i32,
    codec: i32,
    payload: &[u8],
    offset: i32,
    length: i32,
) -> Result<Vec<u8>, NativeError> {
    if offset < 0 || length < 0 {
        return Err(NativeError::IndexOutOfBounds(format!(
            "invalid offset/length: offset={offset}, length={length}"
        )));
    }
    let start = offset as usize;
    let len = length as usize;
    let end = start
        .checked_add(len)
        .ok_or_else(|| NativeError::IndexOutOfBounds("offset + length overflow".into()))?;
    if end > payload.len() {
        return Err(NativeError::IndexOutOfBounds(format!(
            "slice {start}..{end} exceeds payload length {}",
            payload.len()
        )));
    }
    let header = build_video_header_frame(frame_id, pts_us, flags, codec, length)?;
    let mut out = Vec::with_capacity(header.len() + len);
    out.extend_from_slice(&header);
    out.extend_from_slice(&payload[start..end]);
    Ok(out)
}

/// Decodes a 24-byte big-endian video header into `[frameId, ptsUs, flags, codec, payloadLen]`.
pub fn decode_video_packet_header(header24: &[u8]) -> Result<[i64; 5], NativeError> {
    if header24.len() != VIDEO_HEADER_SIZE {
        return Err(NativeError::Io(format!(
            "video header must be {} bytes, got {}",
            VIDEO_HEADER_SIZE,
            header24.len()
        )));
    }
    let frame_id = u32::from_be_bytes([header24[0], header24[1], header24[2], header24[3]]) as i64;
    let pts_us = i64::from_be_bytes(header24[4..12].try_into().unwrap());
    let flags = header24[12] as i64;
    let codec = header24[13] as i64;
    let payload_len =
        u32::from_be_bytes([header24[16], header24[17], header24[18], header24[19]]) as i64;
    Ok([frame_id, pts_us, flags, codec, payload_len])
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NormalizedQrPayload {
    protocol: String,
    version: u8,
    host: String,
    port: u16,
    token: String,
    name: Option<String>,
    server_id: Option<String>,
}

fn optional_non_blank_string(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> Option<Option<String>> {
    match obj.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(s)) => {
            if s.trim().is_empty() {
                Some(None)
            } else {
                Some(Some(s.clone()))
            }
        }
        Some(_) => None,
    }
}

/// Validates a scanned QR payload and returns a normalized JSON string, or `None` if invalid.
pub fn validate_qr_payload(raw: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(raw.trim()).ok()?;
    let obj = parsed.as_object()?;

    let protocol = obj.get("protocol")?.as_str()?.to_string();
    let version_u64 = obj.get("version")?.as_u64()?;
    let version = u8::try_from(version_u64).ok()?;
    let host = obj.get("host")?.as_str()?.to_string();
    if host.trim().is_empty() {
        return None;
    }
    let port_u64 = obj.get("port")?.as_u64()?;
    let port = u16::try_from(port_u64).ok()?;
    let token = obj.get("token")?.as_str()?.to_string();
    let name = optional_non_blank_string(obj, "name")?;
    let server_id = optional_non_blank_string(obj, "serverId")?;

    let proto_qr = QrPayload {
        protocol: protocol.clone(),
        version,
        host: host.clone(),
        port,
        token: token.clone(),
        name: name.clone().unwrap_or_default(),
        server_id: server_id.clone().unwrap_or_default(),
    };
    if !proto_qr.is_well_formed() {
        return None;
    }

    let normalized = NormalizedQrPayload {
        protocol,
        version,
        host,
        port,
        token,
        name,
        server_id,
    };
    serde_json::to_string(&normalized).ok()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDesktopBeacon {
    t: String,
    v: u8,
    role: String,
    server_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    pairing_open: bool,
    #[serde(default)]
    pairing_id: Option<String>,
    #[serde(default)]
    paired_device_ids: Vec<Value>,
    #[serde(default)]
    busy: bool,
    #[serde(default)]
    version: Option<String>,
}

/// Validates a PhoneCam UDP desktop discovery beacon and returns normalized `DesktopBeacon` JSON.
pub fn parse_discovery_beacon(json_bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(json_bytes).ok()?.trim();
    let raw: RawDesktopBeacon = serde_json::from_str(text).ok()?;
    if raw.t != "phonecam.beacon"
        || raw.v != PROTOCOL_VERSION
        || raw.role != "desktop"
        || raw.server_id.trim().is_empty()
    {
        return None;
    }
    let port = raw.port.unwrap_or(SERVICE_PORT);
    if port == 0 {
        return None;
    }
    let name = raw
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "PhoneCam Desktop".to_string());
    let paired_device_ids: Vec<String> = raw
        .paired_device_ids
        .into_iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s),
            _ => None,
        })
        .collect();

    let beacon = DesktopBeacon {
        t: raw.t,
        v: raw.v,
        role: raw.role,
        server_id: raw.server_id,
        name,
        host: raw.host.unwrap_or_default(),
        port,
        pairing_open: raw.pairing_open,
        pairing_id: raw.pairing_id.filter(|s| !s.trim().is_empty()),
        paired_device_ids,
        busy: raw.busy,
        version: raw.version.unwrap_or_default(),
    };
    serde_json::to_string(&beacon).ok()
}

/* ------------------------------------------------------------------ */
/* JNI utilities                                                       */
/* ------------------------------------------------------------------ */

fn read_jstring(env: &mut JNIEnv<'_>, s: &JString<'_>, name: &str) -> Result<String, NativeError> {
    if s.is_null() {
        return Err(NativeError::IllegalArgument(format!(
            "{name} must not be null"
        )));
    }
    env.get_string(s)
        .map(String::from)
        .map_err(|e| NativeError::IllegalArgument(format!("failed to read {name}: {e}")))
}

fn read_optional_jstring(
    env: &mut JNIEnv<'_>,
    s: &JString<'_>,
    name: &str,
) -> Result<Option<String>, NativeError> {
    if s.is_null() {
        return Ok(None);
    }
    read_jstring(env, s, name).map(Some)
}

fn read_jbyte_array(
    env: &mut JNIEnv<'_>,
    arr: &JByteArray<'_>,
    name: &str,
) -> Result<Vec<u8>, NativeError> {
    if arr.is_null() {
        return Err(NativeError::IllegalArgument(format!(
            "{name} must not be null"
        )));
    }
    env.convert_byte_array(arr)
        .map_err(|e| NativeError::Io(format!("failed to read {name}: {e}")))
}

fn read_exact_jbyte_array<const N: usize>(
    env: &mut JNIEnv<'_>,
    arr: &JByteArray<'_>,
    null_err: impl FnOnce() -> NativeError,
    len_err: impl FnOnce(i32) -> NativeError,
) -> Result<[u8; N], NativeError> {
    if arr.is_null() {
        return Err(null_err());
    }
    let len = env
        .get_array_length(arr)
        .map_err(|e| NativeError::Io(format!("failed to inspect byte array: {e}")))?;
    if len != N as i32 {
        return Err(len_err(len));
    }
    let mut buf = [0i8; N];
    env.get_byte_array_region(arr, 0, &mut buf)
        .map_err(|e| NativeError::Io(format!("failed to copy byte array: {e}")))?;
    Ok(buf.map(|b| b as u8))
}

fn to_jstring(env: &mut JNIEnv<'_>, value: &str) -> jstring {
    env.new_string(value)
        .map(|s| s.into_raw())
        .unwrap_or(ptr::null_mut())
}

fn to_jbyte_array(env: &mut JNIEnv<'_>, bytes: &[u8]) -> jbyteArray {
    env.byte_array_from_slice(bytes)
        .map(|a| a.into_raw())
        .unwrap_or(ptr::null_mut())
}

/* ------------------------------------------------------------------ */
/* JNI entry points: com.phonecam.camera.nativecore.NativeCore         */
/* ------------------------------------------------------------------ */

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_nativeVersion<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
) -> jstring {
    to_jstring(&mut env, NATIVE_VERSION)
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_randomHex<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    byte_len: jint,
) -> jstring {
    match generate_random_hex(byte_len) {
        Ok(hex) => to_jstring(&mut env, &hex),
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_secretFingerprint<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    secret_hex: JString<'local>,
) -> jstring {
    let secret = match read_jstring(&mut env, &secret_hex, "secretHex") {
        Ok(v) => v,
        Err(err) => {
            err.throw(&mut env);
            return ptr::null_mut();
        }
    };
    let fp = secret_fingerprint(&secret);
    to_jstring(&mut env, &fp)
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_deviceSecretAuth<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    device_id: JString<'local>,
    secret_hex: JString<'local>,
    server_nonce: JByteArray<'local>,
    client_nonce: jlong,
) -> jstring {
    let res = (|| {
        let dev = read_jstring(&mut env, &device_id, "deviceId")?;
        let sec = read_jstring(&mut env, &secret_hex, "secretHex")?;
        let nonce = read_jbyte_array(&mut env, &server_nonce, "serverNonce")?;
        compute_device_secret_auth(&dev, &sec, &nonce, client_nonce)
    })();
    match res {
        Ok(auth) => to_jstring(&mut env, &auth),
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_pairRequestAuth<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    device_id: JString<'local>,
    secret_hex: JString<'local>,
    server_nonce: JByteArray<'local>,
    client_nonce: jlong,
) -> jstring {
    let res = (|| {
        let dev = read_jstring(&mut env, &device_id, "deviceId")?;
        let sec = read_optional_jstring(&mut env, &secret_hex, "secretHex")?;
        let nonce = read_jbyte_array(&mut env, &server_nonce, "serverNonce")?;
        compute_pair_request_auth(&dev, sec.as_deref(), &nonce, client_nonce)
    })();
    match res {
        Ok(auth) => to_jstring(&mut env, &auth),
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_decodeServerGreeting<
    'local,
>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    bytes: JByteArray<'local>,
) -> jbyteArray {
    let res = (|| {
        let raw = read_exact_jbyte_array::<SERVER_GREETING_SIZE>(
            &mut env,
            &bytes,
            || NativeError::Io("not a PhoneCam peer (greeting is null)".into()),
            |len| {
                NativeError::Io(format!(
                    "not a PhoneCam peer (greeting length={len}, expected {SERVER_GREETING_SIZE})"
                ))
            },
        )?;
        decode_server_greeting(&raw)
    })();
    match res {
        Ok(decoded) => to_jbyte_array(&mut env, &decoded),
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_encodeClientHandshake<
    'local,
>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    role: jint,
    auth_mode: jint,
    client_nonce: jlong,
    auth_bytes: JByteArray<'local>,
) -> jbyteArray {
    let res = (|| {
        let auth = read_jbyte_array(&mut env, &auth_bytes, "authBytes")?;
        encode_client_handshake(role, auth_mode, client_nonce, &auth)
    })();
    match res {
        Ok(encoded) => to_jbyte_array(&mut env, &encoded),
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_decodeServerResponseHeader<
    'local,
>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    header24: JByteArray<'local>,
) -> jintArray {
    let res = (|| {
        let raw = read_exact_jbyte_array::<SERVER_RESPONSE_SIZE>(
            &mut env,
            &header24,
            || NativeError::Io("server response header is null".into()),
            |len| {
                NativeError::Io(format!(
                    "server response header must be {SERVER_RESPONSE_SIZE} bytes, got {len}"
                ))
            },
        )?;
        decode_server_response_header(&raw)
    })();
    match res {
        Ok(vals) => {
            let Ok(arr) = env.new_int_array(4) else {
                return ptr::null_mut();
            };
            if env.set_int_array_region(&arr, 0, &vals).is_err() {
                return ptr::null_mut();
            }
            arr.into_raw()
        }
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_encodeVideoPacketHeader<
    'local,
>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    frame_id: jlong,
    pts_us: jlong,
    flags: jint,
    codec: jint,
    payload_len: jint,
) -> jbyteArray {
    match encode_video_packet_header(frame_id, pts_us, flags, codec, payload_len) {
        Ok(bytes) => to_jbyte_array(&mut env, &bytes),
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_encodeVideoPacketHeaderInto<
    'local,
>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    dst: JByteArray<'local>,
    offset: jint,
    frame_id: jlong,
    pts_us: jlong,
    flags: jint,
    codec: jint,
    payload_len: jint,
) {
    let res = (|| {
        if dst.is_null() {
            return Err(NativeError::IllegalArgument("dst must not be null".into()));
        }
        if offset < 0 {
            return Err(NativeError::IndexOutOfBounds(format!(
                "negative destination offset: {offset}"
            )));
        }
        let dst_len = env
            .get_array_length(&dst)
            .map_err(|e| NativeError::Io(format!("failed to read dst length: {e}")))?;
        let start = offset as usize;
        let end = start
            .checked_add(4 + VIDEO_HEADER_SIZE)
            .ok_or_else(|| NativeError::IndexOutOfBounds("destination offset overflow".into()))?;
        if dst_len < 0 || end > dst_len as usize {
            return Err(NativeError::IndexOutOfBounds(format!(
                "destination slice {start}..{end} exceeds length {dst_len}"
            )));
        }
        let bytes = encode_video_packet_header(frame_id, pts_us, flags, codec, payload_len)?;
        let jbytes: [i8; 4 + VIDEO_HEADER_SIZE] = bytes.map(|b| b as i8);
        env.set_byte_array_region(&dst, offset, &jbytes)
            .map_err(|e| NativeError::Io(format!("failed to write header into dst: {e}")))?;
        Ok(())
    })();
    if let Err(err) = res {
        err.throw(&mut env);
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_encodeVideoPacket<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    frame_id: jlong,
    pts_us: jlong,
    flags: jint,
    codec: jint,
    payload: JByteArray<'local>,
    offset: jint,
    length: jint,
) -> jbyteArray {
    let res = (|| {
        let data = read_jbyte_array(&mut env, &payload, "payload")?;
        encode_video_packet(frame_id, pts_us, flags, codec, &data, offset, length)
    })();
    match res {
        Ok(packet) => to_jbyte_array(&mut env, &packet),
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_decodeVideoPacketHeader<
    'local,
>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    header24: JByteArray<'local>,
) -> jlongArray {
    let res = (|| {
        let raw = read_exact_jbyte_array::<VIDEO_HEADER_SIZE>(
            &mut env,
            &header24,
            || NativeError::Io("video header is null".into()),
            |len| {
                NativeError::Io(format!(
                    "video header must be {VIDEO_HEADER_SIZE} bytes, got {len}"
                ))
            },
        )?;
        decode_video_packet_header(&raw)
    })();
    match res {
        Ok(vals) => {
            let Ok(arr) = env.new_long_array(5) else {
                return ptr::null_mut();
            };
            if env.set_long_array_region(&arr, 0, &vals).is_err() {
                return ptr::null_mut();
            }
            arr.into_raw()
        }
        Err(err) => {
            err.throw(&mut env);
            ptr::null_mut()
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_validateQrPayload<'local>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    raw: JString<'local>,
) -> jstring {
    let Ok(Some(text)) = read_optional_jstring(&mut env, &raw, "raw") else {
        return ptr::null_mut();
    };
    match validate_qr_payload(&text) {
        Some(json) => to_jstring(&mut env, &json),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_phonecam_camera_nativecore_NativeCore_parseDiscoveryBeacon<
    'local,
>(
    mut env: JNIEnv<'local>,
    _cls: JClass<'local>,
    json_bytes: JByteArray<'local>,
) -> jstring {
    if json_bytes.is_null() {
        return ptr::null_mut();
    }
    let Ok(raw) = read_jbyte_array(&mut env, &json_bytes, "jsonBytes") else {
        return ptr::null_mut();
    };
    match parse_discovery_beacon(&raw) {
        Some(json) => to_jstring(&mut env, &json),
        None => ptr::null_mut(),
    }
}

/* ------------------------------------------------------------------ */
/* Unit tests                                                          */
/* ------------------------------------------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;
    use phonecam_protocol::{
        verify_device_secret, verify_pair_request, write_server_greeting, write_server_response,
        AuthMode, ChannelRole, ClientHandshake, VideoCodec, VideoFlags, VideoPacketHeader,
    };

    #[test]
    fn native_version_matches_contract() {
        assert_eq!(
            NATIVE_VERSION,
            concat!(
                "phonecam-android-core/",
                env!("CARGO_PKG_VERSION"),
                " (protocol v1)"
            )
        );
    }

    #[test]
    fn random_hex_generates_lowercase_hex_and_rejects_negative() {
        let hex = generate_random_hex(16).unwrap();
        assert_eq!(hex.len(), 32);
        assert!(hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_eq!(generate_random_hex(0).unwrap(), "");
        assert!(matches!(
            generate_random_hex(-1),
            Err(NativeError::IllegalArgument(_))
        ));
    }

    #[test]
    fn secret_fingerprint_and_auth_vectors_match_protocol() {
        let secret = "00".repeat(32);
        let fp = secret_fingerprint(&secret);
        assert_eq!(fp.len(), 8);

        let server_nonce: Vec<u8> = (0u8..32).collect();
        let client_nonce = 0x0102_0304_0506_0708i64;
        let auth =
            compute_device_secret_auth("android-1a2b3c4d", &secret, &server_nonce, client_nonce)
                .unwrap();
        let (dev, mac) = auth.split_once(':').unwrap();
        assert_eq!(dev, "android-1a2b3c4d");
        assert_eq!(mac.len(), 64);
        assert!(verify_device_secret(
            "android-1a2b3c4d",
            &secret,
            &server_nonce,
            client_nonce as u64,
            mac
        ));

        // Invalid hex must return IllegalArgument.
        assert!(matches!(
            compute_device_secret_auth("android-1a2b3c4d", "zzzz", &server_nonce, 1),
            Err(NativeError::IllegalArgument(_))
        ));
        assert!(matches!(
            compute_device_secret_auth("android-1a2b3c4d", "abc", &server_nonce, 1),
            Err(NativeError::IllegalArgument(_))
        ));

        // Pair request without secret & with secret.
        let pr_none = pair_auth_check("d1", None, &server_nonce, 42);
        assert!(pr_none.starts_with("d1:request:"));
        assert!(verify_pair_request("d1", None, &server_nonce, 42, &pr_none));

        let pr_with = pair_auth_check("d1", Some(&secret), &server_nonce, 43);
        assert!(verify_pair_request(
            "d1",
            Some(&secret),
            &server_nonce,
            43,
            &pr_with
        ));
        assert!(matches!(
            compute_pair_request_auth("d1", Some("not-hex"), &server_nonce, 1),
            Err(NativeError::IllegalArgument(_))
        ));
    }

    fn pair_auth_check(dev: &str, sec: Option<&str>, nonce: &[u8], client_nonce: i64) -> String {
        compute_pair_request_auth(dev, sec, nonce, client_nonce).unwrap()
    }

    #[test]
    fn greeting_decode_validates_magic_length_and_nonce() {
        let nonce: Vec<u8> = (0u8..32).map(|i| i.wrapping_mul(7)).collect();
        let raw = write_server_greeting(HandshakeStatus::Ok, &nonce).unwrap();
        let decoded = decode_server_greeting(&raw).unwrap();
        assert_eq!(decoded[0], PROTOCOL_VERSION);
        assert_eq!(decoded[1], HandshakeStatus::Ok as u8);
        assert_eq!(&decoded[2..34], nonce.as_slice());

        // Bad magic throws IOException containing "PhoneCam".
        let mut bad_magic = raw.clone();
        bad_magic[0] = b'X';
        match decode_server_greeting(&bad_magic) {
            Err(NativeError::Io(msg)) => assert!(msg.contains("PhoneCam"), "msg={msg}"),
            other => panic!("unexpected: {other:?}"),
        }

        // Bad length throws IOException containing "PhoneCam".
        match decode_server_greeting(&raw[..39]) {
            Err(NativeError::Io(msg)) => assert!(msg.contains("PhoneCam"), "msg={msg}"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn client_handshake_encode_matches_wire_contract() {
        let auth = b"android-deadbeef:abc";
        let bytes = encode_client_handshake(
            ChannelRole::Video as i32,
            AuthMode::DeviceSecret as i32,
            0x0102_0304_0506_0708,
            auth,
        )
        .unwrap();
        assert_eq!(bytes.len(), CLIENT_HANDSHAKE_SIZE + auth.len());
        let parsed = ClientHandshake::parse(&bytes).unwrap();
        assert_eq!(parsed.role, ChannelRole::Video);
        assert_eq!(parsed.auth_mode, AuthMode::DeviceSecret);
        assert_eq!(parsed.client_nonce, 0x0102_0304_0506_0708);
        assert_eq!(parsed.auth, "android-deadbeef:abc");

        let too_long = vec![b'a'; MAX_HANDSHAKE_AUTH + 1];
        assert!(matches!(
            encode_client_handshake(1, 1, 1, &too_long),
            Err(NativeError::IllegalArgument(_))
        ));
    }

    #[test]
    fn server_response_header_decode_validates_fields() {
        let body = serde_json::json!({"deviceId": "android-1a2b3c4d"});
        let full = write_server_response(HandshakeStatus::Ok, ChannelRole::Control, &body).unwrap();
        let hdr = decode_server_response_header(&full[..SERVER_RESPONSE_SIZE]).unwrap();
        assert_eq!(hdr[0], PROTOCOL_VERSION as i32);
        assert_eq!(hdr[1], HandshakeStatus::Ok as i32);
        assert_eq!(hdr[2], ChannelRole::Control as i32);
        assert_eq!(hdr[3] as usize, full.len() - SERVER_RESPONSE_SIZE);

        let mut bad_magic = full[..SERVER_RESPONSE_SIZE].to_vec();
        bad_magic[0] = b'Z';
        assert!(matches!(
            decode_server_response_header(&bad_magic),
            Err(NativeError::Io(_))
        ));

        let mut oversized = full[..SERVER_RESPONSE_SIZE].to_vec();
        oversized[16..20].copy_from_slice(&((MAX_CONTROL_JSON + 1) as u32).to_be_bytes());
        assert!(matches!(
            decode_server_response_header(&oversized),
            Err(NativeError::Io(_))
        ));
    }

    #[test]
    fn video_packet_header_and_packet_round_trip() {
        let payload = vec![0xAA, 0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1E, 0xBB];
        let slice = &payload[1..9];
        let hdr_only = encode_video_packet_header(
            0x8000_0007,
            123_456,
            (VideoFlags::KEYFRAME.0 | VideoFlags::CODEC_CONFIG.0) as i32,
            VideoCodec::H264 as i32,
            slice.len() as i32,
        )
        .unwrap();
        assert_eq!(hdr_only.len(), 28);

        let packet = encode_video_packet(
            0x8000_0007,
            123_456,
            (VideoFlags::KEYFRAME.0 | VideoFlags::CODEC_CONFIG.0) as i32,
            VideoCodec::H264 as i32,
            &payload,
            1,
            8,
        )
        .unwrap();
        assert_eq!(&packet[..28], &hdr_only[..]);
        assert_eq!(&packet[28..], slice);

        let (parsed_hdr, parsed_body) = VideoPacketHeader::parse(&packet).unwrap();
        assert_eq!(parsed_hdr.frame_id, 0x8000_0007);
        assert_eq!(parsed_hdr.pts_us, 123_456);
        assert_eq!(parsed_body, slice);

        let decoded = decode_video_packet_header(&packet[4..28]).unwrap();
        assert_eq!(decoded[0], 0x8000_0007i64);
        assert_eq!(decoded[1], 123_456);
        assert_eq!(
            decoded[2],
            (VideoFlags::KEYFRAME.0 | VideoFlags::CODEC_CONFIG.0) as i64
        );
        assert_eq!(decoded[3], VideoCodec::H264 as i64);
        assert_eq!(decoded[4], 8);

        assert!(matches!(
            encode_video_packet_header(0, 0, 0, 1, (MAX_VIDEO_PAYLOAD + 1) as i32),
            Err(NativeError::Io(_))
        ));
        assert!(matches!(
            encode_video_packet(0, 0, 0, 1, &payload, 5, 10),
            Err(NativeError::IndexOutOfBounds(_))
        ));
    }

    #[test]
    fn qr_payload_validation_handles_optional_fields_and_rejects_bad_inputs() {
        let token = "a".repeat(32);
        let without_name = format!(
            r#"{{"protocol":"phonecam","version":1,"host":"192.168.1.24","port":8765,"token":"{token}"}}"#
        );
        let norm1 = validate_qr_payload(&without_name).expect("name-less QR must succeed");
        let v1: Value = serde_json::from_str(&norm1).unwrap();
        assert_eq!(v1["protocol"], "phonecam");
        assert_eq!(v1["version"], 1);
        assert_eq!(v1["host"], "192.168.1.24");
        assert_eq!(v1["port"], 8765);
        assert_eq!(v1["token"], token);
        assert!(v1["name"].is_null());
        assert!(v1["serverId"].is_null());

        let with_name = format!(
            r#"{{"protocol":"phonecam","version":1,"host":"192.168.1.24","port":8765,"token":"{token}","name":"DESKTOP-ABC","serverId":"pc-1"}}"#
        );
        let norm2 = validate_qr_payload(&with_name).unwrap();
        let v2: Value = serde_json::from_str(&norm2).unwrap();
        assert_eq!(v2["name"], "DESKTOP-ABC");
        assert_eq!(v2["serverId"], "pc-1");

        // Blank name normalizes to null.
        let blank_name = format!(
            r#"{{"protocol":"phonecam","version":1,"host":"192.168.1.24","port":8765,"token":"{token}","name":"   "}}"#
        );
        let v3: Value = serde_json::from_str(&validate_qr_payload(&blank_name).unwrap()).unwrap();
        assert!(v3["name"].is_null());

        assert!(validate_qr_payload("https://example.com/not-a-pairing-code").is_none());
        assert!(validate_qr_payload(r#"{"protocol":"something-else","version":1}"#).is_none());
        assert!(validate_qr_payload(
            r#"{"protocol":"phonecam","version":1,"host":"192.168.1.24","port":8765,"token":"nope"}"#
        )
        .is_none());
    }

    #[test]
    fn discovery_beacon_parses_desktop_beacon_and_rejects_others() {
        let beacon = DesktopBeacon::new(
            "pc-1234",
            "Workstation",
            "192.168.1.10",
            SERVICE_PORT,
            true,
            Some("pair-1".into()),
            vec!["android-1".into()],
            false,
            "1.0.0",
        );
        let raw = serde_json::to_vec(&beacon).unwrap();
        let parsed = parse_discovery_beacon(&raw).unwrap();
        let v: Value = serde_json::from_str(&parsed).unwrap();
        assert_eq!(v["t"], "phonecam.beacon");
        assert_eq!(v["serverId"], "pc-1234");
        assert_eq!(v["name"], "Workstation");
        assert_eq!(v["port"], SERVICE_PORT);
        assert_eq!(v["pairingOpen"], true);
        assert_eq!(v["pairedDeviceIds"][0], "android-1");

        // Probe and phone beacons must return None.
        assert!(
            parse_discovery_beacon(br#"{"t":"phonecam.probe","v":1,"role":"phone"}"#).is_none()
        );
        assert!(parse_discovery_beacon(
            br#"{"t":"phonecam.beacon","v":1,"role":"phone","deviceId":"android-1"}"#
        )
        .is_none());
        assert!(parse_discovery_beacon(b"not json").is_none());
    }

    #[test]
    fn encode_video_packet_header_into_matches_allocating_and_checks_bounds() {
        let mut scratch = [0xEEu8; 32];
        encode_video_packet_header_into(
            &mut scratch,
            4,
            0x8000_0007,
            123_456,
            (VideoFlags::KEYFRAME.0 | VideoFlags::CODEC_CONFIG.0) as i32,
            VideoCodec::H264 as i32,
            4096,
        )
        .unwrap();

        let expected = encode_video_packet_header(
            0x8000_0007,
            123_456,
            (VideoFlags::KEYFRAME.0 | VideoFlags::CODEC_CONFIG.0) as i32,
            VideoCodec::H264 as i32,
            4096,
        )
        .unwrap();
        assert_eq!(&scratch[..4], &[0xEE; 4]);
        assert_eq!(&scratch[4..32], &expected[..]);

        assert!(matches!(
            encode_video_packet_header_into(&mut scratch, -1, 1, 0, 0, 1, 100),
            Err(NativeError::IndexOutOfBounds(_))
        ));
        assert!(matches!(
            encode_video_packet_header_into(&mut scratch, 5, 1, 0, 0, 1, 100),
            Err(NativeError::IndexOutOfBounds(_))
        ));
        assert!(matches!(
            encode_video_packet_header_into(
                &mut scratch,
                0,
                1,
                0,
                0,
                1,
                (MAX_VIDEO_PAYLOAD + 1) as i32
            ),
            Err(NativeError::Io(_))
        ));
    }

    #[test]
    fn rust_hot_path_throughput_and_latency() {
        let mut scratch = [0u8; 28];
        let header_start = std::time::Instant::now();
        for i in 0..10_000i64 {
            encode_video_packet_header_into(&mut scratch, 0, i, i * 16_667, 1, 1, 32_768).unwrap();
            std::hint::black_box(&scratch);
        }
        let header_elapsed = header_start.elapsed();

        let secret = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let nonce: [u8; 32] = [0x5A; 32];
        let auth_start = std::time::Instant::now();
        for i in 0..1_000i64 {
            let auth = compute_device_secret_auth("android-perf", secret, &nonce, i).unwrap();
            std::hint::black_box(auth);
        }
        let auth_elapsed = auth_start.elapsed();

        println!(
            "perf(phonecam-android-core): 10k headers={:?}, 1k auth={:?}",
            header_elapsed, auth_elapsed
        );
        assert!(
            header_elapsed.as_millis() < 500,
            "header encode too slow: {header_elapsed:?}"
        );
        assert!(
            auth_elapsed.as_millis() < 1000,
            "auth too slow: {auth_elapsed:?}"
        );
    }
}
