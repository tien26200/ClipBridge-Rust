//! Experimental CapCut / Jianying Media importer and Add-to-Track bridge.
//! Inspired by the public MIT Jiandan implementation:
//! https://github.com/SKYCHENV/Jiandan (source inspected Nov 2026).
//! Uses no physical cursor motion, no project file edits, no clipboard writes.
//! Still uses an invisible Windows file chooser and synthetic window messages.
//! This is NOT a public CapCut API and requires real-version testing.
use crate::capcut_vision::{capture_media, capture_editor, find_imported_media, media_diagnostic, selected_media, needs_select_new_tile, MediaBox};
use crate::timeline_vision::{find_playhead,timeline_drop_target,DropTarget};
use crate::media_reference::Reference;
use std::{
    mem::{size_of,zeroed},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    ptr::null_mut,
    sync::atomic::{AtomicIsize,AtomicU32,Ordering},
    thread,
    time::{Duration,Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::ScreenToClient,
    System::DataExchange::GetClipboardSequenceNumber,
    UI::{
        Accessibility::*,
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*,
    },
};

pub struct ImportJob {
    pub editor: usize,
    pub pid: u32,
    pub image: PathBuf,
    pub sequence: u32,
    pub reuse_media: bool,
    pub at_playhead: bool,
}
pub struct ImportReport {
    pub box_: MediaBox,
    pub visual_change: f64,
    pub elapsed_ms: u128,
    pub placement: &'static str,
    pub playhead_x: Option<i32>,
    pub timeline_visual_change: Option<f64>,
}

// One dedicated import worker serializes operations and owns these flags.
static EXPECTED_PID: AtomicU32=AtomicU32::new(0);
static EXPECTED_EDITOR: AtomicIsize=AtomicIsize::new(0);
static CLOAKED_DIALOG: AtomicIsize=AtomicIsize::new(0);

unsafe fn owned_dialog(hwnd:HWND,pid:u32)->bool {
    if hwnd.is_null() || pid==0 {return false;}
    let mut actual=0u32;
    GetWindowThreadProcessId(hwnd,&mut actual);
    if actual!=pid {return false;}
    let mut class=[0u16;64];
    let n=GetClassNameW(hwnd,class.as_mut_ptr(),class.len() as i32);
    n>0 && String::from_utf16_lossy(&class[..n as usize])=="#32770"
}
unsafe fn cloak_dialog(dialog: HWND, _editor: HWND) {
    if IsWindow(dialog)==0 {return;}
    // Only cloak a dialog once. v0.7.0 repeated foreground changes,
    // EnableWindow and SetWindowPos during dialog events, creating visible
    // focus/redraw jitter in the CapCut editor.
    if CLOAKED_DIALOG.swap(dialog as isize,Ordering::Relaxed)==dialog as isize {
        return;
    }
    let ex=GetWindowLongPtrW(dialog,GWL_EXSTYLE);
    SetWindowLongPtrW(dialog,GWL_EXSTYLE,
        ex|(WS_EX_LAYERED|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW) as isize);
    SetLayeredWindowAttributes(dialog,0,0,LWA_ALPHA);
    // Keep the picker out of the composited screen; no focus stealing or
    // forcing the modal owner enabled while its import is in progress.
    SetWindowPos(dialog,HWND_BOTTOM,-32000,-32000,0,0,
        SWP_NOACTIVATE|SWP_NOSIZE|SWP_NOOWNERZORDER);
}

unsafe extern "system" fn on_dialog_event(
    _:HWINEVENTHOOK,_:u32,hwnd:HWND,object_id:i32,child_id:i32,_:u32,_:u32
) {
    if object_id!=0 || child_id!=0 {return;}
    let pid=EXPECTED_PID.load(Ordering::Relaxed);
    if owned_dialog(hwnd,pid) {
        let editor=EXPECTED_EDITOR.load(Ordering::Relaxed) as HWND;
        if !editor.is_null(){cloak_dialog(hwnd,editor);}
    }
}
struct HookGuard {hook: HWINEVENTHOOK}
impl Drop for HookGuard {
    fn drop(&mut self){
        EXPECTED_PID.store(0,Ordering::Relaxed);
        EXPECTED_EDITOR.store(0,Ordering::Relaxed);
        CLOAKED_DIALOG.store(0,Ordering::Relaxed);
        unsafe {if !self.hook.is_null(){UnhookWinEvent(self.hook);}}
    }
}
unsafe extern "system" fn enum_dialog(hwnd:HWND,param:LPARAM)->i32 {
    let state=&mut *(param as *mut DialogQuery);
    if owned_dialog(hwnd,state.pid) && IsWindowVisible(hwnd)!=0 {
        state.found=hwnd;
        return 0;
    }
    1
}
struct DialogQuery {pid:u32,found:HWND}
unsafe fn find_dialog(pid:u32)->Option<HWND> {
    let mut query=DialogQuery{pid,found:null_mut()};
    EnumWindows(Some(enum_dialog),&mut query as *mut _ as LPARAM);
    if query.found.is_null(){None}else{Some(query.found)}
}
fn input(vk:u16,up:bool)->INPUT {
    INPUT { r#type:INPUT_KEYBOARD, Anonymous:INPUT_0 {
        ki:KEYBDINPUT {wVk:vk,wScan:0,
            dwFlags:if up{KEYEVENTF_KEYUP}else{0},time:0,dwExtraInfo:0}
    }}
}
unsafe fn send_import_shortcut()->Result<(),String> {
    let keys=[
        input(VK_CONTROL,false),input(b'I' as u16,false),
        input(b'I' as u16,true),input(VK_CONTROL,true)
    ];
    if SendInput(keys.len() as u32,keys.as_ptr(),size_of::<INPUT>() as i32)
        !=keys.len() as u32 {
        let release=[input(b'I' as u16,true),input(VK_CONTROL,true)];
        SendInput(2,release.as_ptr(),size_of::<INPUT>() as i32);
        return Err("Windows rejected Ctrl+I input".into());
    }
    Ok(())
}
unsafe fn pump_messages() {
    let mut message:MSG=zeroed();
    while PeekMessageW(&mut message,null_mut(),0,0,PM_REMOVE)!=0 {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }
}

