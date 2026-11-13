## Summary

<!-- Provide a brief description of the changes introduced by this pull request. -->

## Motivation & Context

<!-- Why is this change required? What problem does it solve? -->
<!-- If it fixes an open issue, link to the issue here: Closes #issue_number -->

## Type of Change

- [ ] 🐛 `fix`: Bug fix (non-breaking change which fixes an issue)
- [ ] ✨ `feat`: New feature (non-breaking change which adds functionality)
- [ ] ⚡ `perf`: Performance improvement
- [ ] ♻️ `refactor`: Code change that neither fixes a bug nor adds a feature
- [ ] 📝 `docs`: Documentation updates or additions
- [ ] 🎨 `style`: Code style / formatting changes (white-space, formatting, etc.)
- [ ] 🧪 `test`: Adding or correcting tests
- [ ] 👷 `ci`: Changes to CI/CD workflows or configurations
- [ ] 📦 `build`: Changes to build system or external dependencies
- [ ] 🔧 `chore`: Miscellaneous maintenance tasks

## Affected Components

- [ ] `crates/phonecam-app` (Desktop application & runtime)
- [ ] `crates/phonecam-ui` (GPUI user interface)
- [ ] `crates/phonecam-core` (Network sessions, ADB bridge, discovery)
- [ ] `crates/phonecam-media` (OpenH264 video decoder & NV12/YUV conversions)
- [ ] `crates/phonecam-framebus` (Shared memory ring buffer & directshow IPC)
- [ ] `crates/phonecam-protocol` (Wire protocol, framing, cryptography)
- [ ] `crates/phonecam-android-core` (Rust JNI core for Android)
- [ ] `apps/android` (Android Jetpack Compose camera app)
- [ ] `scripts/` (Packaging, NSIS installer, build scripts)
- [ ] `.github/` (CI/CD workflows, issue templates)

## Checklist

Please verify the following before submitting:

- [ ] **Tests Pass**:
  - [ ] Rust workspace tests pass (`cargo test --workspace`)
  - [ ] Android unit tests pass (`./gradlew test`)
- [ ] **Compilation**:
  - [ ] Rust workspace compiles cleanly without warnings (`cargo check --workspace`)
  - [ ] Android release builds successfully (`./gradlew assembleRelease`)
- [ ] **Semantic Commits**:
  - [ ] Commit message(s) adhere to Conventional Commits specification (validated via `.commitlintrc.json`)
  - [ ] Format: `<type>(<scope>): <subject>` (e.g., `feat(media): optimize NV12 conversion`)
- [ ] **Documentation**:
  - [ ] Relevant documentation or comments updated where appropriate
- [ ] **Safety & Hygiene**:
  - [ ] No temporary files, debug logs, secrets, or keys committed
  - [ ] Wire protocol / IPC backward compatibility verified (if modifying protocol or framebus)

## Testing Steps

<!-- Describe the steps taken to verify the changes manually. -->
1. 
2. 
3. 

## Screenshots / Recordings (if applicable)

<!-- Add screenshots or screen recordings showing UI/UX changes before and after. -->
