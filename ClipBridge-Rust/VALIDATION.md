# ClipBridge v0.5.2 — Experimental CapCut Auto Import

## Workflow
- Copy an image as before. ClipBridge saves the PNG on its worker.
- When CapCut is brought to the foreground within 45 seconds, ClipBridge sends Ctrl+I to open the Import picker.
- Only if a Windows common File Open dialog (#32770) appears in the CapCut process within 6 seconds, ClipBridge uses Ctrl+L and Unicode keyboard input to enter the saved PNG path and confirm.
- No clipboard changes. No automatic insertion into timeline.
- Toggle Auto Import in the ClipBridge settings if the version of CapCut uses a different dialog or keyboard shortcut.

## Test
1. Close every previous ClipBridge instance.
2. Open CapCut's project; verify Ctrl+I manually opens the Windows file chooser. If it does not, turn Auto Import off.
3. Copy one image in browser. Switch back to CapCut within 45 seconds.
4. Check whether the saved image appears in Media. If not, use Import or drag and drop normally.
5. Open `%LOCALAPPDATA%\\ClipBridge\\clipbridge.log` and inspect IMPORT_* entries.

## Notes
The import path is sent only after a foreground CapCut process and an associated Windows #32770 dialog are detected. This feature is experimental and CapCut versions can vary. A log message IMPORT_DIALOG_PATH_SENT means input was sent, not that import succeeded. Input simulation may be blocked by elevated-process security. The app neither injects into CapCut nor writes to its project database.
