# ClipBridge Rust v0.8.1 — regression and playhead placement

## Two separate issues reported
(1) New Media rows: the first image in row 2/3/4 appears in Media but
does not reliably reach timeline. v0.7.9 added dynamically shifted row
scans and source-image matching; user's CapCut still showed failures.

(2) Even when a thumbnail reaches timeline, v0.7.9 ignores playhead
position entirely. The '+' action does not guarantee placement at the
user's requested time.

## Implementation (v0.8.1)

New-row handling: scanning now allows ±70 pixels horizontal shift as well
as ±84px vertical shift around Media grid anchors; it only checks boxes
fully inside the Media view. The source-image matcher collapses correlated
neighboring search candidates, so a shifted first card should not be
rejected as many competing cards. A deterministic regression test covers
first card at (176,402) vs old grid (120,374). This DOES NOT establish
real CapCut success; test every first card after row wrap.
- Playhead placement ON by default. Before importing any image, capture
  the full editor IN MEMORY and detect a neutral vertical line PLUS
  corresponding bright marker in the ruler. Require a unique match.
- Find a likely empty track destination around x=playhead, y in the
  visible lower timeline region. Refuse to insert on occupied/unclear lane.
- Import Media and select source-matched thumbnail, as before.
- Re-identify playhead and empty lane before sending the final action.
  Abort if playhead moved during import, instead of dropping at wrong time.
- Attempt a window-scoped synthetic media-to-playhead drag, without
  moving the user's physical mouse cursor.
- After drag, compare pixel change in a small timeline patch, but
  deliberately label as NOT VERIFIED.
- Toggle playhead checkbox OFF to switch to the legacy Media '+' action.
  The old action may produce incorrect time placement; it exists for
  compatibility and easier A/B diagnosis.
- Keep clipboard image sequence tracking, do not reimport same PNG on
  retry, do not auto duplicate a posted Add/drag command.
- Keep project-STT-localdate source PNG filenames, Settings and 3-color icon.

## CapCut live test — required
1. Close all older ClipBridge instances and use an expendable CapCut draft.
2. Set Auto Add ON, Block Native ON, Clear Clipboard OFF.
3. Check "Đặt ảnh tại playhead (thử nghiệm...)" ON.
4. Pause playback and move the playhead to a clearly visible time in
   an EMPTY section of the timeline; keep editor maximized.
5. Copy distinct image A outside CapCut and Ctrl+V once.
   Confirm A is in Media AND its timeline clip starts at playhead.
6. Move playhead to a different time, copy image B and Ctrl+V.
   Confirm B starts at THAT new playhead time.
7. Fill first Media row; test first image in second and third Media row.
8. If preflight cannot detect playhead, no image will be imported in
   playhead mode; this is intentionally safe. Toggle playhead OFF to
   try old '+' mode and distinguish regression.
9. After test, share latest
   `%LOCALAPPDATA%\ClipBridge\clipbridge.log` and screenshot of
   editor with Media, timeline, and playhead visible.
10. Relevant log events:
    `PLAYHEAD_PRECHECK`, `MEDIA_SOURCE_DIAGNOSTIC`,
    `MEDIA_ROW_REFLOW`, `PLAYHEAD_DRAG_STARTED`,
    `PLAYHEAD_DRAG_SENT`, `PLAYHEAD_DRAG_NO_VISIBLE_TIMELINE_CHANGE`,
    `IMPORT_PLACEMENT_COMMAND_SENT`, `IMPORT_FAILED`.

## Honesty/limitations
No automated test can verify CapCut's own Qt drag-and-drop response or
guarantee frame-exact placement for the user's version. Pixel comparison
is diagnostic, not definitive success evidence. No real pointer movement,
no screenshot files, no clipboard rewriting, no draft edits.
EXE is unsigned; scan before running.
