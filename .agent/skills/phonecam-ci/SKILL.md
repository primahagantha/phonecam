---
name: phonecam-ci
description: Complete reference for GitHub Actions matrix, multi-platform builds, semantic versioning, and release artifacts.
---

# PhoneCam CI/CD & Release Engineering Skill Guide

This skill guide provides a comprehensive reference for PhoneCam's continuous integration pipelines, automated multi-platform build matrix, release packaging, semantic versioning lifecycle, and release artifact generation.

---

## 1. CI/CD Architecture Overview

PhoneCam utilizes GitHub Actions for continuous integration, pull request quality gating, and automated multi-platform release publishing:

```
[ Git Push / PR ]
       │
       ├──► Commitlint (Conventional Commits validation)
       │
       ├──► Rust Workspace Matrix:
       │     ├── Windows (MSVC x64, Win32/DirectX APIs)
       │     └── Linux (Ubuntu x64, headless media & Vulkan/Wayland dependencies)
       │
       └──► Android CI:
             ├── JDK 17 & Rust host toolchain
             ├── Build host phonecam-android-core library
             ├── Gradle JVM Unit Tests (./gradlew test)
             └── Assemble Release APK (./gradlew assembleRelease)

[ Git Tag Push: v* ]
       │
       ├──► Job 1: Build Windows Release & NSIS Installer
       │     ├── cargo build --release -p phonecam-app
       │     ├── makensis scripts/installer.nsi -> PhoneCam-Setup-*.exe
       │     └── Compress-Archive -> PhoneCam-portable.zip
       │
       ├──► Job 2: Build Android Release APK
       │     ├── cargo build -p phonecam-android-core --release
       │     ├── ./gradlew assembleRelease
       │     └── Package -> PhoneCam.apk
       │
       └──► Job 3: Publish GitHub Release (softprops/action-gh-release)
             └── Bundle & upload installer, portable ZIP, APK, and release notes
```

---

## 2. Continuous Integration Workflow (`ci.yml`)

The primary CI workflow is defined at `.github/workflows/ci.yml` and triggers on pushes to `main`/`master`, pull requests, and manual dispatches.

### 2.1 Commitlint Job
- **Target**: Runs on `ubuntu-latest` for all pull requests.
- **Action**: Uses `wagoid/commitlint-github-action@v6` with `.commitlintrc.json`.
- **Validation**: Enforces Conventional Commits formatting:
  `<type>(<scope>): <subject>`
  Valid scopes: `(protocol)`, `(framebus)`, `(media)`, `(core)`, `(ui)`, `(app)`, `(android)`, `(vcam)`, `(deps)`.

### 2.2 Rust Workspace Matrix Job
- **Matrix OS**: `[windows-latest, ubuntu-latest]`.
- **Concurrency**: `cancel-in-progress: true` scoped by workflow and ref.
- **Toolchain**: Rust stable (`dtolnay/rust-toolchain@stable`).
- **Caching**: `Swatinem/rust-cache@v2` with key prefix `rust-ci-${{ matrix.os }}`.
- **Linux System Packages**:
  To compile headless UI and media dependencies on Linux runners, the job installs:
  ```bash
  sudo apt-get update && sudo apt-get install -y --no-install-recommends \
    libasound2-dev \
    libfontconfig1-dev \
    libfreetype6-dev \
    libxkbcommon-dev \
    libx11-dev \
    libxcb1-dev \
    libx11-xcb-dev \
    libvulkan-dev \
    libwayland-dev \
    libssl-dev \
    pkg-config
  ```
- **Step Execution**:
  1. `cargo check --workspace`: Verifies syntax and type soundness across all crates.
  2. `cargo test --workspace`: Executes 230+ automated tests across the workspace.

### 2.3 Android CI Job
- **Target**: Runs on `ubuntu-latest`.
- **JDK Configuration**: Java JDK 17 Temurin (`actions/setup-java@v4`).
- **JNI Host Library Step**:
  Compiles `phonecam-android-core` for the host Linux environment so JVM unit tests can link native JNI methods:
  ```bash
  cargo build -p phonecam-android-core --release
  ```
- **Gradle Caching & Execution**:
  - Uses `gradle/actions/setup-gradle@v4` with read-only cache on PR branches.
  - Runs `./gradlew test --stacktrace` to execute all JVM tests.
  - Runs `./gradlew assembleRelease --stacktrace` to ensure complete APK compilation.
  - Automatically uploads test reports (`apps/android/app/build/reports/tests/`) on test failures.

---

## 3. Release Automation & Gatekeeper Workflow (`release.yml`)

The release pipeline at `.github/workflows/release.yml` triggers on Git tag creation (`v*`), push to `master`/`main`, or manual `workflow_dispatch`.

### 3.0 Release Gatekeeper Contract (`release-gate.json`)
To prevent accidental or premature releases on every commit, PhoneCam enforces an explicit, atomic **Release Gate**:
- File: `release-gate.json` in the root repository.
- Gate Check: The `check-gate` job executes first before any compiler or packaging job.
- When `release_enabled: false`: Downstream jobs (`build-windows`, `build-linux`, `build-android`, and `publish-release`) are skipped cleanly. Code is tested via CI, but no public release is published.
- When `release_enabled: true`: Downstream build matrix is unlocked, building Windows setup installer, portable ZIP, Linux tarball, Android APKs, and publishing the GitHub release.

```json
{
  "release_enabled": false,
  "version": "1.0.0",
  "approved_by": "RegenadeJester",
  "description": "PhoneCam Release Gatekeeper"
}
```

### 3.1 Windows Release Job (`build-windows`)
1. **Compilation**:
   ```powershell
   cargo build --release -p phonecam-app
   ```
