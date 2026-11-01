# ClipBridge Rust v0.7.7 — second image to timeline

## Observed on the user's CapCut
v0.7.6: first image may import into Media and reach timeline, but the
second copied picture enters Media without being added to timeline.

## Source-level failure mode addressed
The v0.7.6 Media detector treated a selected cyan outline as the target,
but CapCut can keep the outline on A after importing B. Its fallback
required difference score >=22 before trying new slot B; many legitimate
thumbnails change the pixels by less. Also Add (+) may not accept messages
for an unselected new tile. These are code-path explanations, not a
verified diagnosis of the user's specific CapCut version without its log.

## Change
1. Rank thumbnail cell changes in memory and require concentration in a
   unique changed slot; permit score >=7 for an otherwise stable UI.
2. If the old selected rectangle A does not overlap new changed slot B,
   derive B's click geometry from A's known selection outline, preserving
   the right/bottom hotspot offsets of the working first import.
3. In that case only, click the CENTER of B by top-level window messages,
   wait up to 720ms for the cyan selection to switch to B, then send the
   standard, unmodified (+) Add-to-Track message pair exactly once.
4. Log Media candidates, selection changes and click coordinates, never
   image pixels; stop if the new selection cannot be confirmed.
5. Keep clipboard conditional paste blocking, optional clear, project/
   STT/date PNG filenames at the moment of copy, and circular app icon.

## Required CapCut regression check
1. Exit the old ClipBridge from tray; run v0.7.7 in a disposable project.
2. Copy image A from browser. Ctrl+V in CapCut. Check Media AND timeline.
3. Copy image B from browser. Ctrl+V once. Check timeline. Do NOT delete A.
4. Copy image C. Ctrl+V. Check timeline.
5. Delete a Media thumbnail, then copy/paste D. Check timeline.
6. Verify PNG filenames retain project/001/002/003 and local date.
7. If B stays Media-only, send the tail of:
   `%LOCALAPPDATA%\ClipBridge\clipbridge.log`.
   Useful events:
   `MEDIA_DIAGNOSTIC`, `MEDIA_SELECTION_STALE`,
   `MEDIA_SELECT_CONFIRMED`, `MEDIA_SELECT_NOT_CONFIRMED`,
   `MEDIA_TILE_FOUND`, `ADD_TO_TRACK_SENT`, `IMPORT_FAILED`.
8. Run manual Ctrl+C → Ctrl+V within CapCut to confirm native paste still works.

## Cautions
Unit tests and Windows Release build validate code and packaging, but do NOT
confirm CapCut actually accepted a message or inserted a timeline clip.
No screenshots/pixel data are written to disk.
The EXE is unsigned; scan with Windows Defender.
