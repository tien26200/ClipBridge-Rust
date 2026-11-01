# v0.4.1 Patch validation

- Source-level review: remove automatic `EmptyClipboard` calls during WM_SAVED.
- Manual button still copies a CF_UNICODETEXT path only on click.
- UI status and hints describe direct CapCut timeline paste.
- WM_SAVED updates `latest` only if save event sequence is not older.
- **Windows compilation and CapCut paste must still be tested on Windows.** No updated EXE is included or claimed.

## Manual regression checklist

1. Start patched ClipBridge and CapCut (same CapCut version used with old build).
2. Copy a web image, wait for saved status, Ctrl+V into CapCut timeline.
3. Verify PNG was saved and clipboard still has original image formats.
4. Copy two images rapidly and verify the most recent saved image path wins.
5. Click Copy đường dẫn, paste into Notepad or Ctrl+I file dialog; path matches latest PNG.
6. Copy an image again; confirm timeline paste is restored after manual path copy.
7. Check pause/resume, tray icon, offline folder and restart handling.
