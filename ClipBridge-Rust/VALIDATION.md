# ClipBridge Rust v0.7.9 — row boundary / first column regression

## Reported real-world symptom
The first newly imported thumbnail on a NEW row in CapCut's Media view
appears in Media but cannot be added to the timeline by ClipBridge.
This is a position-related failure, rather than necessarily image B.

## Changes
- Search ±84 vertical pixels around each expected thumbnail row,
  constrained to fully visible Media area. Do not assume the old
  grid y-coordinate is the current visible y-coordinate.
- Compare sampled Media candidates against the saved PNG RGB histogram.
- Group adjacent scan offsets belonging to one physical thumbnail before
  computing margin, otherwise adjacent offsets can falsely be interpreted
  as multiple independent pictures.
- Keep confirmation across two screenshots.
- After selecting thumbnail, *re-find* source on the current UI frame;
  CapCut may have scrolled again during selection. Confirm the selected
  rectangle belongs to the source before pressing Add (+).
- Preserve per-image no-duplicate Import state and original direct click.
- No image pixels are saved; logs contain coordinates and confidence only.

## Must test in CapCut itself
1. Exit old ClipBridge from tray; launch v0.7.9.
2. Set Auto Add ON; Block Native ON; Clear Clipboard OFF.
3. Start a disposable project, show Media panel and keep its width fixed.
4. Copy/paste enough DIFFERENT PNGs to fill row 1.
5. Copy/paste the FIRST photo of row 2; check that it enters both Media
   and timeline, not only Media.
6. Repeat at the FIRST photo of rows 3 and 4, and after deleting an old
   Media item or after CapCut scrolls the list.
7. Retry Ctrl+V after unsuccessful Import: should try existing Media
   without reimporting duplicated PNGs.
8. Check names `Project_001_DD-MM-YYYY.png` continue incrementing,
   Settings persist and the three-colour circular icon is unchanged.
9. If it fails, share
   `%LOCALAPPDATA%\ClipBridge\clipbridge.log`. Search the failing seq for
   `MEDIA_SOURCE_DIAGNOSTIC`, `MEDIA_ROW_REFLOW`,
   `MEDIA_SELECTION_RELOCATED`, `MEDIA_SELECT_CONFIRMED`,
   `MEDIA_SELECT_NOT_CONFIRMED`, `ADD_TO_TRACK_SENT` and `IMPORT_FAILED`.
10. A command logged as ADD_TO_TRACK_SENT means only the HWND click was
    posted. It does NOT verify a new timeline clip was actually created.

## Limits
The source match is an approximate RGB histogram, not an exact identity
proof. Media thumbnails must remain visible and sufficiently distinguishable
in CapCut's specific version, zoom and layout. It may refuse to add when
several candidates look similar. The executable is unsigned.
