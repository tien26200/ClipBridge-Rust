# ClipBridge Rust v0.8.0

Windows clipboard-image capture → named PNG saved immediately at Ctrl+C
→ CapCut Media import → guarded timeline placement.

## Why v0.7.9 landed clips at the wrong time

It never read CapCut's playhead. It sent a click to the Media thumbnail's
small '+' control; CapCut decided the timeline insertion position.
Fixing thumbnail detection alone could never enforce playhead alignment.

## v0.8.0 — two explicit placement modes

**Playhead placement (default ON, experimental):**
- Read the actual vertical ruler/playhead marker in a full-editor,
  in-memory screen capture BEFORE importing.
- Choose a plausible empty timeline video lane near the playhead. Bail out
  BEFORE importing if the marker or lane cannot be identified safely.
- Identify the exact new Media image using source-PNG matching and
  adaptive row-position scanning from v0.7.9; confirm the correct Media
  thumbnail is selected.
- Recheck that the playhead has not moved during import and the lane
  is still safe. Then simulate one window-scoped drag from Media image
  onto the detected playhead x coordinate. Physical mouse stays still.
- Compare the small timeline patch after drag and log its change score.
  A posted drag or a changed patch is NOT proof of successful placement;
  verify visually in the user's CapCut version.
- Never fall back silently to the old '+' click when marker detection
  fails. This prevents known wrong-position additions.

**Legacy Media '+' mode (checkbox OFF):**
- Keeps the original low-level WM_LBUTTONDOWN / WM_LBUTTONUP message pair
  for compatibility. CapCut determines placement; it may be at the wrong
  time. Explicitly opt into this only if you prefer previous behavior.

Neither mode edits the CapCut draft files, moves the physical mouse
pointer, rewrites Windows clipboard or automatically repeats an
unverified Add/drag command. The Media Import and no-duplicate retry
state machine are retained.

## PNG filenames

Each new external image copy is saved to the selected output folder as
`TenProject_001_DD-MM-YYYY.png`,
`TenProject_002_DD-MM-YYYY.png`, etc.
The project name is inferred read-only when possible or entered manually
in Settings. CapCut's internal asset names are NOT renamed.

## Persistent options

- Auto Add (ON)
- Block native CapCut Ctrl+V for external images (ON)
- Clear Windows clipboard after successful Add/drag **command** (OFF)
- Place at playhead (experimental, ON)

Diagnostic log: `%LOCALAPPDATA%\ClipBridge\clipbridge.log`.
Settings: `%LOCALAPPDATA%\ClipBridge\preferences.ini`.
Tri-colour circular Windows/tray icon is unchanged.

### Validation limitations

The app relies on screen pixels and posted window messages, not an official
CapCut timeline API. Some Qt versions or layouts may ignore synthetic drag,
and the new mode may choose to stop if playhead or destination is ambiguous.
Test on a disposable CapCut project. An EXE build and Rust unit tests
cannot prove real timeline insertion. See VALIDATION.md.
