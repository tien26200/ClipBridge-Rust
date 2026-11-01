# ClipBridge Rust v0.7.6 — urgent timeline insertion regression fix

v0.7.5 was reported to add NONE of the images to the timeline (Media only).
Reviewing the precise source change revealed two experimental differences
from the earlier 0.7.4 build which had inserted some images successfully:
1. A new `WM_MOUSEMOVE` hover plus timing delays before Add (+) click.
2. The inferred grid bounding box replacing CapCut's selection outline,
   changing the click coordinate for the first/selected image.

## Fix
- Remove the synthetic hover and return to v0.7.4's direct
  `WM_LBUTTONDOWN` + `WM_LBUTTONUP` pair on the top-level editor HWND.
- Restore the selected Media thumbnail rectangle as click geometry when
  it overlaps the freshly changed tile; retain conservative second-tile
  detection fallback and two-observation stability check.
- Keep v0.7.5's feature: save the original PNG immediately at Ctrl+C
  as `TênProject_001_DD-MM-YYYY.png`, sequential per project and day.
  Existing files are never renamed or overwritten; CapCut Media names are
  not directly modified.
- Keep Settings checkboxes (Auto Add, Block Native, Clear Clipboard).
- Keep circular three-colour Windows app icon.

## User regression steps
1. Exit every old ClipBridge instance; launch 0.7.6.
2. In a test CapCut project, copy A from browser and Ctrl+V in editor.
   Confirm A appears in BOTH Media and timeline.
3. Copy B from browser and Ctrl+V again. Confirm B appears in timeline.
4. Remove older Media clip A, copy C, and Ctrl+V; confirm timeline.
5. Verify output folder has consecutive project-numbered date PNGs
   created at time of copying images, not after pasting.
6. If no clip is added, send the log:
   `%LOCALAPPDATA%\ClipBridge\clipbridge.log`.
   Log lines: MEDIA_TILE_FOUND, MEDIA_TILE_STABLE,
   ADD_TO_TRACK_SENT, IMPORT_FAILED, IMPORT_ADD_COMMAND_SENT.

## Limitations
- A successful Rust test/build verifies implementation/packaging only,
  NOT successful CapCut insertion on the user's version.
- This reverts the regressions but does not prove every second-tile layout
  is handled. If B still stays in Media, exact log/CapCut layout needed.
- EXE unsigned; scan with Microsoft Defender.
