# ClipBridge Rust v0.7.8

Windows clipboard image → saved PNG → hidden CapCut import → optional
Add-to-Track in the CapCut editor, **without moving the physical pointer**.

## v0.7.8 second-picture safety fix

Based on the supplied `clipbridge.log`:
- Image A (sequence 570) was imported and its Add (+) message posted.
- Image B (sequence 579) was imported but Media identification failed
  after 22 screenshots, with multiple reflowing Media slots.
- Pressing Ctrl+V again re-imported B, creating duplicate Media candidates.

To address this, the image worker tracks the current clipboard sequence
and saved PNG path, plus whether Import or Add was already attempted.
It now checks visible Media cards against a sparse RGB histogram decoded
from that exact saved PNG. This match is approximate, so the app will
fail closed if images are too similar, offscreen, or the UI is ambiguous.
A conservative selection-based fallback remains for the first import.
If the newly imported thumbnail is not selected, ClipBridge selects and
confirms that exact card before posting the Add (+) message.

If the first Import succeeded but Add could not be identified, the next
Ctrl+V of the SAME clipboard sequence searches Media again **without**
re-importing B. If an Add command was posted, a repeat Ctrl+V does not
post another Add: copy the source image again if intentional repeat is
needed, to avoid accidentally adding duplicate timeline clips.
No global clipboard clearing, CapCut draft edits or physical cursor moves.

**Status is intentionally 'Add command sent, not verified'.** There is
no verified direct timeline API or reliable proof from one HWND message
that CapCut inserted a clip. Test in a disposable CapCut project.

## PNG filename, at time of Ctrl+C

New image copies are saved as
`TenProject_001_DD-MM-YYYY.png`,
`TenProject_002_DD-MM-YYYY.png`, etc. Existing files are never renamed
or overwritten. If automatic read-only CapCut project name detection
cannot determine the active draft, use the optional manual project
name field in ClipBridge. CapCut Media asset names are not edited.

## Settings
- **Tự thêm ảnh vào CapCut khi Ctrl+V** — ON by default.
- **Chặn Ctrl+V gốc khi thêm ảnh mới** — ON (recommended).
- **Clear Windows Clipboard after Add command** — OFF. This option clears
  only Windows Clipboard, not older Media assets; keep it OFF for testing.

Settings persist at `%LOCALAPPDATA%\ClipBridge\preferences.ini`.
The app retains its small tri-colour circular Windows/tray icon.
Diagnostics: `%LOCALAPPDATA%\ClipBridge\clipbridge.log`.
Refer to `VALIDATION.md` for detailed regression scenarios.
