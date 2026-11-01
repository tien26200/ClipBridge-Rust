# ClipBridge Rust v0.7.1 — experimental hidden Import → Add to Track

## What changed
On Ctrl+V while CapCut is foreground and the current Windows clipboard still contains
the latest auto-saved image, ClipBridge attempts to:
1. Use the existing PNG file from the worker (no re-encode on paste).
2. Send CapCut's Ctrl+I shortcut after physical Ctrl+V is released.
3. Detect and hide CapCut's native Windows file picker, fill only that PNG path.
4. Compare the Media panel before/after import to identify a new selected thumbnail.
5. Find the newly selected tile using aligned Media grid slots.
6. Send the Add-to-Track click message to the editor's top-level HWND.
7. Keep the Windows cursor stationary and avoid repeated focus thrashing.
8. Log each stage without writing clipboard contents or screenshots to disk.

The pointer stays in place; application windows are not resized.
This is an experimental UI-message automation, not a native CapCut API.
It creates an invisible dialog and sends mouse **messages** without moving the mouse.
The CapCut International layout and hidden importer still need real-machine testing.
Do NOT assume a successful build proves the timeline insert worked.

## Test
1. Close every old ClipBridge instance and run v0.7.1.
2. Use a disposable CapCut project with Media panel visible and normal window size.
3. Ensure Ctrl+I manually opens Windows File Open dialog; if not, uncheck Auto Add.
4. Copy a fresh picture from Chrome; do not click Copy Path.
5. Switch to CapCut Media and press Ctrl+V just once. Keep CapCut foreground until done.
6. Check whether PNG appears in Media and timeline at playhead.
7. Open `%LOCALAPPDATA%\\ClipBridge\\clipbridge.log`, inspect IMPORT_STEP, IMPORT_FAILED and IMPORT_ADD_COMMAND_SENT.
8. Retry with different image sizes and a different window layout. Cancel the feature if tile coordinates are unreliable.

## Safety and performance
- Bounded one-slot import worker, no work on the keyboard hook.
- Clipboard untouched on auto-save and on paste.
- No physical cursor movement, no disk draft edits, no background folder scans.
- Fail closed on no image, wrong foreground process, unrecognized import dialog,
  uncertain thumbnail, busy queue or focus changes.
- `IMPORT_ADD_COMMAND_SENT` confirms only message dispatch, **not** verified insertion.
- v0.7.0 did import into Media but failed to add to timeline for the tester;
  v0.7.1 changes HWND targeting and tile geometry; it still requires a real test.
- Turn off the experimental checkbox if the CapCut version differs.
- Antivirus scan recommended; executable is unsigned.

## Prior art
Algorithm inspired by the MIT-licensed Jiandan source project:
https://github.com/SKYCHENV/Jiandan .
