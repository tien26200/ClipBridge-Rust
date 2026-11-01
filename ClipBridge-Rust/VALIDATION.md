# ClipBridge Rust v0.6.0 — Passive timeline bridge foundation

## Current functionality
- Automatically captures Windows PNG/DIB images and saves PNG in a bounded background worker.
- Retains source clipboard formats after automatic save.
- Maintains the last saved image in a Rust TimelineBridge module.
- Observes Ctrl+V using a non-blocking, pass-through Windows low-level keyboard hook.
- Checks whether CapCut is the foreground executable before logging the paste event.
- Does not click, drag, move the mouse, show Import, change the timeline or edit project files.
- CapCut adapter intentionally returns Unsupported until a version-specific integration is independently tested.
- Log: `%LOCALAPPDATA%\\ClipBridge\\clipbridge.log`.

## Why no timeline insertion yet?
Observed: CapCut doesn't accept Explorer PNG Ctrl+V into timeline, but does accept file drag-and-drop into Media.
Windows alone cannot force another application to insert external media into its internal timeline by writing clipboard image/file formats.
An actual adapter must use a supported integration or tested CapCut UI automation with no mouse/viewport changes.
Do not claim Timeline Insert is supported until verified on the target version.

## Regression checklist
1. Exit previous instances, start v0.6.0 and copy an image.
2. Verify PNG save, image clipboard stays intact, tray and log function.
3. Switch to CapCut, press Ctrl+V; no Import or mouse automation should occur.
4. See `CAPCUT_PASTE_OBSERVED result=Unsupported` in log (if keyboard hook was installed).
5. Copy again rapidly; verify latest image and the original clipboard still remain usable.
6. Verify no virtual-key interception or drop-down injection occurs.
