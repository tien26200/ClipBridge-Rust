# ClipBridge Rust v0.7.9

## Fix — first image when CapCut Media starts a new row

The user identified that the FIRST thumbnail in a newly filled Media row
often imports correctly into Media but is not added to timeline. Previous
ClipBridge builds assumed all thumbnails stayed on a rigid 136px-by-112px
grid. CapCut can scroll/reflow the Media panel when a row fills; a changed
rectangle may not be the newly imported thumbnail.

v0.7.9:
1. Adds a bounded ±84px vertical scan in 14px steps around each known
   Media grid anchor (only fully visible parts of the Media panel).
2. Scores those candidates against the *actual saved PNG* source, rather
   than trusting changed pixels alone. It collapses nearby scan offsets
   from the SAME thumbnail before calculating the confidence margin.
3. Logs the source match confidence and row offset as numbers, without
   logging image content or screenshots.
4. If selecting the new thumbnail causes CapCut to scroll again, re-runs
   source matching against the CURRENT screen and confirms the selection
   still belongs to the source before clicking Add (+).
5. Keeps the exact v0.7.4 low-level Add-to-Track down/up message pair,
   and the v0.7.8 no-duplicate re-import state machine.

The algorithm deliberately fails closed on ambiguous or offscreen media
rather than pressing Add (+) on an older image. No Windows mouse pointer
movement, no edits to CapCut drafts.

## Filename when copying images

ClipBridge saves PNGs as `TenProject_001_DD-MM-YYYY.png`, numbered per
project/day when Ctrl+C fires outside CapCut. Existing files are never
renamed or overwritten. If automatic project detection is not accurate,
provide a manual name in ClipBridge's window.

## Settings

- Auto Add on Ctrl+V (ON by default)
- Block native CapCut Ctrl+V for new copied image (ON by default)
- Optionally clear Windows Clipboard after sending Add (OFF by default)
- Circular pink/blue/yellow Windows/tray icon unchanged.

Settings and diagnostic log live in `%LOCALAPPDATA%\ClipBridge`.
This is Windows GUI automation, not a verified CapCut API.
A successful test/build is not proof a clip was inserted into timeline.
Run the checks in `VALIDATION.md` on a disposable CapCut project.