2. **Unpacked Distribution Staging**:
   Creates `dist/unpacked/` and copies `target/release/phonecam.exe`, `PhoneCamVirtualCamera.dll`, `adb.exe`, and batch scripts.
3. **NSIS Installer Packaging**:
   Executes `makensis scripts/installer.nsi`.
   - Locates `makensis` from `PATH`, default NSIS program directory, or installs via Chocolatey.
   - Generates `dist/installer/PhoneCam-Setup-1.0.0.exe`.
4. **Portable Zip Packaging**:
   Archives all files in `dist/unpacked/` into `dist/PhoneCam-v1.0.0-windows-x64.zip` including `run-portable.bat`.
5. **Artifact Upload**:
   Uploads `windows-installer` and `windows-portable` to GitHub Actions artifact storage.

### 3.2 Linux Release Job (`build-linux`)
1. Compiles `phonecam` release binary on `ubuntu-latest`.
2. Packages `dist/PhoneCam-v1.0.0-linux-x64.tar.gz`.
3. Supports Arch Linux, Debian/Ubuntu, and Fedora/DNF via `v4l2loopback`.

### 3.3 Android Release Job (`build-android`)
1. **Host Native Library**:
   ```bash
   cargo build -p phonecam-android-core --release
   ```
2. **Release APK Build**:
   ```bash
   cd apps/android
   chmod +x gradlew
   ./gradlew assembleRelease --stacktrace
   ```
3. **Multi-ABI Artifact Staging**:
   Collects `app-universal-release.apk` (copied to `PhoneCam.apk`), `app-arm64-v8a-release.apk`, `app-armeabi-v7a-release.apk`, and `app-x86_64-release.apk`.
4. **Artifact Upload**:
   Uploads `android-apks` to GitHub Actions artifact storage.

### 3.4 GitHub Release Publication & Automated Notes (`publish-release`)
1. **Artifact Gathering**:
   Downloads all artifacts from previous jobs using `actions/download-artifact@v4`.
2. **Automated Notes & Contributor Profile Avatars**:
   Executes `python3 scripts/generate-release-notes.py <tag> release-notes.md`:
   - Categorizes commits using Conventional Commits (`feat:`, `fix:`, `perf:`, `refactor:`, `vcam:`, `android:`, `docs:`, `ci:`).
   - Generates table of contributors with live GitHub avatar profile pictures (`https://github.com/<username>.png?size=64`), commit counts, and profile links.
   - Integrates `.github/release.yml` for GitHub native release notes categorization and pull request mapping.
3. **Publish via `softprops/action-gh-release@v2`**:
   - Releases under the approved tag (e.g. `v1.0.0`).
   - Attaches:
     - `PhoneCam-Setup-*.exe` (Windows Installer)
     - `PhoneCam-*-windows-x64.zip` (Windows Portable)
     - `PhoneCam-*-linux-x64.tar.gz` (Linux Standalone)
     - `PhoneCam*.apk` (Universal & Multi-ABI Android APKs)

---

## 4. Semantic Versioning & Release Lifecycle

PhoneCam strictly adheres to [Semantic Versioning 2.0.0](https://semver.org/):

```
vMAJOR.MINOR.PATCH
```

- **MAJOR**: Incompatible wire protocol breaks (bumping `PROTOCOL_VERSION` in `phonecam-protocol`), fundamental architectural overhauls, or breaking shared memory framebus layout changes.
- **MINOR**: Backward-compatible features (e.g., new resolution rung, new UI filter/zoom mode, new platform support).
- **PATCH**: Backward-compatible bug fixes, performance optimizations, memory leak fixes, documentation.

### 4.1 Synchronized Version Locations Checklist
When cutting a new release, update the version string across all manifests:

| File | Field / Directive | Example |
| :--- | :--- | :--- |
| `Cargo.toml` | `[workspace.package].version` | `version = "1.0.0"` |
| `Cargo.lock` | Root package versions | `cargo update -p phonecam-protocol --precise ...` |
| `apps/android/app/build.gradle.kts` | `defaultConfig.versionName` | `versionName = "1.0.0"` |
| `apps/android/app/build.gradle.kts` | `defaultConfig.versionCode` | `versionCode = 1` (monotonic integer) |
| `scripts/installer.nsi` | `!define PRODUCT_VERSION` | `!define PRODUCT_VERSION "1.0.0"` |
| `CHANGELOG.md` | Release header | `## [1.0.0] - 2026-11-13` |

### 4.2 Release Tagging Workflow
```powershell
# 1. Ensure working directory is clean and all tests pass
cargo test --workspace
cd apps\android; .\gradlew.bat test; cd ..\..

# 2. Commit version bumps
git add Cargo.toml Cargo.lock apps/android/app/build.gradle.kts scripts/installer.nsi CHANGELOG.md
git commit -m "chore(release): bump version to v1.0.0"

# 3. Create annotated Git tag
git tag -a v1.0.0 -m "release: PhoneCam v1.0.0"

# 4. Push branch and tag to GitHub
git push origin master
git push origin v1.0.0
```

---

## 5. Local CI Validation Commands

Before pushing pull requests, contributors should run local verification simulating CI steps:

```powershell
# 1. Format check
cargo fmt --all -- --check

# 2. Strict Clippy check
cargo clippy --workspace --all-targets -- -D warnings

# 3. Workspace test execution
cargo test --workspace

# 4. Android test execution
cd apps\android
.\gradlew.bat test
cd ..\..

# 5. Local commitlint validation (requires npm/npx)
npx commitlint --from=HEAD~1 --to=HEAD
```
