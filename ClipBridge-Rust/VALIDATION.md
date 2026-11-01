# ClipBridge v0.5.0 — clipboard preservation and performance checks

The auto-save event does **not** call EmptyClipboard or SetClipboardData. Source formats remain available to applications that support them. This does not establish that CapCut accepts native clipboard images on its timeline.

## Windows regression tests

1. Close every previous ClipBridge instance before launching the new EXE.
2. Copy an image; verify it is saved as PNG.
3. Paste into Paint and verify the image clipboard is preserved after saving.
4. Test CapCut Ctrl+V into Media and Timeline independently. Record CapCut version and outcomes.
5. Copy multiple images quickly; verify no freeze and that latest path tracks latest completed copy.
6. Press the explicit Copy Path button, verify the path in Notepad. Copy an image again to restore the native clipboard.
7. Test DIBV5 transparency, large images, duplicate images, pause/resume and tray recovery.

## Performance

PNG reuse avoids re-encoding; DIB encoding uses fast compression; file writes use temp-file rename without forcing disk sync on every saved PNG. Some formats require synchronous snapshot copying before clipboard ownership changes.

## Limitations

CapCut timeline paste is controlled by CapCut. A successful compile or PNG save does not prove timeline compatibility. For reliable media insertion when CapCut rejects Windows clipboard data, a tested import automation or supported CapCut integration is required.
