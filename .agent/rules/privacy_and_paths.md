# Privacy, Anonymity & Path Generality Rules

This document defines the strict privacy, path neutrality, and hardware abstraction rules for the PhoneCam codebase. Every agent, tool, and human contributor must follow these requirements across all crates, apps, scripts, tests, and documentation.

---

## 1. Non-Negotiable Prohibition Invariants

The following elements must **never** appear hardcoded in source code, unit/integration tests, build scripts, markdown documentation, or configuration files:

### 1.1 Personal User Directories & Profiles
- **Prohibited**: `C:\Users\<username>\...`, `%USERPROFILE%\...` in code without environment variable abstraction, `/home/<username>/...`, `/Users/<username>/...`.
- **Reasoning**: Hardcoding developer home directories compromises privacy, leaks identity, and breaks execution on any other developer machine, container, or CI runner.

### 1.2 Developer Drive Letters & Machine-Specific Roots
- **Prohibited**: Hardcoded Windows drive roots (e.g. `D:\production\...`, `D:\apps\...`, `E:\...`) or non-standard file system paths.
- **Reasoning**: Directory structures and disk partitioning vary across contributor workstations and CI virtual machines.

### 1.3 Personal Usernames & Accounts
- **Prohibited**: Personal account usernames (e.g., `Prima`), local login identities, or private workspace paths.
- **Allowed Exception**: The public GitHub repository namespace (`github.com/primahagantha/phonecam`) and release author metadata in `release-gate.json` are legitimate repository URLs.

### 1.4 Single-Device Hardware Serials
- **Prohibited**: Hardcoding physical hardware serials (e.g., `79134849`) into tests, scripts, or runtime configs.
- **Reasoning**: Tests must be completely hardware-agnostic and able to run against any connected Android device, or skip cleanly when no device is present.

### 1.5 Transient AI Agent / Session Brain Directories
- **Prohibited**: Storing test output, screenshots, or logs into session-specific AI brain paths (e.g., `C:\Users\...\.gemini\antigravity\brain\...`).
- **Reasoning**: Brain paths are ephemeral, conversation-scoped, and specific to individual AI agent harnesses.

---

## 2. Standard Dynamic Resolution Patterns

When interacting with external tools, devices, or file systems, always use these dynamic resolution patterns:

### 2.1 Android Device Serial Resolution
Integration and hardware tests must resolve target device serials dynamically:

```rust
pub fn resolve_device_serial(adb_path: &Path) -> Option<String> {
    // 1. Explicit override via environment variable
    if let Ok(serial) = std::env::var("ANDROID_SERIAL") {
        let trimmed = serial.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }

    // 2. Query `adb devices` for the first authorized device in "device" state
    if let Ok(output) = Command::new(adb_path).arg("devices").output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("List of devices") {
                continue;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() >= 2 && parts[1] == "device" {
                return Some(parts[0].to_string());
            }
        }
    }

    // 3. Fallback to `adb get-serialno`
    if let Ok(output) = Command::new(adb_path).arg("get-serialno").output() {
        let serial = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !serial.is_empty() && serial != "unknown" && !serial.contains("error") {
            return Some(serial);
        }
    }

    None
}
```

#### Graceful Test Skipping
When no physical Android device is connected, hardware tests must skip gracefully without panicking:
```rust
let device_serial = match resolve_device_serial(&adb_path) {
    Some(s) => s,
    None => {
        println!("SKIPPED: No Android device detected");
        return;
    }
};
```

---

### 2.2 Android Debug Bridge (`adb`) Resolution
The desktop application and test suites must resolve `adb` through prioritized discovery:

