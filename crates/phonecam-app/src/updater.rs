//! User-initiated GitHub Releases update checker.
//!
//! Zero-telemetry invariant: this module NEVER runs automatically on startup or
//! on any background timer. It is invoked strictly in response to an explicit
//! user action (`SideEffect::CheckForUpdates` / `SideEffect::OpenUpdateUrl`).

use std::cmp::Ordering;

use phonecam_ui::{ReleaseInfo, UpdateStatus};
use serde::Deserialize;

/// GitHub API host for release queries.
pub const GITHUB_API_HOST: &str = "api.github.com";

/// GitHub API path for the latest PhoneCam release.
pub const GITHUB_API_PATH: &str = "/repos/primahagantha/phonecam/releases/latest";

/// Full HTTPS endpoint queried when the user clicks "Check for Updates".
pub const GITHUB_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/primahagantha/phonecam/releases/latest";

/// Required URL prefix for any browser launch triggered by the update UI.
pub const ALLOWED_UPDATE_URL_PREFIX: &str = "https://github.com/primahagantha/phonecam";

/// Network timeout (milliseconds) applied to resolve, connect, send, and receive.
const REQUEST_TIMEOUT_MS: i32 = 8_000;

/// Maximum JSON response size read from the GitHub Releases API (2 MiB).
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

/// A single dot-separated SemVer 2.0.0 prerelease identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreReleaseIdent {
    Numeric(u64),
    Alphanumeric(String),
}

impl Ord for PreReleaseIdent {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Numeric(a), Self::Numeric(b)) => a.cmp(b),
            // SemVer 2.0.0 §11.4.3: Numeric identifiers always have lower
            // precedence than non-numeric identifiers.
            (Self::Numeric(_), Self::Alphanumeric(_)) => Ordering::Less,
            (Self::Alphanumeric(_), Self::Numeric(_)) => Ordering::Greater,
            (Self::Alphanumeric(a), Self::Alphanumeric(b)) => a.cmp(b),
        }
    }
}

impl PartialOrd for PreReleaseIdent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parsed Semantic Version 2.0.0 (`MAJOR.MINOR.PATCH[-PRERELEASE]`).
///
/// Build metadata (`+...`) is ignored for precedence per SemVer 2.0.0 §10.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Vec<PreReleaseIdent>,
}

impl Ord for SemVer {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(
                || match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
                    (true, true) => Ordering::Equal,
                    // SemVer 2.0.0 §11.3: Normal version has higher precedence than a pre-release.
                    (true, false) => Ordering::Greater,
                    (false, true) => Ordering::Less,
                    (false, false) => {
                        for (a, b) in self.prerelease.iter().zip(other.prerelease.iter()) {
                            let ord = a.cmp(b);
                            if ord != Ordering::Equal {
                                return ord;
                            }
                        }
                        // SemVer 2.0.0 §11.4.4: Larger set of pre-release fields has higher precedence.
                        self.prerelease.len().cmp(&other.prerelease.len())
                    }
                },
            )
    }
}

impl PartialOrd for SemVer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parses a SemVer 2.0.0 version string, accepting an optional leading `v`/`V`.
pub fn parse_semver(input: &str) -> Option<SemVer> {
    let trimmed = input.trim();
    let without_v = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);

    // Strip optional build metadata (+...) per SemVer 2.0.0 §10.
    let without_build = match without_v.split_once('+') {
        Some((before, build)) => {
            if build.is_empty() {
                return None;
            }
            before
        }
        None => without_v,
    };

    let (core, pre_str) = match without_build.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (without_build, None),
    };

    let mut parts = core.split('.');
    let major = parse_numeric_core(parts.next()?)?;
    let minor = parse_numeric_core(parts.next()?)?;
    let patch = parse_numeric_core(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }

    let mut prerelease = Vec::new();
    if let Some(pre) = pre_str {
        if pre.is_empty() {
            return None;
        }
        for ident in pre.split('.') {
            if ident.is_empty()
                || !ident
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return None;
            }
            if ident.bytes().all(|b| b.is_ascii_digit()) {
                // SemVer 2.0.0 §9: Numeric identifiers must not include leading zeroes.
                if ident.len() > 1 && ident.starts_with('0') {
                    return None;
                }
                let n = ident.parse::<u64>().ok()?;
                prerelease.push(PreReleaseIdent::Numeric(n));
            } else {
                prerelease.push(PreReleaseIdent::Alphanumeric(ident.to_string()));
            }
        }
    }

    Some(SemVer {
        major,
        minor,
        patch,
        prerelease,
    })
}

fn parse_numeric_core(part: &str) -> Option<u64> {
    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if part.len() > 1 && part.starts_with('0') {
        return None;
    }
    part.parse::<u64>().ok()
}

