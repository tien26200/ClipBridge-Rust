//! Experimental CapCut / Jianying Media importer and Add-to-Track bridge.
//! Inspired by the public MIT Jiandan implementation:
//! https://github.com/SKYCHENV/Jiandan (source inspected Nov 2026).
//! Uses no physical cursor motion, no project file edits, no clipboard writes.
//! Still uses an invisible Windows file chooser and synthetic window messages.
//! This is NOT a public CapCut API and requires real-version testing.
use crate::capcut_vision::{capture_media, find_imported_media, media_diagnostic, selected_media, needs_select_new_tile, MediaBox};
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
}
pub struct ImportReport {
    pub box_: MediaBox,
    pub visual_change: f64,
    pub elapsed_ms: u128,
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
    open_image(editor,job.pid,&job.image,&mut trace)?;
    trace("MEDIA_IMPORT_REQUEST_FINISHED");
    let mut found=None;
    let mut candidate:Option<(MediaBox,f64)>=None;
    let deadline=Instant::now()+Duration::from_millis(3200);
    let mut check_count=0;
    let mut last_media=None;
    while Instant::now()<deadline {
        if !window_is_editor(editor,job.pid) {break;}
        let after=capture_media(editor)?;
        check_count+=1;
        let by_source=source.as_ref().and_then(|s|s.locate(&before,&after));
        let chosen=if let Some(m)=by_source {
            Some((m.rect,m.change_score))
        } else {
            // Never auto-click a barely changed selected border (the 5.4
            // score in the user's failed seq579 retry): it may be the old
            // Media item reselected while the actual B is elsewhere.
            find_imported_media(&before,&after)
                .filter(|(_,score)|*score>=12.0)
        };
        if let Some((b,score))=chosen {
            // Two agreeing observations distinguish the actual new image
            // from CapCut's first-frame selection/thumbnail redraw.
            if let Some((prev,_))=candidate {
                let stable=(b.left-prev.left).abs()<=8 &&
                    (b.top-prev.top).abs()<=8 &&
                    (b.right-prev.right).abs()<=8 &&
                    (b.bottom-prev.bottom).abs()<=8;
                if stable {
                    found=Some((b,score));
                    trace("MEDIA_TILE_STABLE confirmed=2");
                    last_media=Some(after);
                    break;
                }
            }
            candidate=Some((b,score));
        } else {
            candidate=None;
        }
        last_media=Some(after);
        thread::sleep(Duration::from_millis(130));
    }
    trace(&format!("MEDIA_CHECKS count={check_count}"));
    trace(&format!("SOURCE_MATCHER enabled={} fallback_min_change=12.0",source.is_some()));
    if let Some(ref after)=last_media {
        // Only bounding boxes, scores and selection coordinates; never
        // export the user's images or screenshots to log files.
        trace(&format!("MEDIA_DIAGNOSTIC {}",media_diagnostic(&before,after)));
    }
    let (box_,score)=found.ok_or("Imported thumbnail not confidently identified; check MEDIA_DIAGNOSTIC")?;
    trace(&format!("MEDIA_TILE_FOUND x={} y={} diff={score:.1}",box_.right,box_.bottom));
    if !editor_is_foreground(editor) {
        return Err("Focus changed before Add-to-Track; no click sent".into());
    }
    if GetClipboardSequenceNumber()!=job.sequence {
        return Err("Clipboard changed during import; refusing to add a stale image".into());
    }
    let mut target=box_;
    let current_selection=last_media.as_ref().and_then(selected_media);
    if needs_select_new_tile(box_,current_selection) {
        trace("MEDIA_SELECTION_STALE selecting_new_tile=true");
        // Keep the main (+) click unchanged from the user's working 0.7.4.
        // Only perform this preparatory selection for an unselected B/C card.
        native_select_media_tile(editor,box_)?;
        let settle=Instant::now()+Duration::from_millis(720);
        let mut confirmed=false;
        while Instant::now()<settle {
            thread::sleep(Duration::from_millis(95));
            if !editor_is_foreground(editor) || GetClipboardSequenceNumber()!=job.sequence {
                return Err("Focus or clipboard changed while selecting new Media".into());
            }
            let selection_shot=capture_media(editor)?;
            if let Some(selected)=selected_media(&selection_shot) {
                if selected.overlaps(box_) {
                    target=selected;
                    confirmed=true;
                    break;
                }
            }
        }
        if !confirmed {
            trace("MEDIA_SELECT_NOT_CONFIRMED");
            return Err("New Media tile not selected; refused to click stale Add (+) target".into());
        }
        trace("MEDIA_SELECT_CONFIRMED target=new_import");
    } else {
        trace("MEDIA_SELECTION_ALREADY_MATCHED_OR_UNAVAILABLE");
    }
    let (x,y)=native_add_to_track(editor,target)?;
    trace(&format!("ADD_TO_TRACK_SENT target=editor client_x={x} client_y={y} result_not_verified"));
    Ok(ImportReport{box_:target,visual_change:score,elapsed_ms:started.elapsed().as_millis()})
}
