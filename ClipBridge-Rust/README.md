# ClipBridge Rust v0.7.5 — Experimental CapCut Timeline Paste

Copy an image in Windows; ClipBridge saves its PNG in a bounded background worker.
When the same image remains on Windows clipboard, pressing Ctrl+V in foreground
CapCut attempts **Import into Media → Add to Track** automatically.

## v0.7.5: Media deletion regression + 3-colour icon

- After a previously imported Media item is deleted, CapCut may reuse its
  exact thumbnail slot for the next import. The vision engine now accepts
  actual pixel changes even when the selected outline stays in the same
  place. A two-frame stability check prevents clicks during grid reflow.
- The native Windows title, taskbar, Alt-Tab and tray icons are embedded at
  build time as a minimalist **circular pink/blue/yellow icon**, in nine sizes.
  This changes the EXE icon without extra runtime dependencies.
- Retains the three persistent Settings toggles and the conditional
  single-paste gate from v0.7.3.

## Copy-time PNG naming (v0.7.5)

When Ctrl+C copies a new image, ClipBridge immediately saves a NEW PNG file in
the configured output folder with the name:

`TênProject_001_01-11-2026.png`, `TênProject_002_01-11-2026.png`, etc.

This is the filename ON DISK when ClipBridge saves the copied image. It is
not an in-place rename of Media or other content inside CapCut. The numbered
file becomes the source used later by the Import → Add-to-Track pipeline.
Counters increment by project and local calendar date, are rebuilt from
existing filenames after restart, and do not overwrite anything on disk.
Do not delete those PNGs: CapCut projects may be referencing them.

Project-name priority: manually entered override in the app (optional), then
a recognized CapCut project name in the Windows window title, then a uniquely
recent, read-only CapCut draft_meta_info.json, else `CapCutProject`.
The explicit override field can be left blank for automatic detection.
Neither CapCut's internal project title nor Media asset names are edited.

## Second paste fix (v0.7.5)

CapCut may append the next imported picture in a second thumbnail slot.
The detector now uses the **actual grid slot** for the (+) hit target rather
than a possibly offset cyan selection rectangle. The bridge sends one
targeted synthetic hover message (no physical cursor movement), a short
press, and a release, allowing Qt to expose/react to the second tile's (+).
Two-frame thumbnail stability checks from v0.7.4 remain. Real desktop
regression verification is required: UI automation cannot promise that
CapCut accepted the Add-to-Track command.

## Settings: three independent checkboxes

Open the ClipBridge window from its tray icon, and tick any combination:

- **Tự thêm ảnh vào CapCut**: Auto Add (ON by default).
- **Chặn Ctrl+V gốc**: block CapCut's native paste of previously copied media when ClipBridge receives an external image (ON by default, recommended).
- **Clear clipboard Windows**: clear the current clipboard after the new image's Add-to-Track command has been sent (OFF by default). This also removes that copied image from Windows Clipboard, but keeps PNG files intact. It is not a substitute for native paste blocking.

These settings survive restart in `%LOCALAPPDATA%\\ClipBridge\\preferences.ini`.
Both block and clear may be ticked together. If block is disabled, native CapCut Ctrl+V may still create an old timeline clip, even if clear is enabled.

## CapCut action bridge (experimental)
- **Fix double paste:** CapCut's native Ctrl+V was previously passed through while
  ClipBridge imported the new image, so an old internally copied Media clip
  appeared on the timeline before the requested image.
- The targeted keyboard hook now **consumes only Ctrl+V** when CapCut is foreground,
  auto-add is enabled and the clipboard sequence matches the current copied image.
- Normal text paste, other apps and Ctrl+C/Ctrl+V within CapCut's own Media workflow
  are passed through. Clipboard data is never replaced.
- Import dialog is cloaked once, without repeatedly focusing/re-enabling editor.
- A dedicated single-job import worker waits for Ctrl+V to be released.
- Sends Ctrl+I only in foreground CapCut/Jianying and hides the Windows File Open dialog.
- Sends the exact saved PNG path to the native dialog.
- Uses a bounded GDI screenshot of the Media panel before and after import.
- Uses aligned Media grid slots and fresh selection to find the imported tile.
- Sends the Add (+) hotspot mouse-message pair directly to the top-level
  editor HWND (rather than an unrelated child); the physical pointer does not move.
- Does not change window dimensions, alter project draft files or install plugins.
- Does **not** create a native CapCut API: this is best-effort UI automation.

**Important:** Verified compilation alone does not prove it works with a given
CapCut version, theme, scaling, Media layout or input-message behavior. The
`IMPORT_ADD_COMMAND_SENT` event means only that the click message was posted.
An actual timeline insertion must be confirmed on the user's CapCut desktop.
Other Ctrl+V operations pass through when no matching fresh image is on clipboard.
If the operation cannot be routed, ClipBridge never intentionally pastes a stale
media item, but OS or version-dependent behavior may still require investigation.

## Test steps
1. Exit older ClipBridge instances. Run the new EXE and keep the experimental
   checkbox enabled.
2. Open a disposable CapCut project with the Media panel showing.
3. Check that Ctrl+I opens the normal Windows file picker when performed manually.
4. Copy an image from a browser or screenshot tool.
5. Switch to CapCut and press Ctrl+V once; keep CapCut foreground through import.
6. Confirm image in Media and on the timeline near the current playhead.
7. If unsuccessful, open `%LOCALAPPDATA%\\ClipBridge\\clipbridge.log`.
   Share the IMPORT_STEP / IMPORT_FAILED lines; no screenshot is written.
8. Turn off experimental Auto Add if the workflow misbehaves.

## Performance and privacy
- PNG auto-save reuses raw PNG when possible and encodes DIB with fast compression.
- Latest image registry avoids directory scanning and re-encoding on paste.
- A single worker serializes imports and prevents rapid duplicate requests.
- Media screenshot polling is reduced; screenshots include only the CapCut Media pane,
  is used in memory, and is not saved or transmitted.
- The log uses a bounded async channel and rotates near 2 MiB.
- The binary is unsigned; scan with Windows Defender before execution.

## Prior art
The user-interface automation approach was adapted to Rust after reviewing the
MIT-licensed **Jiandan** project for Jianying:
https://github.com/SKYCHENV/Jiandan .
Jiandan is an independent third-party project and is not affiliated with ClipBridge.

## Build
`cargo test` and `cargo build --release --target x86_64-pc-windows-msvc`
are run by GitHub Actions in `.github/workflows/windows-build.yml`.
The artifact ZIP contains `ClipBridge.exe`, `SHA256.txt`, and `TESTING.md`.

If import reports failure, do not assume CapCut supports the same UI control
structure as Jianying; adapt only after reviewing the actual desktop behavior.
