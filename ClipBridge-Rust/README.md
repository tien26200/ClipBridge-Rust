# ClipBridge Rust v0.6.0

Lightweight Windows clipboard PNG auto-save utility with a **passive**, experimental CapCut timeline bridge foundation.

## Current behavior
- Watches Windows clipboard image changes (PNG, DIBV5, DIB), writes PNG asynchronously.
- Deduplicates images by content hash and retains source clipboard on auto-save.
- Background tray application, configurable image folder, manual copy-path button.
- Rotating asynchronous diagnostic log: `%LOCALAPPDATA%\\ClipBridge\\clipbridge.log`.
- Observes Ctrl+V when CapCut is foreground, without consuming the keystroke or moving mouse.
- Adds `src/timeline_bridge.rs` interface for future CapCut-version-specific integration.

## Not yet implemented
**Ctrl+V does NOT insert external images directly into the CapCut timeline.** The built-in adapter deliberately returns `Unsupported`. User testing showed CapCut did not accept Explorer PNG clipboard paste into the timeline, although manual drag-and-drop into Media worked. No verified native timeline insertion interface is available for this CapCut version. This release does not invoke Import, modify project drafts, inject code into CapCut, or simulate mouse dragging.

## Windows build
GitHub Actions: `.github/workflows/windows-build.yml`. Download the latest successful artifact `ClipBridge-Rust-v0.6.0-Windows-x64`, containing `ClipBridge.exe`, `SHA256.txt`, and `TESTING.md`. Extract once. The executable is not code-signed. Scan it with Windows Defender before running.

## Logs / troubleshooting
Start ClipBridge, copy a PNG/DIB image, then check `SAVE_OK` in the log. Ctrl+V in focused CapCut should emit `CAPCUT_PASTE_OBSERVED result=Unsupported` if the hook is installed. No keys should be blocked and no windows should open automatically. Do not confuse this log event with successful timeline insertion.

## Next integration step
A supported or verified CapCut timeline adapter is required for inserting PNG at the playhead without modifying the mouse or workspace layout. Verify the exact CapCut Desktop version and its accessible integration points before enabling this feature.