#[derive(Default)]
struct Controls {edits:Vec<HWND>,buttons:Vec<HWND>}
unsafe extern "system" fn enum_controls(hwnd:HWND,param:LPARAM)->i32 {
    if IsWindowVisible(hwnd)==0 {return 1;}
    let c=&mut *(param as *mut Controls);
    let mut name=[0u16;64];
    let n=GetClassNameW(hwnd,name.as_mut_ptr(),name.len() as i32);
    if n>0 {
        let class=String::from_utf16_lossy(&name[..n as usize]);
        if class=="Edit" {c.edits.push(hwnd);}
        else if class=="Button" {c.buttons.push(hwnd);}
    }
    1
}
unsafe fn bounds(hwnd:HWND)->RECT {
    let mut rect:RECT=zeroed();
    GetWindowRect(hwnd,&mut rect);
    rect
}
unsafe fn find_file_controls(dialog:HWND)->Option<(HWND,HWND)> {
    let mut c=Controls::default();
    EnumChildWindows(dialog,Some(enum_controls),&mut c as *mut _ as LPARAM);
    if c.edits.is_empty() || c.buttons.len()<2 {return None;}
    // Standard file picker: filename edit near the bottom, Open immediately
    // to the left of Cancel. Avoid relying on localized button labels.
    c.edits.sort_by_key(|h|bounds(*h).top);
    c.buttons.sort_by_key(|h|bounds(*h).left);
    Some((*c.edits.last()?,c.buttons[c.buttons.len()-2]))
}
unsafe fn send_path(dialog:HWND,path:&PathBuf)->Result<(),String> {
    let (edit,open)=find_file_controls(dialog).ok_or("File dialog controls unavailable")?;
    let utf16:Vec<u16>=path.as_os_str().encode_wide().collect();
    if utf16.len()>4000 {return Err("Image path is too long".into());}
    // WM_CHAR(U+0001) selects existing text in native Windows edit controls.
    SendMessageW(edit,WM_CHAR,1,0);
    for c in utf16 {
        SendMessageW(edit,WM_CHAR,c as usize,0);
    }
    if PostMessageW(open,BM_CLICK,0,0)==0 {
        return Err("Cannot activate file Open button".into());
    }
    Ok(())
}

