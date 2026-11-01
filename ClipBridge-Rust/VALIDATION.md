# ClipBridge Rust v0.7.3 — selectable Ctrl+V blocking and clipboard clearing

## Settings (ClipBridge tray icon → Mở ClipBridge)

- **Tự thêm ảnh vào CapCut khi Ctrl+V**: Auto Add, enabled by default.
- **Chặn Ctrl+V gốc khi thêm ảnh mới**: enabled by default. Only
  intercepts the V key for the exact current captured clipboard image
  while CapCut foreground. It avoids CapCut pasting previously copied media.
- **Clear clipboard Windows sau khi gửi lệnh thêm ảnh**: disabled by default.
  Optional cleanup after the successful Add-to-Track command. It clears
  Windows clipboard only if sequence still matches the image being imported.
  If another image/text is copied in the meantime, it does NOT clear it.
  This option does NOT remove old media, previous copied project state or PNG files.

All three toggles persist in `%LOCALAPPDATA%\ClipBridge\preferences.ini`.
All options can be toggled independently. Both protections can be selected
at the same time; native Ctrl+V blocking still owns the anti-duplicate path.
When blocking is OFF, CapCut's normal Ctrl+V can still paste an old media
clip in addition to ClipBridge's imported image — clearing afterward does
not prevent that. Keep blocking enabled for the normal workflow.

## Quick regression checklist
1. Exit older ClipBridge fully and launch v0.7.3.
2. Open Settings and verify defaults: Auto Add ON, Block Native ON, Clear OFF.
3. Copy image from browser, Ctrl+V in CapCut. Only the new image should
   be added (not previously copied CapCut media).
4. Enable Clear, copy a NEW image, Ctrl+V once. Check timeline and
   clipboard emptiness after the Add-to-Track message is sent.
5. Turn Clear OFF, copy a NEW image, Ctrl+V. Image should remain in
   Windows Clipboard after import.
6. Switch Block OFF and copy a NEW image, Ctrl+V. ClipBridge should still
   import the image, but CapCut may also paste an old media clip.
7. Copy text and paste into Notepad; verify normal behavior.
8. Copy an existing media tile within CapCut then Ctrl+V; its native
   copy/paste must still work.
9. Restart ClipBridge and verify settings have persisted.
10. For errors send `%LOCALAPPDATA%\ClipBridge\clipbridge.log` events:
    `CAPCUT_PASTE_ROUTED`, `IMPORT_STARTED`,
    `IMPORT_ADD_COMMAND_SENT`, `CLIPBOARD_CLEARED`.

## Notes
- The copied PNG files are never removed: CapCut projects may reference them.
- Clear executes AFTER an Add-to-Track message has been sent, not after a
  verified timeline insertion. Real CapCut visual confirmation is still required.
- The EXE is unsigned. Test on a disposable CapCut project.
