# Contributing to PhoneCam

Thank you for your interest in contributing to PhoneCam! Whether you are reporting an issue, optimizing media pipelines, improving UI ergonomics, or enhancing documentation, your efforts help make PhoneCam the fastest, most reliable open-source webcam solution.

This document outlines our conventions, development workflows, testing requirements, and code standards.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Development Environment Setup](#development-environment-setup)
- [Conventional Commits Specification](#conventional-commits-specification)
  - [Commit Format](#commit-format)
  - [Types](#types)
  - [Scopes](#scopes)
  - [Good Commit Examples](#good-commit-examples)
- [Coding Standards](#coding-standards)
  - [Rust Standards](#rust-standards)
  - [Android / Kotlin Standards](#android--kotlin-standards)
- [Testing Requirements](#testing-requirements)
  - [Rust Workspace Tests](#rust-workspace-tests)
  - [Android Unit Tests](#android-unit-tests)
- [Pull Request Workflow](#pull-request-workflow)
- [Questions and Support](#questions-and-support)

---

## Code of Conduct

We are committed to providing a friendly, safe, and welcoming environment for everyone, regardless of background, identity, or experience level. Please treat fellow contributors with kindness, empathy, and professional respect.

---

## Development Environment Setup

To contribute to both the desktop application and the Android client, ensure your machine has the following tools installed:

1. **Rust Toolchain**: Rust 1.85 or newer (`stable-x86_64-pc-windows-msvc`).
   ```powershell
   rustup update stable
   rustup component add clippy rustfmt
   ```
2. **Visual Studio C++ Build Tools**: Visual Studio 2022 with the C++ desktop workload and Windows 11 SDK (10.0.22621.0+).
3. **Java Development Kit**: JDK 17 (`JAVA_HOME` pointing to JDK 17).
4. **Android SDK & NDK**:
   - Android SDK 34+.
   - Android NDK 28+ (e.g. `28.2.13676358`).
   - `cargo-ndk`: `cargo install cargo-ndk`.
   - Android targets: `rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android`.

---

## Conventional Commits Specification

PhoneCam strictly enforces the [Conventional Commits](https://www.conventionalcommits.org/) standard. Automated CI systems validate all commit messages and PR titles against this specification.

### Commit Format

```
<type>(<scope>): <subject>

[optional body explaining *why* the change was made]

[optional footer(s), e.g., Closes #123]
```

### Types

- **`feat`**: A new user-facing feature or enhancement.
- **`fix`**: A bug fix or defect resolution.
- **`perf`**: A performance improvement or allocation reduction.
- **`refactor`**: A code change that neither fixes a bug nor adds a feature.
- **`test`**: Adding missing tests or correcting existing tests.
- **`docs`**: Documentation changes only.
- **`style`**: Changes that do not affect the meaning of the code (formatting, white-space, etc.).
- **`build`**: Changes that affect the build system, cargo dependencies, or gradle plugins.
- **`ci`**: Changes to our CI configuration files and scripts (GitHub Actions).
- **`chore`**: Routine tasks, repository maintenance, or tool adjustments.

### Scopes

Always provide a scope indicating which subsystem is modified:

| Scope | Description |
| :--- | :--- |
| `(protocol)` | The frozen wire contract, framing, HMAC-SHA256, packet models |
| `(framebus)` | Windows shared memory, seqlock synchronization, NV12 conversion |
| `(media)` | openh264 in-process decoder, SPS parsing, pacer, color matrix |
| `(core)` | Transport socket listeners, pairing handshake, adaptive bitrate ladder |
| `(ui)` | GPUI views, design system tokens, preview stage, instrument panels |
| `(app)` | Native Windows binary, Win32 tray integration, ADB supervisor, runtime |
| `(android)` | Android Kotlin application, Camera2 pipeline, MediaCodec hardware encoder |
| `(vcam)` | Media Foundation C++ virtual camera driver and registration tool |
| `(deps)` | Dependency updates |

### Good Commit Examples

- `feat(ui): add tabular numbers to fps readout to prevent text jitter`
- `fix(core): hold adaptive quality ladder during pending rung confirmation`
- `perf(media): eliminate heap allocation in NV12 pitch padding loop`
- `docs(readme): add step-by-step cargo-ndk cross-compilation instructions`
- `test(protocol): add unit test for HMAC signature replay prevention`

---

## Coding Standards

### Rust Standards

1. **Formatting & Linter**:
   - Code must be formatted with `rustfmt`:
     ```powershell
     cargo fmt --all -- --check
     ```
   - Code must be clean of Clippy warnings:
     ```powershell
     cargo clippy --workspace --all-targets -- -D warnings
     ```
2. **Safety & Unsafe Code**:
   - All `unsafe` blocks must be annotated with an explicit `// SAFETY:` comment justifying why the invariant cannot be violated.
   - `unsafe` is strictly restricted to shared memory pointer manipulation in `phonecam-framebus`, Win32 API interop in `phonecam-app`, and JNI boundaries in `phonecam-android-core`.
3. **Error Handling**:
   - Never use `unwrap()` or `expect()` in streaming hot paths or network socket loops.
   - Use `thiserror` for structured library crate errors and `anyhow` for top-level runtime orchestration in `phonecam-app`.
4. **Zero-Allocation Hot Path**:
   - Decoded frame processing, seqlock updating, and NV12/BGRA pixel transformation loops must operate on pre-allocated buffers. Avoid vector allocations, boxed closures, or formatting strings on a per-frame basis.
5. **Security & Redaction**:
   - Never log plain cryptographic secrets, tokens, or video bytes. Always apply the `redact()` filter on string outputs before writing to disk logs.

### Android / Kotlin Standards

1. **Android Kotlin Style Guide**:
   - Follow standard Kotlin naming conventions (camelCase for methods and variables, PascalCase for classes).
2. **Camera2 & MediaCodec Lifecycle**:
   - Properly release `CameraDevice` and `MediaCodec` instances when the service transitions to background or unbinds.
   - Always verify available camera characteristics and hardware level before requesting unsupported FPS/resolution combinations.
3. **Threading**:
   - Keep camera capture and hardware encode buffers off the Android main UI thread. Perform socket transmission on dedicated I/O coroutines or background threads.

---

## Testing Requirements

Every Pull Request must pass all test suites before being merged.

### Rust Workspace Tests

Run the complete suite of over 230 tests across all crates:

```powershell
cargo test --workspace
```

To run end-to-end integration tests that spin up the full runtime:

```powershell
cargo test -p phonecam-app --test end_to_end
```

### Android Unit Tests

Run Android and JVM JNI unit tests:

```powershell
cd apps\android
.\gradlew.bat test
```

---

## Pull Request Workflow

1. **Fork & Branch**:
   - Fork the repository and create a descriptive feature branch:
     ```powershell
     git checkout -b feat/adaptive-bitrate-tweaks
     ```
2. **Implement Changes & Tests**:
   - Write clean, self-contained code.
   - Add unit tests for any new logic or bug fixes.
3. **Verify Locally**:
   - Run `cargo fmt --all -- --check`
   - Run `cargo clippy --workspace --all-targets -- -D warnings`
   - Run `cargo test --workspace`
4. **Commit with Conventional Messages**:
   - Create clean, atomic commits adhering to conventional commit guidelines.
5. **Open a Pull Request**:
   - Push your branch to GitHub and open a Pull Request against `main`.
   - Fill out the PR template thoroughly, referencing any related issue tickets.
6. **Code Review & CI**:
   - Ensure all automated GitHub Actions checks turn green.
   - Address any reviewer feedback promptly. Once approved, changes will be merged via squash-and-merge.

---

## Questions and Support

If you have questions about architecture or need guidance:
- Open a discussion or issue on GitHub.
- Consult the architectural blueprints in [`docs/architecture.md`](docs/architecture.md).
- Reference the agent skill guide in [`.agent/skills/phonecam-dev/SKILL.md`](.agent/skills/phonecam-dev/SKILL.md).