/// Returns `true` iff `latest` is strictly greater than `current` according to SemVer 2.0.0.
pub fn is_newer_version(current: &str, latest: &str) -> bool {
    match (parse_semver(current), parse_semver(latest)) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => false,
    }
}

/// Validates that an update URL points strictly to the official PhoneCam GitHub repository
/// and contains no whitespace or shell/control characters.
pub fn is_safe_update_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix(ALLOWED_UPDATE_URL_PREFIX) else {
        return false;
    };
    if !rest.is_empty()
        && !rest.starts_with('/')
        && !rest.starts_with('?')
        && !rest.starts_with('#')
    {
        return false;
    }
    !url.chars().any(|c| {
        c.is_control()
            || c.is_whitespace()
            || matches!(c, '"' | '\'' | '<' | '>' | '|' | '&' | '^' | '`')
    })
}

#[derive(Debug, Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
}

/// Parses a GitHub Releases API JSON payload into a `ReleaseInfo`.
pub fn parse_release_json(json: &str) -> Result<ReleaseInfo, String> {
    let resp: GithubReleaseResponse = serde_json::from_str(json)
        .map_err(|err| format!("Could not parse GitHub release response: {err}"))?;

    let tag = resp.tag_name.trim().to_string();
    let version = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(&tag)
        .to_string();

    if parse_semver(&version).is_none() {
        return Err(format!("GitHub release tag '{tag}' is not valid SemVer."));
    }

    let url = resp.html_url.trim().to_string();
    if !is_safe_update_url(&url) {
        return Err("GitHub release URL failed safety validation.".to_string());
    }

    let published_at = resp
        .published_at
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.split('T').next().unwrap_or(s).to_string());

    let changelog = resp.body.unwrap_or_default().trim().to_string();

    Ok(ReleaseInfo {
        version,
        tag,
        url,
        changelog,
        published_at,
    })
}

/// Queries GitHub Releases once and compares the latest tag against `current_version`.
pub fn check_latest_release(current_version: &str) -> UpdateStatus {
    let normalized_current = current_version
        .trim()
        .strip_prefix('v')
        .or_else(|| current_version.trim().strip_prefix('V'))
        .unwrap_or(current_version.trim())
        .to_string();

    let body = match fetch_latest_release_json(&normalized_current) {
        Ok(body) => body,
        Err(err) => return UpdateStatus::Failed(err),
    };

    let release = match parse_release_json(&body) {
        Ok(release) => release,
        Err(err) => return UpdateStatus::Failed(err),
    };

    if is_newer_version(&normalized_current, &release.version) {
        UpdateStatus::Available(release)
    } else {
        UpdateStatus::UpToDate {
            current: normalized_current,
        }
    }
}

/// Opens a validated GitHub release URL in the user's default browser.
pub fn open_update_url(url: &str) -> Result<(), String> {
    if !is_safe_update_url(url) {
        return Err(format!("Refused to open untrusted update URL: {url}"));
    }

    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let wide_url: Vec<u16> = OsStr::new(url)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: `wide_url` is a valid null-terminated UTF-16 string that outlives the
        // `ShellExecuteW` call, and `w!("open")` is a static null-terminated wide string.
        // Passing `None` as HWND opens the validated HTTPS URL in the user's default browser.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide_url.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if (result.0 as isize) > 32 {
            Ok(())
        } else {
            Err(format!(
                "ShellExecuteW failed with code {}",
                result.0 as isize
            ))
        }
    }

    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        let opener = "open";
        #[cfg(not(target_os = "macos"))]
        let opener = "xdg-open";

        std::process::Command::new(opener)
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("could not launch {opener}: {err}"))
    }
}

