# ClipBridge Rust v0.7.1 — Experimental CapCut Timeline Paste

Copy an image in Windows; ClipBridge saves its PNG in a bounded background worker.
When the same image remains on Windows clipboard, pressing Ctrl+V in foreground
CapCut attempts **Import into Media → Add to Track** automatically.

## CapCut action bridge (experimental)
- The keyboard observer never blocks Ctrl+V and never changes clipboard.
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
