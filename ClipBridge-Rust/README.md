# ClipBridge Rust v0.4.1 — CapCut compatibility patch

## Download a Windows EXE using GitHub Actions

This repository includes `.github/workflows/windows-build.yml`. On GitHub, open **Actions** → **Build ClipBridge Windows EXE** → **Run workflow**. The workflow runs Rust tests and compiles the Windows x64 release EXE on GitHub's Windows runner. Once successful, download **ClipBridge-Rust-v0.4.1-Windows-x64** from the run's **Artifacts** section; extract twice (GitHub artifact ZIP, then the bundled ZIP) to obtain `ClipBridge.exe`, `SHA256.txt`, and `TESTING.md`.

An Actions run also starts on pushes to `main` or `master` that modify the source or workflow. If the repository has no `main` or `master`, choose Run workflow on its default branch or adjust the branch filter.

**Important:** The EXE is not included with this source ZIP. It is compiled by the GitHub workflow only after you upload the repository and the run succeeds. Windows + CapCut paste compatibility still requires a real manual test.

## Behavior of the patch

- Copy an image in Windows: ClipBridge automatically saves a PNG file.
- Auto-save **does not replace the original image on the clipboard**, preserving the ability to press Ctrl+V in CapCut's timeline (depending on CapCut's version and its clipboard support).
- **Copy đường dẫn ảnh cuối** explicitly copies the last saved file path as text when pressed; this replaces the clipboard contents, so copy the image again before using Ctrl+V to paste the image.
- The latest saved path updates on the `WM_SAVED` event. Older worker completion events cannot replace a newer capture.
- The application launches minimized to the Windows notification area.

## Build on Windows locally

Requires Rust stable MSVC and Windows SDK with `rc.exe` (Visual Studio C++ Build Tools):

```powershell
cargo test --locked
cargo build --release --locked --target x86_64-pc-windows-msvc
```

The output will be `target\x86_64-pc-windows-msvc\release\clipbridge.exe`.

## Regression tests

See `VALIDATION.md` for a hands-on test checklist, including CapCut timeline paste, auto-save, manual copy-path, and rapid successive copies.
