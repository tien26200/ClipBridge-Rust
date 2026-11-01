# ClipBridge Rust v0.7.5: second picture + auto-named PNG on Ctrl+C

## Scenario A — second paste is Media-only (regression)
1. Exit old ClipBridge fully and run 0.7.5.
2. Test in a disposable project with Media visible, regular layout.
3. Copy picture A outside CapCut and Ctrl+V once. Confirm both Media and timeline.
4. Without deleting A, copy picture B outside CapCut and Ctrl+V again.
5. Confirm B appears BOTH in Media and timeline (not only Media).
6. Repeat with C, including after deleting A, and with a nearly-black image.
7. If a clip is missing, provide log events: MEDIA_TILE_FOUND,
   MEDIA_TILE_STABLE, ADD_TO_TRACK_SENT, IMPORT_FAILED.
8. A dispatched WM_LBUTTONUP is not proof of an added timeline clip; please
   confirm visually in CapCut.

Changes: choose the precise changed Media grid slot, not the outer selection
outline; trigger a single hover for (+) followed by a short mouse message
press/release on the CapCut editor HWND. The pointer is NOT physically moved.

## Scenario B — filename on initial copy, NOT rename inside CapCut
1. Open a CapCut project, e.g. "Spa October".
2. Copy image from browser (Ctrl+C).
3. Immediately open ClipBridge's output folder. A new PNG should be named:
   `Spa_October_001_01-11-2026.png` (date reflects your Windows local date).
4. Copy another different image. Expect `Spa_October_002_01-11-2026.png`.
5. Close/reopen ClipBridge, copy a third image: expected STT 003, no overwrite.
6. Create a different project: its STT starts at 001 for the date.
7. Test another calendar date: numbering starts over at 001 for that day.
8. If your CapCut title does not expose the active project, use the new
   "Tên project cho file PNG" field and "Lưu tên project" button, or leave it
   empty for automatic read-only project-name detection.
9. This NEVER renames an image already in CapCut's Media, nor changes the
   CapCut project name. Existing PNG files are left untouched.
10. Review NAMING / WORKER_FINISHED / SAVE_OK in the log if naming looks wrong.

## Settings and limitations
- Existing Auto Add, Block Native Ctrl+V, and Clear Windows Clipboard toggles
  are retained and continue to persist.
- A real CapCut timeline check is mandatory; Rust tests/build alone do not
  demonstrate successful insertion on every CapCut version/layout.
- No draft file edits, no physical cursor motion.
- EXE is unsigned; scan with Microsoft Defender before running.