unsafe fn wait_picker(
    editor: HWND,pid:u32,
    trace:&mut impl FnMut(&str)
)->Result<HWND,String>{
    let deadline=Instant::now()+Duration::from_millis(2400);
    while Instant::now()<deadline {
        pump_messages();
        if let Some(dialog)=find_dialog(pid) {
            cloak_dialog(dialog,editor);
            trace("DIALOG_FOUND_CLOAKED");
            return Ok(dialog);
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err("CapCut did not open a standard Windows file picker (#32770)".into())
}
unsafe fn close_picker(dialog:HWND,editor:HWND) {
    if IsWindow(dialog)!=0 {PostMessageW(dialog,WM_CLOSE,0,0);}
    let _=editor;
}
unsafe fn open_image(
    editor:HWND,pid:u32,path:&PathBuf,
    trace:&mut impl FnMut(&str)
)->Result<(),String>{
    let mut msg:MSG=zeroed();
    PeekMessageW(&mut msg,null_mut(),0,0,PM_NOREMOVE);
    CLOAKED_DIALOG.store(0,Ordering::Relaxed);
    EXPECTED_EDITOR.store(editor as isize,Ordering::Relaxed);
    EXPECTED_PID.store(pid,Ordering::Relaxed);
    let hook=SetWinEventHook(EVENT_OBJECT_SHOW,EVENT_OBJECT_SHOW,
        null_mut(),Some(on_dialog_event),pid,0,WINEVENT_OUTOFCONTEXT);
    let _guard=HookGuard{hook};
    trace(if hook.is_null(){"WIN_EVENT_HOOK_UNAVAILABLE"} else {"WIN_EVENT_HOOK_READY"});
    send_import_shortcut()?;
    trace("CTRL_I_SENT");
    let dialog=wait_picker(editor,pid,trace)?;
    let attempt=(|| {
        let deadline=Instant::now()+Duration::from_millis(1700);
        while Instant::now()<deadline {
            pump_messages();
            if find_file_controls(dialog).is_some(){break;}
            thread::sleep(Duration::from_millis(10));
        }
        if !path.is_file() {return Err("Saved PNG is missing".into());}
        send_path(dialog,path)?;
        trace("FILE_DIALOG_PATH_SENT");
        let close_deadline=Instant::now()+Duration::from_millis(4500);
        while Instant::now()<close_deadline {
            pump_messages();
            if IsWindow(dialog)==0 {trace("IMPORT_DIALOG_CLOSED");return Ok(());}
            thread::sleep(Duration::from_millis(20));
        }
        Err("File picker did not finish import".into())
    })();
    if attempt.is_err(){close_picker(dialog,editor);}
    // The user may have switched away while import ran. Do not steal focus.
    if GetForegroundWindow()==dialog {SetForegroundWindow(editor);}
    attempt
}
unsafe fn window_is_editor(editor:HWND,pid:u32)->bool {
    if IsWindow(editor)==0 {return false;}
    let mut actual=0u32;
    GetWindowThreadProcessId(editor,&mut actual);
    actual==pid
}
unsafe fn editor_is_foreground(editor:HWND)->bool {
    let fore=GetForegroundWindow();
    fore==editor || (!fore.is_null() && GetAncestor(fore,GA_ROOT)==editor)
}
unsafe fn packed_point(point:POINT)->isize {
    let x=point.x as u32 & 0xffff;
    let y=point.y as u32 & 0xffff;
    ((y<<16)|x) as isize
}
// On an unselected second thumbnail the '+' hotspot may not be active.
// Select only the newly changed Media tile, without moving the physical
// cursor. Never select the old highlighted tile.
unsafe fn native_select_media_tile(editor:HWND,b:MediaBox)->Result<(),String>{
    let rect=bounds(editor);
    let point=POINT{
        x:rect.left+(b.left+b.right)/2,
        y:rect.top+(b.top+b.bottom)/2,
    };
    let mut client=point;
    if ScreenToClient(editor,&mut client)==0 {
        return Err("Cannot map new Media thumbnail center".into());
    }
    let pos=packed_point(client);
    if PostMessageW(editor,WM_LBUTTONDOWN,1usize,pos)==0 ||
        PostMessageW(editor,WM_LBUTTONUP,0,pos)==0 {
        return Err("Could not select newly imported Media item".into());
    }
    Ok(())
}

unsafe fn native_add_to_track(editor:HWND,b:MediaBox)
    ->Result<(i32,i32),String>{
    let rect=bounds(editor);
    // Jiandan's verified Jianying implementation sends the WM_LBUTTON pair
    // directly to the editor's top-level HWND. v0.7.0 used WindowFromPoint,
    // which may target an unrelated Qt child, overlay or transparent surface.
    let screen_point=POINT {
        x:rect.left+b.right-12,
        y:rect.top+b.bottom-12,
    };
    let mut client=screen_point;
    if ScreenToClient(editor,&mut client)==0 {
        return Err("Cannot map Add-to-Track button into CapCut editor".into());
    }
    let pos=packed_point(client);
    // Restore the direct top-level HWND click pair from v0.7.4,
    // which was confirmed to insert clips on the user's CapCut.
    // v0.7.5 introduced WM_MOUSEMOVE and delays; this changed the
    // Qt hover state and regressed even the first timeline insertion.
    // No physical pointer movement and no synthetic hover.
    if PostMessageW(editor,WM_LBUTTONDOWN,1usize,pos)==0 {
        return Err("Native Add-to-Track mouse-down message was rejected".into());
    }
    if PostMessageW(editor,WM_LBUTTONUP,0,pos)==0 {
        return Err("Native Add-to-Track mouse-up message was rejected".into());
    }
    Ok((client.x,client.y))
}

// Simulate Qt mouse drag strictly inside the editor HWND message queue;
// never change the user's physical cursor, clipboard or saved CapCut draft.
// Unlike the old plus-button code, the destination is derived from the
// actual timeline playhead marker, not from a Media thumbnail position.
// A failed playhead check must NOT silently fall back to '+' (wrong time).
unsafe fn native_drag_to_playhead(
    editor:HWND,source:MediaBox,target:DropTarget
)->Result<(),String>{
    let rect=bounds(editor);
    let src=POINT{
        x:rect.left+source.left+24,
        y:rect.top+(source.top+source.bottom)/2,
    };
    let dst=POINT {
        x:rect.left+target.x,
        y:rect.top+target.y,
    };
    let mut from=src;
    let mut to=dst;
    if ScreenToClient(editor,&mut from)==0 ||
       ScreenToClient(editor,&mut to)==0 {
        return Err("Cannot map Media or playhead into editor window".into());
    }
    if (to.y-from.y).abs()<100 || to.x<145 {
        return Err("Playhead drop target outside a valid timeline region".into());
    }
    let start=packed_point(from);
    if PostMessageW(editor,WM_LBUTTONDOWN,1usize,start)==0 {
        return Err("Cannot press Media image for playhead placement".into());
    }
    // One local move passes the minimum Qt drag threshold while remaining
    // on the correct media image; subsequent moves follow a single smooth
    // bounded path to the playhead. Synthetic events do not move real cursor.
    let mut path_ok=true;
    for step in 1..=18i32{
        let px=from.x+(to.x-from.x)*step/18;
        let py=from.y+(to.y-from.y)*step/18;
        let point=POINT{x:px,y:py};
        if PostMessageW(editor,WM_MOUSEMOVE,1usize,
            packed_point(point))==0 {
            path_ok=false;
            break;
        }
        thread::sleep(Duration::from_millis(11));
    }
    // Always release even when an intermediate message was rejected, so
    // the target editor does not remain in a synthetic pressed state.
    let released=PostMessageW(editor,WM_LBUTTONUP,0,
        packed_point(if path_ok{to}else{from}))!=0;
    if !path_ok || !released {
        return Err("CapCut window rejected drag path or mouse release".into());
    }
    Ok(())
}
fn timeline_patch_difference(
    before:&crate::capcut_vision::Shot,
    after:&crate::capcut_vision::Shot,
    x:i32,y:i32,
)->f64{
    if before.width()!=after.width()||before.height()!=after.height(){
        return 0.0;
    }
    let mut change=0u64;
    let mut count=0u64;
    for yy in ((y-22).max(0)..(y+23).min(before.height() as i32)).step_by(4){
        for xx in ((x+8).max(0)..(x+100).min(before.width() as i32)).step_by(4){
            let a=before.pixel(xx as usize,yy as usize);
            let b=after.pixel(xx as usize,yy as usize);
            change+=a.0.abs_diff(b.0) as u64+
                a.1.abs_diff(b.1) as u64+
                a.2.abs_diff(b.2) as u64;
            count+=3;
        }
    }
    change as f64 / count.max(1) as f64
}

/// Runs only on a dedicated worker, never inside the Windows keyboard hook.
/// Ok means the add command was SENT, not proof that a clip was inserted.
pub unsafe fn import_then_add(
    job:&ImportJob,mut trace:impl FnMut(&str)
)->Result<ImportReport,String>{
    let started=Instant::now();
    let editor=job.editor as HWND;
    if !window_is_editor(editor,job.pid) || !editor_is_foreground(editor) {
        return Err("CapCut no longer foreground or target window changed".into());
    }
    if !job.image.is_file(){return Err("Copied image not found on disk".into());}
    if GetClipboardSequenceNumber()!=job.sequence {
        return Err("Clipboard changed before import; refusing to add a stale image".into());
    }
    // Preflight the real playhead BEFORE importing. If CapCut's layout
    // lacks an identifiable marker, fail before changing Media.
    let original_playhead=if job.at_playhead{
        let full=capture_editor(editor)?;
        let playhead=find_playhead(&full)
            .ok_or("Cannot identify CapCut playhead; no Media import or timeline action performed")?;
        let drop=timeline_drop_target(&full,playhead)
            .ok_or("Cannot find a safe empty video track at playhead; no import performed")?;
        trace(&format!("PLAYHEAD_PRECHECK x={} y={} marker_score={}",
            playhead.x,drop.y,playhead.score));
        Some(playhead.x)
    }else{None};
    // Wait until the user's physical Ctrl+V is released before Ctrl+I.
    let deadline=Instant::now()+Duration::from_millis(800);
    while Instant::now()<deadline &&
        ((GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000)!=0 ||
         (GetAsyncKeyState(b'V' as i32) as u16 & 0x8000)!=0) {
        thread::sleep(Duration::from_millis(12));
    }
    if (GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000)!=0 {
        return Err("Ctrl still pressed; refusing to inject another shortcut".into());
    }
    let before=capture_media(editor)?;
    trace("MEDIA_BEFORE_CAPTURED");
    // Decoding only a sparse source histogram avoids screenshots on disk and
    // independently identifies B when CapCut still highlights A.
    let source=Reference::from_png(&job.image).ok();
    trace(if source.is_some(){"SOURCE_FINGERPRINT_READY"}else{"SOURCE_FINGERPRINT_UNAVAILABLE using_visual_only"});
    if job.reuse_media {
        // The same clipboard image was imported on an earlier attempt.
        // Never open Ctrl+I again: locate its existing Media thumbnail.
        trace("MEDIA_RETRY_WITHOUT_REIMPORT");
    } else {
        open_image(editor,job.pid,&job.image,&mut trace)?;
        trace("MEDIA_IMPORT_REQUEST_FINISHED");
    }
    let mut found:Option<(MediaBox,MediaBox,f64)>=None; // click box, slot, score
    let mut candidate:Option<(MediaBox,MediaBox,f64)>=None;
    let deadline=Instant::now()+Duration::from_millis(if job.reuse_media {950}else{2700});
    let mut check_count=0;
    let mut last_media=None;
    let selected_before=selected_media(&before);
    while Instant::now()<deadline {
        if !window_is_editor(editor,job.pid){break;}
        let after=capture_media(editor)?;
        check_count+=1;
        let by_source=source.as_ref().and_then(|s|
            s.locate(if job.reuse_media {None}else{Some(&before)},&after));
        // Conservative fallback ONLY when CapCut actually shows a newly
        // selected card. Diff without a matching source can be a grid
        // repaint (seen in seq579) rather than the imported B.
        let visual=if job.reuse_media {None} else {
            let new_selection=selected_media(&after)
                .filter(|s|Some(*s)!=selected_before);
            find_imported_media(&before,&after).and_then(|(rect,score)|{
                let selected=new_selection?;
                if score>=20.0 && rect.overlaps(selected) {
                    Some((selected,selected,score))
                }else{None}
            })
        };
        let chosen=if let Some(m)=by_source {
            Some((m.rect,m.slot,m.change_score,true,m.loss,m.runner_up_loss,m.row_shift))
        } else {
            visual.map(|(rect,slot,score)|(rect,slot,score,false,0.0,0.0,0))
        };
        if let Some((rect,slot,score,matched,loss,runner,row_shift))=chosen {
            if let Some((_,prev_slot,_))=candidate {
                let stable=(slot.left-prev_slot.left).abs()<=8 &&
                    (slot.top-prev_slot.top).abs()<=8 &&
                    (slot.right-prev_slot.right).abs()<=8 &&
                    (slot.bottom-prev_slot.bottom).abs()<=8;
                if stable {
                    found=Some((rect,slot,score));
                    trace(if matched{"MEDIA_SOURCE_MATCH_STABLE"}else{"MEDIA_SELECTED_MATCH_STABLE"});
                    if matched {
                        trace(&format!("SOURCE_MATCH_CONFIDENT loss={loss:.2} runner={runner:.2}"));
                        if row_shift!=0 {
                            trace(&format!("MEDIA_ROW_REFLOW row_shift_px={row_shift} column_first={}",slot.left<245));
                        }
                    }
                    last_media=Some(after);
                    break;
                }
            }
            candidate=Some((rect,slot,score));
        }else{
            candidate=None;
        }
        last_media=Some(after);
        thread::sleep(Duration::from_millis(if job.reuse_media{95}else{130}));
    }
    trace(&format!("MEDIA_CHECKS count={check_count}"));
    trace(&format!("SOURCE_MATCHER enabled={} fallback_min_change=12.0",source.is_some()));
    if let Some(ref after)=last_media {
        // Only bounding boxes, scores and selection coordinates; never
        // export the user's images or screenshots to log files.
        trace(&format!("MEDIA_DIAGNOSTIC {}",media_diagnostic(&before,after)));
        if let Some(ref fingerprint)=source {
            trace(&format!("MEDIA_SOURCE_DIAGNOSTIC {}",fingerprint.diagnostic(after)));
        }
    }
    let (box_,slot,score)=found.ok_or(
        "Exact/new Media item not confidently identified; no Add command sent (see MEDIA_DIAGNOSTIC)"
    )?;
    trace(&format!("MEDIA_TILE_FOUND x={} y={} diff={score:.1} slot={},{},{},{} reuse_media={}",
        box_.right,box_.bottom,slot.left,slot.top,slot.right,slot.bottom,job.reuse_media));
    if !editor_is_foreground(editor) {
        return Err("Focus changed before Add-to-Track; no click sent".into());
    }
    if GetClipboardSequenceNumber()!=job.sequence {
        return Err("Clipboard changed during import; refusing to add a stale image".into());
    }
    let mut target=box_;
    let current_selection=last_media.as_ref().and_then(selected_media);
    // Selection NONE also requires selecting the exact imported card before
    // touching its Add (+) button. That was missing for B in v0.7.7.
    let must_select=current_selection.is_none() ||
        needs_select_new_tile(slot,current_selection);
    if must_select {
        trace("MEDIA_SELECTION_STALE selecting_new_tile=true");
        // Keep the main (+) click unchanged from the user's working 0.7.4.
        // Only perform this preparatory selection for an unselected B/C card.
        native_select_media_tile(editor,slot)?;
        let settle=Instant::now()+Duration::from_millis(720);
        let mut confirmed=false;
        while Instant::now()<settle {
            thread::sleep(Duration::from_millis(95));
            if !editor_is_foreground(editor) || GetClipboardSequenceNumber()!=job.sequence {
                return Err("Focus or clipboard changed while selecting new Media".into());
            }
            let selection_shot=capture_media(editor)?;
            if let Some(selected)=selected_media(&selection_shot) {
                // A newly selected first-column tile can cause CapCut to
                // scroll/reflow AGAIN. In that case its screen coordinates
                // move; verify source content on the current frame instead
                // of trusting the pre-click slot position.
                let matched_now=source.as_ref().and_then(|fingerprint|
                    fingerprint.locate(None,&selection_shot));
                let right_media=if let Some(m)=matched_now {
                    selected.overlaps(m.slot)
                } else {
                    // Older / unsupported source PNGs have no fingerprint.
                    // Only accept the original unchanged Media slot then.
                    source.is_none() && selected.overlaps(slot)
                };
                if right_media {
                    if !selected.overlaps(slot) {
                        trace("MEDIA_SELECTION_RELOCATED after_row_scroll=true");
                    }
                    target=selected;
                    confirmed=true;
                    break;
                }
            }
        }
        if !confirmed {
            trace("MEDIA_SELECT_NOT_CONFIRMED");
            return Err("Target media not verified after selection/row reflow; skipped Add (+) to avoid wrong clip".into());
        }
        trace("MEDIA_SELECT_CONFIRMED target=new_import");
    } else {
        trace("MEDIA_SELECTION_MATCHED target=new_import");
    }
    let (placement,playhead_x,visual_change)=if job.at_playhead{
        let before_drag=capture_editor(editor)?;
        let playhead=find_playhead(&before_drag)
            .ok_or("CapCut playhead disappeared during Media import; refused to place clip")?;
        let saved=original_playhead.ok_or("Playhead precheck missing")?;
        if (playhead.x-saved).abs()>4 {
            return Err("CapCut playhead moved during import; refused to place at wrong time".into());
        }
        let target_drop=timeline_drop_target(&before_drag,playhead)
            .ok_or("Timeline target is occupied or unclear; no drag or Add (+) sent")?;
        trace(&format!("PLAYHEAD_DRAG_STARTED source=({},{}..{},{}),drop_x={},drop_y={}",
            target.left,target.top,target.right,target.bottom,target_drop.x,target_drop.y));
        native_drag_to_playhead(editor,target,target_drop)?;
        thread::sleep(Duration::from_millis(160));
        let after_drag=capture_editor(editor)?;
        let visual=timeline_patch_difference(
            &before_drag,&after_drag,target_drop.x,target_drop.y
        );
        trace(&format!("PLAYHEAD_DRAG_SENT x={} y={} timeline_patch_change={visual:.1} result_not_verified",
            target_drop.x,target_drop.y));
        if visual<4.5 {
            trace("PLAYHEAD_DRAG_NO_VISIBLE_TIMELINE_CHANGE posted_not_confirmed");
        }
        ("playhead_drag",Some(playhead.x),Some(visual))
    }else{
        let (x,y)=native_add_to_track(editor,target)?;
        trace(&format!("ADD_TO_TRACK_SENT target=editor client_x={x} client_y={y} result_not_verified"));
        ("legacy_plus",None,None)
    };
    Ok(ImportReport{
        box_:target,visual_change:score,elapsed_ms:started.elapsed().as_millis(),
        placement,playhead_x,timeline_visual_change:visual_change,
    })
}
