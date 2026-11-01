# ClipBridge v0.5.1 — Windows diagnostic log

Windows log location: `%LOCALAPPDATA%\\ClipBridge\\clipbridge.log`
Previous rotated log: `%LOCALAPPDATA%\\ClipBridge\\clipbridge.old.log`

- App writes a bounded asynchronous event log (2 MiB max per file, rotation).
- Events include startup, clipboard sequence, owner process ID, clipboard format IDs, image bytes, image type, worker duration, save result, pause/resume, explicit manual path copy and shutdown.
- No clipboard image bytes, text contents or window titles are written. Filename is a SHA256-derived PNG name, not a private user file path.
- Clipboard reading and saving do not alter clipboard formats.
- Click **Mở log** in the app to open the file.

Reproduction:
1. Exit older ClipBridge instances from tray; start v0.5.1.
2. Copy one image; let ClipBridge save PNG.
3. Attempt Ctrl+V into CapCut Timeline, then into Media; compare with Paint.
4. Open log and inspect CLIPBOARD_UPDATE -> CAPTURE_READ -> WORKER_FINISHED -> SAVE_OK.
5. For privacy, review log before sharing it.

Note: ClipBridge cannot directly observe CapCut accepting or rejecting a paste. Test those interactions separately.
