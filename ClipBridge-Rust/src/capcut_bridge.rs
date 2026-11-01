//! Experimental CapCut / Jianying Media importer and Add-to-Track bridge.
//! Inspired by the public MIT Jiandan implementation:
//! https://github.com/SKYCHENV/Jiandan (source inspected Nov 2026).
//! Uses no physical cursor motion, no project file edits, no clipboard writes.
//! Still uses an invisible Windows file chooser and synthetic window messages.
//! This is NOT a public CapCut API and requires real-version testing.
use crate::capcut_vision::{capture_media, find_imported_media, MediaBox};
use std::{
    ffi::OsStr,
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

unsafe fn owned_dialog(hwnd:HWND,pid:u32)->bool {
    if hwnd.is_null() || pid==0 {return false;}
    let mut actual=0u32;
    GetWindowThreadProcessId(hwnd,&mut actual);
    if actual!=pid {return false;}
    let mut class=[0u16;64];
    let n=GetClassNameW(hwnd,class.as_mut_ptr(),class.len() as i32);
    n>0 && String::from_utf16_lossy(&class[..n as usize])=="#32770"
}
unsafe fn cloak_dialog(dialog: HWND, editor: HWND) {
    if !IsWindow(dialog).eq(&0) {return;}
    let ex=GetWindowLongPtrW(dialog,GWL_EXSTYLE);
    SetWindowLongPtrW(dialog,GWL_EXSTYLE,
        ex|(WS_EX_LAYERED|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW) as isize);
    SetLayeredWindowAttributes(dialog,0,0,LWA_ALPHA);
    SetWindowPos(dialog,HWND_BOTTOM,-32000,-32000,0,0,
        SWP_NOACTIVATE|SWP_NOSIZE|SWP_NOOWNERZORDER);
    // A modal file picker disables the owning editor. Restore the editor
    // immediately so it does not remain visually dimmed throughout import.
    EnableWindow(editor,1);
    if GetForegroundWindow()==dialog {SetForegroundWindow(editor);}
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
        unsafe {if !self.hook.is_null(){UnhookWinEvent(self.hook);}}
    }
}
unsafe extern "system" fn enum_dialog(hwnd:HWND,param:LPARAM)->BOOL {
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
unsafe extern "system" fn enum_controls(hwnd:HWND,param:LPARAM)->BOOL {
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
    EnableWindow(editor,1);
}
unsafe fn open_image(
    editor:HWND,pid:u32,path:&PathBuf,
    trace:&mut impl FnMut(&str)
)->Result<(),String>{
    let mut msg:MSG=zeroed();
    PeekMessageW(&mut msg,null_mut(),0,0,PM_NOREMOVE);
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
unsafe fn native_add_to_track(editor:HWND,b:MediaBox)->Result<(),String>{
    let rect=bounds(editor);
    let screen_point=POINT {
        x:rect.left+b.right-12,
        y:rect.top+b.bottom-12,
    };
    let hit=WindowFromPoint(screen_point);
    let target=if !hit.is_null() && GetAncestor(hit,GA_ROOT)==editor {
        hit
    } else {editor};
    let mut client=screen_point;
    if ScreenToClient(target,&mut client)==0 {return Err("ScreenToClient failed".into());}
    let pos=packed_point(client);
    // WM_MOUSEMOVE, not SetCursorPos: do not change the physical cursor.
    PostMessageW(target,WM_MOUSEMOVE,0,pos);
    thread::sleep(Duration::from_millis(28));
    if PostMessageW(target,WM_LBUTTONDOWN,MK_LBUTTON as usize,pos)==0
        || PostMessageW(target,WM_LBUTTONUP,0,pos)==0 {
        return Err("Add-to-Track native window messages rejected".into());
    }
    Ok(())
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
    open_image(editor,job.pid,&job.image,&mut trace)?;
    trace("MEDIA_IMPORT_REQUEST_FINISHED");
    let mut found=None;
    let deadline=Instant::now()+Duration::from_millis(2000);
    while Instant::now()<deadline {
        if !window_is_editor(editor,job.pid) {break;}
        let after=capture_media(editor)?;
        if let Some((b,score))=find_imported_media(&before,&after) {
            found=Some((b,score));break;
        }
        thread::sleep(Duration::from_millis(80));
    }
    let (box_,score)=found.ok_or("Imported thumbnail could not be identified safely")?;
    trace(&format!("MEDIA_TILE_FOUND x={} y={} diff={score:.1}",box_.right,box_.bottom));
    if !editor_is_foreground(editor) {
        return Err("Focus changed before Add-to-Track; no click sent".into());
    }
    native_add_to_track(editor,box_)?;
    trace("ADD_TO_TRACK_COMMAND_SENT_NOT_VERIFIED");
    Ok(ImportReport{box_,visual_change:score,elapsed_ms:started.elapsed().as_millis()})
}
