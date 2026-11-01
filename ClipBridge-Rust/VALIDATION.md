# ClipBridge Rust v0.7.2 — fix duplicated old image when pasting

## Regression observed in v0.7.1

Pressing Ctrl+V with a newly copied image sometimes created two clips: first a
previous CapCut Media item, then the current image. Source-level root cause:
the keyboard hook always called CallNextHookEx for Ctrl+V while concurrently
sending ClipBridge's Import -> Add-to-Track request.

## Fix v0.7.2

1. Intercept the physical V-down only when Ctrl is held, CapCut/Jianying is
   foreground, Auto Add is enabled, and Windows clipboard sequence equals
   ClipBridge's last captured image sequence.
2. Queue a single ClipBridge paste request and return nonzero from the hook,
   preventing CapCut's own Ctrl+V from inserting a stale internal item.
3. Suppress the matching V-up and autorepeat; do not globally block the keyboard.
4. Copy/Paste with text or with CapCut Media internal objects remains untouched.
5. Capture and route an image even while the PNG save worker is finishing; once
   saved, import the exact sequence that was requested.
6. If the clipboard changes before or during import, refuse to add the stale
   file to timeline. No clipboard writes and no project draft edits.
7. The v0.7.1 Media tile + button targeting and reduced focus jitter remain.

## Test checklist

- Close previous ClipBridge, launch v0.7.2; use a disposable CapCut project.
- Copy media tile A inside CapCut, place one instance manually on timeline.
- Copy NEW image B from the browser, then Ctrl+V in CapCut once:
  **only B** should be imported and added, with no additional A.
- Repeat with another image C; verify that B is not inserted again.
- In CapCut, select a Media item and manually Ctrl+C then Ctrl+V:
  internal CapCut paste should still work.
- Copy text and paste it into Notepad and other apps; verify normal behavior.
- Try holding Ctrl+V to verify a single import per V press.
- Check `%LOCALAPPDATA%\ClipBridge\clipbridge.log` for
  `CAPCUT_CTRL_V_SUPPRESSED`, `IMPORT_STARTED`, `IMPORT_ADD_COMMAND_SENT`,
  or `IMPORT_FAILED`.
- If pressing Ctrl+V produces no clip, verify Auto Add is enabled, the image
  was captured, and the clipboard did not change.

Release build and tests do **not** establish that native CapCut inserted a clip:
`IMPORT_ADD_COMMAND_SENT` means only that an input message was posted.
Real desktop regression test is required before calling the bug resolved.

The EXE is unsigned; scan it with Microsoft Defender before use.