```rust
pub fn resolve_adb_path() -> Option<PathBuf> {
    // 1. Explicit user override
    if let Ok(path) = std::env::var("PHONECAM_ADB_PATH") {
        let p = PathBuf::from(path.trim());
        if p.is_file() {
            return Some(p);
        }
    }

    // 2. Relative paths from current working directory / dist staging
    let candidates = [
        "dist/unpacked/adb.exe",
        "dist/unpacked/adb",
        "../../dist/unpacked/adb.exe",
        "../../dist/unpacked/adb",
        "adb.exe",
        "adb",
    ];
    for rel in candidates {
        let p = PathBuf::from(rel);
        if p.is_file() {
            return Some(p);
        }
    }

    // 3. System PATH search
    if let Ok(path) = std::env::var("PATH") {
        for entry in std::env::split_paths(&path) {
            let exe = entry.join("adb.exe");
            if exe.is_file() {
                return Some(exe);
            }
            let bin = entry.join("adb");
            if bin.is_file() {
                return Some(bin);
            }
        }
    }

    // 4. Standard Android SDK locations
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(local).join("Android").join("Sdk").join("platform-tools").join("adb.exe");
        if p.is_file() {
            return Some(p);
        }
    }

    for env_var in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(root) = std::env::var(env_var) {
            let p = PathBuf::from(root).join("platform-tools").join("adb.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }

    None
}
```

---

### 2.3 Test Artifacts & Screenshot Output
Test suites that generate diagnostic frames or screenshots must resolve output directories via `PHONECAM_ARTIFACT_DIR`, defaulting to the operating system's standard temporary directory:

```rust
pub fn resolve_artifact_dir() -> PathBuf {
    let dir = if let Ok(val) = std::env::var("PHONECAM_ARTIFACT_DIR") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            PathBuf::from(trimmed)
        } else {
            std::env::temp_dir().join("phonecam-artifacts")
        }
    } else {
        std::env::temp_dir().join("phonecam-artifacts")
    };

    let _ = std::fs::create_dir_all(&dir);
    dir
}
```

---

### 2.4 Android NDK Discovery in Build Scripts
PowerShell and shell build scripts must discover the Android NDK dynamically from standard environment variables and SDK directories without hardcoded drive letters:

```powershell
if (-not $env:ANDROID_NDK_HOME -or -not (Test-Path $env:ANDROID_NDK_HOME)) {
    $candidateRoots = @(
        $env:ANDROID_NDK_ROOT,
        $(if ($env:ANDROID_HOME) { Join-Path $env:ANDROID_HOME "ndk" }),
        $(if ($env:ANDROID_SDK_ROOT) { Join-Path $env:ANDROID_SDK_ROOT "ndk" }),
        $(if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "Android\Sdk\ndk" })
    ) | Where-Object { $_ -and (Test-Path $_) }

    foreach ($root in $candidateRoots) {
        $latest = Get-ChildItem -Path $root -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
        if ($latest) {
            $env:ANDROID_NDK_HOME = $latest.FullName
            break
        }
    }
}
if (-not $env:ANDROID_NDK_HOME -or -not (Test-Path $env:ANDROID_NDK_HOME)) {
    throw "ANDROID_NDK_HOME is not set or not found. Please set ANDROID_NDK_HOME to your Android NDK installation directory."
}
```

---

## 3. Permitted System Invariant Exceptions

The only permissible hardcoded absolute system paths in the PhoneCam codebase are standardized operating system contracts:

1. **Windows Shared Memory Framebus Backing File**:
   - `C:\ProgramData\PhoneCam\framebuffer-v2.bin`
   - Fixed by the Windows Media Foundation Virtual Camera specification (`svchost.exe -k Camera` running under `NT AUTHORITY\LocalService`).
2. **Windows Installation Program Files Path**:
   - `C:\Program Files\PhoneCam`
   - Fixed by the standard 64-bit Windows NSIS installer directory structure.

---

## 4. Privacy Audit & Verification Checklist

Before committing or submitting changes, verify that zero hardcoded personal paths exist:

```powershell
# 1. Search for personal username or user profile paths
git grep -i "users\\\\"
git grep -i "users/"

# 2. Search for machine-specific drive paths
git grep -E "[D-Zd-z]:\\\\"

# 3. Search for specific hardware device serials
git grep "79134849"

# 4. Search for AI agent conversation or brain paths
git grep -i "antigravity"
git grep -i "\.gemini"
```
All searches must return zero hits in source, test, script, and markdown files.
