# ClipBridge Rust v0.7.8 — tested logic, live CapCut confirmation pending

## User's v0.7.7 evidence
Clipboard seq=570: source A imported, selected thumbnail detected,
Add message posted. This is not definitive evidence CapCut inserted the clip.
Clipboard seq=579: source B imported, then 22 screenshots showed three
large, competing changes (217/201/198) and no selected tile; no Add sent.
Retrying the same source B re-imported B again and posted an uncertain
Add message using only 5.4 visual-difference score.

## Improvements
1. Import identity is the clipboard sequence + exactly saved PNG path;
   never alter the CapCut project file or PNG after import.
2. Track each sequence as new / Import attempted / Add message sent.
   On a later Ctrl+V for a sequence with Import attempted, find the
   already imported B. Do not submit Ctrl+I or duplicate its Media entry.
3. Build a sparse RGB histogram of source PNG in the existing import worker
   and compare it with visible Media thumbnails. Require a strong unique
   source match. This is approximate, not a pixel-perfect identity test.
4. If source match is unavailable, allow a fallback only if the new
   thumbnail has moved into a selected state with a visual score >=20.
   Prevent low-confidence 5.4-score repeated clicks.
5. If B is not selected (even if NOTHING is selected), select B by direct
   HWND messages, confirm its cyan highlight, then send the known-good
   Add (+) message pair once, without moving the physical pointer.
6. If the import appears to have finished but B is not visible, fail
   safely and log an explanation, rather than clicking another thumbnail.
7. Do not auto-repeat Add for a sequence after sending a click. It
   might have worked; repeating would create timeline duplicates.

## Live regression checklist
- Exit ClipBridge fully (including tray) and start v0.7.8.
- Set Auto Add ON, Block Native ON, Clear Clipboard OFF.
- In a disposable CapCut project with visible Media panel:
  copy image A outside CapCut, Ctrl+V, inspect Media + timeline;
  copy a DIFFERENT image B, Ctrl+V, inspect Media + timeline.
- Repeat Ctrl+V for B after a failed Add: it must NOT import a second B
  into Media, and should attempt to identify the first B.
- If Add already reported "sent", a second Ctrl+V of same clipboard
  will be ignored to avoid duplicate clips. Ctrl+C again to retry.
- Repeat with similar-looking images, B after Media reflow/deletion,
  and alternative CapCut window layouts.
- Review `%LOCALAPPDATA%\ClipBridge\clipbridge.log` for:
  IMAGE_TRACK_CREATED, IMAGE_STATE, SOURCE_FINGERPRINT_READY,
  SOURCE_MATCH_CONFIDENT, MEDIA_SOURCE_MATCH_STABLE,
  MEDIA_SELECTION_STALE, MEDIA_SELECT_CONFIRMED,
  MEDIA_DIAGNOSTIC, IMPORT_FAILED, IMPORT_ALREADY_ADD_SENT.
- Verify PNG files still use project + STT + local date at copy time;
  Settings and circular three-colour icon are unchanged.

## Limitations and safeguards
There is NO verified CapCut public timeline API. A posted HWND click is
not proof of a clip on timeline, so the app must not claim completion.
When the UI metadata and visual evidence are insufficient, the importer
stops without clicking. It might still fail on custom Qt layouts,
offscreen Media thumbnails, DPI scaling and visually similar images.
No screenshots or PNG samples are stored in the diagnostics.
The EXE is unsigned; scan with Windows Defender and test a disposable
project before using it in production.