#[cfg(windows)]
fn fetch_latest_release_json(current_version: &str) -> Result<String, String> {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Networking::WinHttp::{
        WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
        INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
        WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_ADDREQ_FLAG_REPLACE,
        WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    };

    struct WinHttpHandle(*mut c_void);

    impl Drop for WinHttpHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` was returned by a WinHTTP open/connect/request call
                // and is owned exclusively by this guard, so closing it once on drop is sound.
                unsafe {
                    let _ = WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    fn to_wide_null(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let user_agent = to_wide_null(&format!("PhoneCam-Desktop/{current_version}"));
    let host = to_wide_null(GITHUB_API_HOST);
    let path = to_wide_null(GITHUB_API_PATH);

    // SAFETY: `user_agent` is null-terminated UTF-16; WinHttpOpen returns either a valid
    // HINTERNET handle or null on error.
    let session_raw = unsafe {
        let h = WinHttpOpen(
            PCWSTR(user_agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if h.is_null() {
            WinHttpOpen(
                PCWSTR(user_agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        } else {
            h
        }
    };
    if session_raw.is_null() {
        return Err(format!(
            "Could not initialize Windows HTTP session: {}",
            std::io::Error::last_os_error()
        ));
    }
    let session = WinHttpHandle(session_raw);

    // SAFETY: `session.0` is a valid WinHTTP session handle.
    unsafe {
        let _ = WinHttpSetTimeouts(
            session.0,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
        );
    }

    // SAFETY: `session.0` is valid and `host` is a null-terminated UTF-16 hostname.
    let connect_raw = unsafe {
        WinHttpConnect(
            session.0,
            PCWSTR(host.as_ptr()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        )
    };
    if connect_raw.is_null() {
        return Err(format!(
            "Could not connect to GitHub: {}",
            std::io::Error::last_os_error()
        ));
    }
    let connect = WinHttpHandle(connect_raw);

    // SAFETY: `connect.0` is valid, `w!("GET")` and `path` are null-terminated UTF-16 strings,
    // and null pointers for version/referrer/accept-types request WinHTTP defaults over TLS.
    let request_raw = unsafe {
        WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            PCWSTR(path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        )
    };
    if request_raw.is_null() {
        return Err(format!(
            "Could not open GitHub Releases request: {}",
            std::io::Error::last_os_error()
        ));
    }
    let request = WinHttpHandle(request_raw);

    let headers: Vec<u16> =
        OsStr::new("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n")
            .encode_wide()
            .collect();

    // SAFETY: `request.0` is valid and `headers` is a valid UTF-16 slice.
    unsafe {
        WinHttpAddRequestHeaders(
            request.0,
            &headers,
            WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
        )
        .map_err(|err| format!("Could not set HTTP headers: {err}"))?;
    }

    // SAFETY: `request.0` is valid; no additional headers or request body are supplied.
    unsafe {
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(|err| {
            format!("Could not reach GitHub Releases (check your internet connection): {err}")
        })?;
    }

    // SAFETY: `request.0` is a sent WinHTTP request handle; reserved parameter must be null.
    unsafe {
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|err| format!("No response received from GitHub Releases: {err}"))?;
    }

    let mut status_code: u32 = 0;
    let mut status_len = std::mem::size_of::<u32>() as u32;
    // SAFETY: `status_code` is properly aligned and `status_len` matches its byte size
    // as required by `WINHTTP_QUERY_FLAG_NUMBER`.
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status_code as *mut u32).cast::<c_void>()),
            &mut status_len,
            std::ptr::null_mut(),
        )
        .map_err(|err| format!("Could not read HTTP status from GitHub: {err}"))?;
    }

    match status_code {
        200..=299 => {}
        404 => return Err("No published releases found on GitHub yet.".to_string()),
        403 | 429 => {
            return Err("GitHub API rate limit reached. Please try again later.".to_string());
        }
        code => return Err(format!("GitHub Releases returned HTTP {code}.")),
    }

    let mut body = Vec::with_capacity(8 * 1024);
    loop {
        let mut available: u32 = 0;
        // SAFETY: `request.0` is a valid response handle and `available` is a valid out-pointer.
        unsafe {
            WinHttpQueryDataAvailable(request.0, &mut available)
                .map_err(|err| format!("Error checking GitHub response stream: {err}"))?;
        }
        if available == 0 {
            break;
        }

        let to_read = (available as usize).min(64 * 1024);
        if body.len().saturating_add(to_read) > MAX_RESPONSE_BYTES {
            return Err("GitHub release response exceeded size limit.".to_string());
        }

        let start = body.len();
        body.resize(start + to_read, 0);
        let mut bytes_read: u32 = 0;
        // SAFETY: `body[start..start + to_read]` is valid writable memory of length `to_read`.
        unsafe {
            WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast::<c_void>(),
                to_read as u32,
                &mut bytes_read,
            )
            .map_err(|err| format!("Error reading GitHub release response: {err}"))?;
        }
        body.truncate(start + bytes_read as usize);
        if bytes_read == 0 {
            break;
        }
    }

    String::from_utf8(body)
        .map_err(|err| format!("GitHub release response was not valid UTF-8: {err}"))
}

#[cfg(not(windows))]
fn fetch_latest_release_json(current_version: &str) -> Result<String, String> {
    let user_agent = format!("User-Agent: PhoneCam-Desktop/{current_version}");
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "8",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            &user_agent,
            GITHUB_LATEST_RELEASE_URL,
        ])
        .output()
        .map_err(|err| format!("Could not execute curl for update check: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!("GitHub update check failed ({})", output.status)
        } else {
            format!("GitHub update check failed: {detail}")
        });
    }

    if output.stdout.len() > MAX_RESPONSE_BYTES {
        return Err("GitHub release response exceeded size limit.".to_string());
    }

    String::from_utf8(output.stdout)
        .map_err(|err| format!("GitHub release response was not valid UTF-8: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_handles_v_prefix_and_core_ordering() {
        assert!(is_newer_version("1.4.0", "1.4.1"));
        assert!(is_newer_version("v1.4.0", "v1.4.1"));
        assert!(is_newer_version("1.4.0", "v1.5.0"));
        assert!(is_newer_version("1.4.9", "2.0.0"));
        assert!(is_newer_version("1.4.0", "1.10.0"));
        assert!(!is_newer_version("1.4.0", "1.4.0"));
        assert!(!is_newer_version("v1.4.0", "1.4.0"));
        assert!(!is_newer_version("1.4.1", "1.4.0"));
        assert!(!is_newer_version("2.0.0", "1.99.99"));
    }

    #[test]
    fn semver_handles_prerelease_precedence() {
        // 1.4.0-nightly.1 < 1.4.0 < 1.4.1
        assert!(is_newer_version("1.4.0-nightly.1", "1.4.0"));
        assert!(is_newer_version("1.4.0", "1.4.1"));
        assert!(is_newer_version("1.4.0-nightly.1", "1.4.1"));
        assert!(!is_newer_version("1.4.0", "1.4.0-nightly.1"));

        // Numeric vs alphanumeric and numeric ordering inside prereleases
        assert!(is_newer_version("1.4.0-alpha.2", "1.4.0-alpha.10"));
        assert!(is_newer_version("1.4.0-alpha", "1.4.0-alpha.1"));
        assert!(is_newer_version("1.4.0-alpha.1", "1.4.0-alpha.beta"));
        assert!(is_newer_version("1.4.0-alpha.beta", "1.4.0-beta"));
        assert!(is_newer_version("1.4.0-beta.2", "1.4.0-beta.11"));
        assert!(is_newer_version("1.4.0-beta.11", "1.4.0-rc.1"));
        assert!(is_newer_version("1.4.0-rc.1", "1.4.0"));
    }

    #[test]
    fn semver_ignores_build_metadata_and_rejects_malformed() {
        assert!(!is_newer_version("1.4.0+build.1", "1.4.0+build.2"));
        assert!(is_newer_version("1.4.0+build.99", "1.4.1+build.1"));
        assert!(parse_semver("1.4").is_none());
        assert!(parse_semver("1.4.0.1").is_none());
        assert!(parse_semver("01.4.0").is_none());
        assert!(parse_semver("1.4.0-").is_none());
        assert!(parse_semver("1.4.0-alpha..1").is_none());
        assert!(parse_semver("1.4.0-01").is_none());
        assert!(parse_semver("1.4.0+").is_none());
        assert!(!is_newer_version("invalid", "1.4.1"));
        assert!(!is_newer_version("1.4.0", "invalid"));
    }

    #[test]
    fn parse_release_json_extracts_fields_and_validates_url() {
        let sample = r###"{
            "tag_name": "v1.5.0",
            "html_url": "https://github.com/primahagantha/phonecam/releases/tag/v1.5.0",
            "body": "## What's New\n- Ultra-low latency video\n- Bundled fonts",
            "published_at": "2026-04-20T18:30:00Z"
        }"###;
        let info = parse_release_json(sample).expect("valid GitHub release JSON");
        assert_eq!(info.tag, "v1.5.0");
        assert_eq!(info.version, "1.5.0");
        assert_eq!(
            info.url,
            "https://github.com/primahagantha/phonecam/releases/tag/v1.5.0"
        );
        assert!(info.changelog.contains("Ultra-low latency video"));
        assert_eq!(info.published_at.as_deref(), Some("2026-04-20"));
    }

    #[test]
    fn parse_release_json_rejects_untrusted_url_and_bad_tag() {
        let untrusted = r#"{
            "tag_name": "v1.5.0",
            "html_url": "https://evil.example.com/phonecam/releases/tag/v1.5.0",
            "body": "spoofed"
        }"#;
        assert!(parse_release_json(untrusted).is_err());

        let bad_tag = r#"{
            "tag_name": "not-a-semver",
            "html_url": "https://github.com/primahagantha/phonecam/releases/tag/not-a-semver"
        }"#;
        assert!(parse_release_json(bad_tag).is_err());
    }

    #[test]
    fn safe_update_url_enforces_repo_prefix_and_no_injection() {
        assert!(is_safe_update_url(
            "https://github.com/primahagantha/phonecam"
        ));
        assert!(is_safe_update_url(
            "https://github.com/primahagantha/phonecam/releases/tag/v1.5.0"
        ));
        assert!(!is_safe_update_url(
            "https://github.com/primahagantha/phonecam-evil/releases"
        ));
        assert!(!is_safe_update_url(
            "http://github.com/primahagantha/phonecam/releases"
        ));
        assert!(!is_safe_update_url(
            "https://github.com/primahagantha/phonecam/releases & calc.exe"
        ));
        assert!(!is_safe_update_url(
            "https://github.com/primahagantha/phonecam/releases\"calc"
        ));
    }
}
