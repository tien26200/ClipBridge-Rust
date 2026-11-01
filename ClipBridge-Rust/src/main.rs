#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use clipbridge::{encode, prepare_folder, save_setting, store_png_named, Capture, MAX_BYTES};
mod timeline_bridge;
mod capcut_bridge;
mod capcut_vision;
mod project_name;
use timeline_bridge::{TimelineBridge, UnsupportedCapCutAdapter};
use capcut_bridge::ImportJob;
use std::{
    ffi::{OsStr, OsString},
    fs,
    collections::HashMap,
    fs::OpenOptions,
    io::Write,
    time::{Instant, SystemTime, UNIX_EPOCH},
    mem::{size_of, zeroed},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::{mpsc::{self, SyncSender}, atomic::{AtomicBool, AtomicU32, AtomicIsize, Ordering}},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{Com::*, DataExchange::*, LibraryLoader::*, Memory::*, Threading::*, SystemInformation::*},
    UI::{Controls::MARGINS, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const WM_SAVED: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
const FOLDER: isize = 101;
const BROWSE: isize = 102;
const APPLY: isize = 103;
const OPEN: isize = 104;
const COPY: isize = 105;
const PAUSE: isize = 106;
const EXIT: isize = 107;
const STATUS: isize = 108;
const SHOW: isize = 109;
const LOG: isize = 110;
const AUTO_ADD: isize = 112;
const BLOCK_NATIVE: isize = 113;
const CLEAR_AFTER: isize = 114;
const PROJECT_NAME_SAVE: isize = 115;
const WM_IMPORT_FINISHED: u32 = WM_APP + 4;
const LOG_LIMIT: u64 = 2 * 1024 * 1024;
struct Job {
    capture: Capture,
    folder: PathBuf,
    sequence: u32,
    project_override: String,
}
struct Saved {
    result: Result<PathBuf, String>,
    sequence: u32,
}
struct ImportComplete {
    result: Result<capcut_bridge::ImportReport, String>,
    sequence: u32,
}
struct App {
    folder: PathBuf,
    config: PathBuf,
    preferences: PathBuf,
    project_file: PathBuf,
    manual_project: String,
    project_edit: HWND,
    block_native: bool,
    clear_after_import: bool,
    latest: Option<PathBuf>,
    latest_sequence: Option<u32>,
    tx: SyncSender<Job>,
    paused: bool,
    outstanding: u8,
    retries: u8,
    pending_seq: u32,
    last_seq: u32,
    clipboard_registered: bool,
    main: HWND,
    edit: HWND,
    status: HWND,
    pause: HWND,
    tray: NOTIFYICONDATAW,
    icon_small: HICON,
    taskbar_created: u32,
    worker: Option<std::thread::JoinHandle<()>>,
    log_tx: SyncSender<String>,
    log_path: PathBuf,
    timeline: TimelineBridge<UnsupportedCapCutAdapter>,
    keyboard_hook: HHOOK,
    import_tx: SyncSender<ImportJob>,
    import_worker: Option<std::thread::JoinHandle<()>>,
    import_busy: bool,
    import_enabled: bool,
    block_checkbox: HWND,
    clear_checkbox: HWND,
    pending_import: Option<(u32, usize, u32)>,
    last_import_attempt: Option<Instant>,
}

// Async diagnostic logging: bounded queue, small single writer, no clipboard payload.
// %LOCALAPPDATA%/ClipBridge/clipbridge.log; previous rotated at 2 MiB.
fn logger_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("ClipBridge")
        .join("clipbridge.log")
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
struct PastePreferences {
    auto_add: bool,
    block_native: bool,
    clear_after_import: bool,
}
impl Default for PastePreferences {
    fn default()->Self {
        Self {auto_add:true,block_native:true,clear_after_import:false}
    }
}
fn prefs_path()->PathBuf {
    logger_path().with_file_name("preferences.ini")
}
fn parse_preferences(text:&str)->PastePreferences {
    let mut p=PastePreferences::default();
    for line in text.lines(){
        let Some((key,value))=line.split_once('=') else {continue;};
        let val=match value.trim(){"true"|"1"=>true,"false"|"0"=>false,_=>continue};
        match key.trim(){
            "auto_add"=>p.auto_add=val,
            "block_native"=>p.block_native=val,
            "clear_after_import"=>p.clear_after_import=val,
            _=>{}
        }
    }
    p
}
fn load_preferences(path:&Path)->PastePreferences {
    fs::read_to_string(path).ok().map(|s|parse_preferences(&s))
        .unwrap_or_default()
}
fn save_preferences(a:&App)->Result<(),String>{
    if let Some(dir)=a.preferences.parent(){
        fs::create_dir_all(dir).map_err(|e|e.to_string())?;
    }
    fs::write(&a.preferences,format!(
        "auto_add={}\nblock_native={}\nclear_after_import={}\n",
        a.import_enabled,a.block_native,a.clear_after_import
    )).map_err(|e|e.to_string())
}
#[cfg(test)]
mod preferences_tests {
    use super::{PastePreferences,parse_preferences,is_image_paste_eligible};
    #[test] fn defaults_protect_from_stale_capcut_copy() {
        assert_eq!(parse_preferences(""),PastePreferences {
            auto_add:true,block_native:true,clear_after_import:false
        });
    }
    #[test] fn settings_can_be_independently_ticked() {
        assert_eq!(parse_preferences("block_native=false\nclear_after_import=true\n"),
            PastePreferences{auto_add:true,block_native:false,clear_after_import:true});
        assert_eq!(parse_preferences("auto_add=false\nblock_native=true\nclear_after_import=true\n"),
            PastePreferences{auto_add:false,block_native:true,clear_after_import:true});
        assert!(is_image_paste_eligible(true,42,42));
    }
    #[test] fn unrecognized_values_do_not_disable_protection(){
        assert!(parse_preferences("block_native=hello").block_native);
        assert!(!parse_preferences("clear_after_import=maybe").clear_after_import);
    }
}
fn start_logger(path: PathBuf) -> (SyncSender<String>, std::thread::JoinHandle<()>) {
    let (tx, rx) = mpsc::sync_channel::<String>(256);
    let thread = std::thread::spawn(move || {
        let Some(dir) = path.parent() else { return; };
        if fs::create_dir_all(dir).is_err() { return; }
        let old = path.with_extension("old.log");
        let mut count = 0u32;
        let mut writer = None;
        for line in rx {
            if writer.is_none() {
                if fs::metadata(&path).is_ok_and(|m| m.len() >= LOG_LIMIT) {
                    let _ = fs::remove_file(&old);
                    let _ = fs::rename(&path, &old);
                }
                writer = OpenOptions::new().create(true).append(true).open(&path).ok();
            }
            if let Some(file) = &mut writer {
                let _ = writeln!(file, "{line}");
            }
            count += 1;
            if count >= 64 {
                count = 0;
                if let Some(file) = &mut writer { let _ = file.flush(); }
                if fs::metadata(&path).is_ok_and(|m| m.len() >= LOG_LIMIT) {
                    writer = None;
                }
            }
        }
    });
    (tx, thread)
}
fn log_event(tx: &SyncSender<String>, description: impl AsRef<str>) {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)
        .unwrap_or_default().as_millis();
    let _ = tx.try_send(format!("{millis} | {}", description.as_ref()));
}
unsafe fn clipboard_metadata() -> String {
    let owner = GetClipboardOwner();
    let mut pid = 0u32;
    if !owner.is_null() {
        GetWindowThreadProcessId(owner, &mut pid);
    }
    // Enumerate format IDs only: never capture text, image data or window titles in logs.
    let mut formats = Vec::with_capacity(12);
    if OpenClipboard(null_mut()) != 0 {
        let mut id = 0u32;
        for _ in 0..24 {
            id = EnumClipboardFormats(id);
            if id == 0 { break; }
            formats.push(id);
        }
        CloseClipboard();
    }
    format!("owner_pid={pid} formats={formats:?}")
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
unsafe fn text(hwnd: HWND, s: &str) {
    SetWindowTextW(hwnd, wide(s).as_ptr());
}
unsafe fn message(hwnd: HWND, s: &str) {
    MessageBoxW(
        hwnd,
        wide(s).as_ptr(),
        wide("ClipBridge Rust").as_ptr(),
        MB_OK | MB_ICONINFORMATION,
    );
}
unsafe fn control(
    parent: HWND,
    class: &str,
    label: &str,
    id: isize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    style: u32,
) -> HWND {
    let handle = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(label).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        parent,
        id as HMENU,
        GetModuleHandleW(null()),
        null(),
    );
    SendMessageW(
        handle,
        WM_SETFONT,
        GetStockObject(DEFAULT_GUI_FONT) as usize,
        1,
    );
    handle
}
unsafe fn get_text(hwnd: HWND) -> String {
    let len = GetWindowTextLengthW(hwnd);
    let mut b = vec![0u16; len as usize + 1];
    GetWindowTextW(hwnd, b.as_mut_ptr(), b.len() as i32);
    String::from_utf16_lossy(&b[..len as usize])
}
unsafe fn open_path(hwnd: HWND, path: &Path) {
    if ShellExecuteW(
        hwnd,
        wide("open").as_ptr(),
        wide(path).as_ptr(),
        null(),
        null(),
        SW_SHOWNORMAL,
    ) as isize
        <= 32
    {
        message(hwnd, "Windows không mở được đường dẫn này.");
    }
}

// Copy only image formats. Text, browser history and private CapCut formats are never read.
unsafe fn read_capture(app: &App) -> Result<Option<Capture>, String> {
    let hwnd = app.main;
    let png = RegisterClipboardFormatW(wide("PNG").as_ptr());
    let kind = if IsClipboardFormatAvailable(png) != 0 {
        png
    } else if IsClipboardFormatAvailable(17) != 0 {
        17
    } else if IsClipboardFormatAvailable(8) != 0 {
        8
    } else {
        return Ok(None);
    };
    if OpenClipboard(hwnd) == 0 {
        return Err("Clipboard đang bận; đang thử lại…".into());
    }
    let result = (|| {
        let h = GetClipboardData(kind);
        if h.is_null() {
            return Err("Không đọc được dữ liệu ảnh clipboard".into());
        }
        let len = GlobalSize(h);
        if len == 0 || len > MAX_BYTES {
            return Err("Ảnh clipboard rỗng hoặc lớn hơn 128 MB".into());
        }
        let p = GlobalLock(h);
        if p.is_null() {
            return Err("Không khóa được dữ liệu clipboard".into());
        }
        let bytes = std::slice::from_raw_parts(p as *const u8, len).to_vec();
        log_event(&app.log_tx, format!("CAPTURE_READ kind={} bytes={len}", if kind==png {"PNG"} else if kind==17 {"DIBV5"} else {"DIB"}));
        GlobalUnlock(h);
        Ok(Some(if kind == png {
            Capture::Png(bytes)
        } else {
            Capture::Dib(bytes)
        }))
    })();
    CloseClipboard();
    result
}

// Date is read from the local Windows calendar, not UTC, so a copy
// just after midnight is named for the correct day on this computer.
fn local_date_stamp()->String {
    let mut value:SYSTEMTIME=unsafe{zeroed()};
    unsafe{GetLocalTime(&mut value);}
    format!("{:02}-{:02}-{:04}",value.wDay,value.wMonth,value.wYear)
}

unsafe fn capture(app: &mut App) {
    if app.paused {
        return;
    }
    let seq = GetClipboardSequenceNumber();
    if seq == app.last_seq {
        return;
    }
    if app.outstanding >= 2 {
        app.pending_seq = seq;
        log_event(&app.log_tx, format!("CAPTURE_BACKPRESSURE seq={seq} outstanding={}", app.outstanding));
        SetTimer(app.main, 1, 200, None);
        return;
    }
    let began = Instant::now();
    match read_capture(app) {
        Ok(Some(c)) => {
            match app.tx.try_send(Job {
                capture: c,
                folder: app.folder.clone(),
                sequence: seq,
                project_override: app.manual_project.clone(),
            }) {
                Ok(()) => {
                    log_event(&app.log_tx, format!("CAPTURE_ENQUEUED seq={seq} outstanding={} elapsed_ms={}", app.outstanding+1, began.elapsed().as_millis()));
                    app.outstanding += 1;
                    app.last_seq = seq;
                    // Image metadata can be routed even if PNG encoding has
                    // not completed; WM_PASTE_OBSERVED waits for WM_SAVED.
                    PASTE_IMAGE_SEQ.store(seq,Ordering::Relaxed);
                    text(app.status, "Đang lưu ảnh…");
                    app.retries = 0;
                    KillTimer(app.main, 1);
                }
                Err(mpsc::TrySendError::Full(_)) => {
                    text(app.status, "Đang xử lý ảnh trước; ảnh mới sẽ được thử lại.");
                    app.pending_seq = seq;
                    log_event(&app.log_tx, format!("CAPTURE_QUEUE_FULL seq={seq}"));
                    SetTimer(app.main, 1, 200, None);
                }
                Err(_) => {
                    text(
                        app.status,
                        "Luồng lưu ảnh đã dừng. Hãy khởi động lại ứng dụng.",
                    );
                    KillTimer(app.main, 1);
                }
            }
        }
        Ok(None) => {
            log_event(&app.log_tx, format!("CLIPBOARD_IGNORED seq={seq} reason=no_supported_image"));
            app.last_seq = seq;
            app.retries = 0;
            KillTimer(app.main, 1);
        }
        Err(e) => {
            if app.retries == 0 { log_event(&app.log_tx, format!("CAPTURE_RETRY seq={seq} reason={e}")); }
            text(app.status, &e);
            if app.retries < 10 {
                app.retries += 1;
                app.pending_seq = seq;
                SetTimer(app.main, 1, 100, None);
            } else {
                KillTimer(app.main, 1);
                text(
                    app.status,
                    "Chưa đọc được ảnh. Hãy copy lại khi ứng dụng nguồn phản hồi.",
                );
            }
        }
    }
}

// Intercept only paste of the current external image in CapCut.
// Previously the passive hook allowed CapCut to paste an OLD internal Media
// object, then our importer added the NEW image, creating two timeline clips.
// Text, unrelated applications and internal CapCut copy remain pass-through.
static PASTE_TARGET: AtomicIsize = AtomicIsize::new(0);
static PASTE_IMAGE_SEQ: AtomicU32 = AtomicU32::new(0);
static PASTE_AUTOMATION: AtomicBool = AtomicBool::new(false);
static PASTE_BLOCK_NATIVE: AtomicBool = AtomicBool::new(true);
static PASTE_V_ROUTED: AtomicBool = AtomicBool::new(false);
static PASTE_V_SUPPRESSED: AtomicBool = AtomicBool::new(false);
const WM_PASTE_OBSERVED: u32 = WM_APP + 3;

#[inline]
fn is_image_paste_eligible(enabled:bool,ready_seq:u32,current_seq:u32)->bool {
    enabled && ready_seq!=0 && ready_seq==current_seq
}

#[cfg(test)]
mod paste_route_tests {
    use super::is_image_paste_eligible;
    #[test] fn intercepts_only_current_external_image() {
        assert!(is_image_paste_eligible(true,2311,2311));
        assert!(!is_image_paste_eligible(true,2311,2320)); // clipboard changed
        assert!(!is_image_paste_eligible(true,0,2311));    // CapCut internal copy
        assert!(!is_image_paste_eligible(false,2311,2311)); // checkbox off
        assert!(!is_image_paste_eligible(true,0,0));      // no clipboard
    }
}


unsafe extern "system" fn paste_observer(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {return CallNextHookEx(null_mut(),code,w,l);}
    let k = &*(l as *const KBDLLHOOKSTRUCT);
    if (k.flags & LLKHF_INJECTED)!=0 {
        return CallNextHookEx(null_mut(),code,w,l);
    }
    // Preserve CapCut's own Ctrl+C / Ctrl+V, even if its internal copy does
    // not change the Windows clipboard. A new external copy will re-arm later.
    if k.vkCode==b'C' as u32 &&
        (w as u32==WM_KEYDOWN || w as u32==WM_SYSKEYDOWN) &&
        (GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000)!=0 &&
        focused_capcut().is_some() {
        PASTE_IMAGE_SEQ.store(0,Ordering::Relaxed);
    }
    if k.vkCode != b'V' as u32 {
        return CallNextHookEx(null_mut(),code,w,l);
    }
    let key_down=w as u32==WM_KEYDOWN || w as u32==WM_SYSKEYDOWN;
    let key_up=w as u32==WM_KEYUP || w as u32==WM_SYSKEYUP;
    if key_up {
        PASTE_V_ROUTED.store(false,Ordering::Relaxed);
        if PASTE_V_SUPPRESSED.swap(false,Ordering::Relaxed) {return 1;}
        return CallNextHookEx(null_mut(),code,w,l);
    }
    if key_down && PASTE_V_ROUTED.load(Ordering::Relaxed){
        // One ClipBridge request per physical V press. If native blocking is
        // selected, suppress repeats too. Otherwise allow regular key flow.
        if PASTE_V_SUPPRESSED.load(Ordering::Relaxed){return 1;}
        return CallNextHookEx(null_mut(),code,w,l);
    }
    if key_down &&
        (GetAsyncKeyState(VK_CONTROL as i32) as u16 & 0x8000)!=0 {
        let seq=GetClipboardSequenceNumber();
        if is_image_paste_eligible(
            PASTE_AUTOMATION.load(Ordering::Relaxed),
            PASTE_IMAGE_SEQ.load(Ordering::Relaxed),seq) {
            if focused_capcut().is_some() {
                let target=PASTE_TARGET.load(Ordering::Relaxed) as HWND;
                // Always route the new image to ClipBridge. Whether CapCut's
                // own Ctrl+V also runs is controlled by a separate checkbox.
                if !target.is_null() &&
                    PostMessageW(target,WM_PASTE_OBSERVED,seq as usize,0)!=0 {
                    PASTE_V_ROUTED.store(true,Ordering::Relaxed);
                    if PASTE_BLOCK_NATIVE.load(Ordering::Relaxed){
                        PASTE_V_SUPPRESSED.store(true,Ordering::Relaxed);
                        return 1;
                    }
                }
            }
        }
    }
    CallNextHookEx(null_mut(),code,w,l)
}
unsafe fn focused_capcut() -> Option<(usize,u32)> {
    let hwnd=GetForegroundWindow();
    if hwnd.is_null() {return None;}
    let mut pid=0u32;
    GetWindowThreadProcessId(hwnd,&mut pid);
    if pid==0{return None;}
    let handle=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,0,pid);
    if handle.is_null(){return None;}
    let mut buf=[0u16;1024];
    let mut size=buf.len() as u32;
    let ok=QueryFullProcessImageNameW(handle,0,buf.as_mut_ptr(),&mut size)!=0;
    CloseHandle(handle);
    if !ok{return None;}
    let process=String::from_utf16_lossy(&buf[..size as usize]).to_ascii_lowercase();
    if process.ends_with("\\capcut.exe") || process.ends_with("\\jianyingpro.exe") {
        Some((hwnd as usize,pid))
    } else {None}
}
unsafe fn enqueue_import(a:&mut App,editor:usize,pid:u32,sequence:u32,path:PathBuf){
    if a.paused || !a.import_enabled{return;}
    if a.import_busy {
        log_event(&a.log_tx,"IMPORT_SKIPPED reason=busy");
        return;
    }
    if a.last_import_attempt.is_some_and(|t|t.elapsed().as_millis()<400) {
        log_event(&a.log_tx,"IMPORT_SKIPPED reason=cooldown");
        return;
    }
    if GetClipboardSequenceNumber()!=sequence || !path.is_file() {
        log_event(&a.log_tx,"IMPORT_SKIPPED reason=clipboard_changed_or_file_missing");
        return;
    }
    match a.import_tx.try_send(ImportJob {editor,pid,image:path,sequence}) {
        Ok(()) => {
            a.import_busy=true;
            a.last_import_attempt=Some(Instant::now());
            log_event(&a.log_tx,format!("IMPORT_QUEUED seq={sequence} pid={pid}"));
            text(a.status,"Đang đưa ảnh vào CapCut Media rồi thêm vào timeline…");
        }
        Err(mpsc::TrySendError::Full(_))=>{
            log_event(&a.log_tx,"IMPORT_QUEUE_FULL");
        }
        Err(_)=>{
            log_event(&a.log_tx,"IMPORT_WORKER_UNAVAILABLE");
        }
    }
}

// Deliberately optional: clearing Windows clipboard is cleanup AFTER an
// import command, not a substitute for suppressing CapCut's native paste.
// Do not remove the PNG file: CapCut may reference it in this project.
unsafe fn clear_clipboard_if_unchanged(a:&mut App,seq:u32)->Result<bool,String>{
    if GetClipboardSequenceNumber()!=seq {return Ok(false);}
    if OpenClipboard(a.main)==0 {
        return Err("Cannot open clipboard for optional cleanup".into());
    }
    let result=if GetClipboardSequenceNumber()!=seq {
        Ok(false)
    } else if EmptyClipboard()==0 {
        Err("Windows refused EmptyClipboard".to_string())
    } else {
        a.last_seq=GetClipboardSequenceNumber();
        a.latest_sequence=None;
        a.pending_import=None;
        a.latest=None; // clears only in-memory latest; PNG files remain.
        PASTE_IMAGE_SEQ.store(0,Ordering::Relaxed);
        Ok(true)
    };
    CloseClipboard();
    result
}
unsafe fn choose_folder(hwnd: HWND) -> Option<PathBuf> {
    let mut display = [0u16; 260];
    let title = wide("Chọn thư mục lưu ảnh mới · Ảnh cũ giữ nguyên vị trí");
    let mut info: BROWSEINFOW = zeroed();
    info.hwndOwner = hwnd;
    info.pszDisplayName = display.as_mut_ptr();
    info.lpszTitle = title.as_ptr();
    info.ulFlags = 0x41;
    let pidl = SHBrowseForFolderW(&info);
    if pidl.is_null() {
        return None;
    }
    let mut path = vec![0u16; 32768];
    let mut result = None;
    if SHGetPathFromIDListEx(pidl, path.as_mut_ptr(), path.len() as u32, 0) != 0 {
        let len = path.iter().position(|&x| x == 0).unwrap_or(0);
        result = Some(PathBuf::from(OsString::from_wide(&path[..len])));
    }
    CoTaskMemFree(pidl as *const _);
    result
}
unsafe fn apply_folder(app: &mut App) -> Result<(), String> {
    let proposed = PathBuf::from(get_text(app.edit).trim());
    match prepare_folder(&proposed).and_then(|p| {
        save_setting(&app.config, &p)?;
        Ok(p)
    }) {
        Ok(folder) => {
            app.folder = folder;
            text(
                app.status,
                "Đã đổi nơi lưu cho ảnh mới. Ảnh cũ không di chuyển.",
            );
            Ok(())
        }
        Err(e) => Err(format!(
            "Chưa đổi thư mục. {e}\nCấu hình trước vẫn được giữ."
        )),
    }
}
unsafe fn write_path(hwnd: HWND, path: &Path) -> Result<(), String> {
    let data = wide(path);
    let h = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2);
    if h.is_null() {
        return Err("Không đủ bộ nhớ clipboard.".into());
    }
    let p = GlobalLock(h);
    if p.is_null() {
        GlobalFree(h);
        return Err("Không khóa được bộ nhớ clipboard.".into());
    }
    std::ptr::copy_nonoverlapping(data.as_ptr(), p as *mut u16, data.len());
    GlobalUnlock(h);
    if OpenClipboard(hwnd) == 0 {
        GlobalFree(h);
        return Err("Clipboard đang bận.".into());
    }
    if EmptyClipboard() == 0 {
        CloseClipboard();
        GlobalFree(h);
        return Err("Không mở được clipboard để ghi.".into());
    }
    if SetClipboardData(13, h).is_null() {
        CloseClipboard();
        GlobalFree(h);
        return Err("Không copy được đường dẫn.".into());
    }
    CloseClipboard();
    Ok(())
}
unsafe fn copy_path(app: &mut App) {
    let Some(path) = &app.latest else {
        text(app.status, "Chưa lưu ảnh nào trong phiên này.");
        return;
    };
    match write_path(app.main, path) {
        Ok(()) => {
            app.last_seq = GetClipboardSequenceNumber();
            PASTE_IMAGE_SEQ.store(0,Ordering::Relaxed);
            log_event(&app.log_tx, "MANUAL_COPY_PATH success clipboard_replaced=true");
            text(app.status, "Đã copy đường dẫn dạng text (clipboard ảnh gốc đã được thay).");
        }
        Err(e) => { log_event(&app.log_tx, format!("MANUAL_COPY_PATH error={e}")); text(app.status, &e); },
    }
}
fn newer_or_same_sequence(candidate: u32, previous: u32) -> bool {
    candidate.wrapping_sub(previous) < 0x8000_0000
}

unsafe fn pause(app: &mut App) {
    app.paused = !app.paused;
    PASTE_AUTOMATION.store(!app.paused && app.import_enabled,Ordering::Relaxed);
    log_event(&app.log_tx, format!("PAUSE changed={}", app.paused));
    text(
        app.pause,
        if app.paused {
            "Tiếp tục nhận ảnh"
        } else {
            "Tạm dừng"
        },
    );
    text(
        app.status,
        if app.paused {
            "Đã tạm dừng nhận ảnh."
        } else {
            "Sẵn sàng — tự lưu PNG; không ghi đè clipboard."
        },
    );
    if app.paused {
        KillTimer(app.main, 1);
    } else {
        capture(app);
    }
}
unsafe fn show(app: &App) {
    ShowWindow(app.main, SW_RESTORE);
    SetForegroundWindow(app.main);
}
unsafe fn install_tray(app: &mut App) {
    app.tray.uID = 1;
    if Shell_NotifyIconW(NIM_ADD, &app.tray) == 0 {
        app.tray.uID = 0;
        // Keep settings reachable if Explorer/the notification area is unavailable.
        show(app);
    }
}

unsafe fn window_shadow(hwnd: HWND) {
    let policy = DWMNCRP_ENABLED;
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_NCRENDERING_POLICY as u32,
        &policy as *const _ as *const _,
        size_of::<i32>() as u32,
    );
    let margins = MARGINS {
        cxLeftWidth: 1,
        cxRightWidth: 1,
        cyTopHeight: 1,
        cyBottomHeight: 1,
    };
    DwmExtendFrameIntoClientArea(hwnd, &margins);
    // Rounded corners on Windows 11; unsupported Windows 10 attributes are ignored.
    let round = 2u32;
    DwmSetWindowAttribute(
        hwnd,
        33,
        &round as *const _ as *const _,
        size_of::<u32>() as u32,
    );
}
unsafe fn menu(hwnd: HWND, paused: bool) -> isize {
    let menu = CreatePopupMenu();
    AppendMenuW(
        menu,
        MF_STRING,
        SHOW as usize,
        wide("Mở ClipBridge").as_ptr(),
    );
    AppendMenuW(
        menu,
        MF_STRING,
        PAUSE as usize,
        wide(if paused {
            "Tiếp tục"
        } else {
            "Tạm dừng"
        })
        .as_ptr(),
    );
    AppendMenuW(menu, MF_STRING, EXIT as usize, wide("Thoát").as_ptr());
    let mut point: POINT = zeroed();
    GetCursorPos(&mut point);
    SetForegroundWindow(hwnd);
    let id = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        point.x,
        point.y,
        0,
        hwnd,
        null(),
    );
    DestroyMenu(menu);
    id as isize
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(l as *const CREATESTRUCTW);
        let app = create.lpCreateParams as *mut App;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
        (*app).main = hwnd;
    }
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if p.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    if msg != 0 && msg == (*p).taskbar_created {
        install_tray(&mut *p);
        return 0;
    }
    match msg {
        WM_CREATE => {
            let a = &mut *p;
            control(
                hwnd,
                "STATIC",
                "ClipBridge Rust · Auto-save nhẹ và giữ clipboard",
                0,
                20,
                18,
                650,
                26,
                0,
            );
            control(
                hwnd,
                "STATIC",
                "Thư mục lưu ảnh mới:",
                0,
                20,
                55,
                600,
                22,
                0,
            );
            a.edit = control(
                hwnd,
                "EDIT",
                &a.folder.to_string_lossy(),
                FOLDER,
                20,
                80,
                550,
                28,
                WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32,
            );
            control(hwnd, "BUTTON", "Chọn…", BROWSE, 580, 80, 90, 28, WS_TABSTOP);
            control(
                hwnd,
                "BUTTON",
                "Áp dụng",
                APPLY,
                20,
                120,
                100,
                30,
                WS_TABSTOP,
            );
            control(
                hwnd,
                "BUTTON",
                "Mở thư mục",
                OPEN,
                130,
                120,
                120,
                30,
                WS_TABSTOP,
            );
            control(
                hwnd,
                "STATIC",
                "Đổi thư mục không di chuyển/xóa ảnh cũ. Giữ ảnh cũ để CapCut không mất liên kết.",
                0,
                20,
                165,
                650,
                44,
                0,
            );
            a.status = control(
                hwnd,
                "STATIC",
                "Sẵn sàng — tự lưu PNG và giữ nguyên clipboard ảnh.",
                STATUS,
                20,
                217,
                650,
                54,
                0,
            );
            control(
                hwnd,
                "BUTTON",
                "Copy đường dẫn ảnh cuối",
                COPY,
                20,
                280,
                195,
                32,
                WS_TABSTOP,
            );
            a.pause = control(
                hwnd,
                "BUTTON",
                "Tạm dừng",
                PAUSE,
                230,
                280,
                155,
                32,
                WS_TABSTOP,
            );
            control(hwnd, "BUTTON", "Mở log", LOG, 403, 280, 150, 32, WS_TABSTOP);
            control(hwnd,"STATIC","TÙY CHỌN PASTE TRONG CAPCUT",0,20,317,640,21,0);
            let auto=control(hwnd,"BUTTON","Tự thêm ảnh vào CapCut khi Ctrl+V",
                AUTO_ADD,20,342,630,27,BS_AUTOCHECKBOX as u32|WS_TABSTOP);
            SendMessageW(auto,BM_SETCHECK,if a.import_enabled{1}else{0},0);
            a.block_checkbox=control(hwnd,"BUTTON",
                "Chặn Ctrl+V gốc khi thêm ảnh mới (khuyên dùng; tránh dán ảnh cũ)",
                BLOCK_NATIVE,38,375,630,28,BS_AUTOCHECKBOX as u32|WS_TABSTOP);
            SendMessageW(a.block_checkbox,BM_SETCHECK,if a.block_native{1}else{0},0);
            a.clear_checkbox=control(hwnd,"BUTTON",
                "Clear clipboard Windows sau khi gửi lệnh thêm ảnh (tùy chọn)",
                CLEAR_AFTER,38,408,630,28,BS_AUTOCHECKBOX as u32|WS_TABSTOP);
            SendMessageW(a.clear_checkbox,BM_SETCHECK,if a.clear_after_import{1}else{0},0);
            control(hwnd, "BUTTON", "Thoát", EXIT, 575, 280, 95, 32, WS_TABSTOP);
            control(
                hwnd,
                "STATIC",
                "Clear chỉ xóa clipboard Windows sau khi gửi nút +; không xóa Media/PNG cũ.",
                0,
                38,
                442,
                640,
                26,
                0,
            );
            control(hwnd,"STATIC",
                "Tên project cho file PNG (để trống = tự nhận diện CapCut)",
                0,20,479,650,22,0);
            a.project_edit=control(hwnd,"EDIT",&a.manual_project,0,
                20,506,505,29,WS_BORDER|WS_TABSTOP|ES_AUTOHSCROLL as u32);
            control(hwnd,"BUTTON","Lưu tên project",
                PROJECT_NAME_SAVE,535,506,135,29,WS_TABSTOP);
            a.clipboard_registered = AddClipboardFormatListener(hwnd) != 0;
            log_event(&a.log_tx, format!("LISTENER registered={}", a.clipboard_registered));
            if !a.clipboard_registered {
                text(a.status,"Không đăng ký được clipboard listener. Hãy mở lại ứng dụng bằng tài khoản desktop.");
            }
            a.tray.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
            a.tray.hWnd = hwnd;
            a.tray.uID = 1;
            a.tray.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            a.tray.uCallbackMessage = WM_TRAY;
            a.tray.hIcon = a.icon_small;
            let tip = wide("ClipBridge Rust · tự lưu ảnh clipboard");
            a.tray.szTip[..tip.len()].copy_from_slice(&tip);
            install_tray(a);
            0
        }
        WM_PASTE_OBSERVED => {
            let a=&mut *p;
            let seq=w as u32;
            if !a.paused && a.import_enabled && seq==GetClipboardSequenceNumber() {
                if let Some((editor,pid))=focused_capcut() {
                    log_event(&a.log_tx,format!("CAPCUT_PASTE_ROUTED seq={seq} block_native={} clear_after_import={}",
                        a.block_native,a.clear_after_import));
                    if a.latest_sequence==Some(seq) {
                        if let Some(path)=a.latest.clone(){
                            enqueue_import(a,editor,pid,seq,path);
                        }
                    } else if a.outstanding>0 && a.last_seq==seq {
                        a.pending_import=Some((seq,editor,pid));
                        log_event(&a.log_tx,format!("IMPORT_AWAITING_SAVE seq={seq}"));
                    } else {
                        log_event(&a.log_tx,"PASTE_PASSTHROUGH reason=clipboard_not_latest_image");
                    }
                }
            }
            0
        }
        WM_IMPORT_FINISHED => {
            let complete=Box::from_raw(l as *mut ImportComplete);
            let a=&mut *p;
            a.import_busy=false;
            let ImportComplete {result,sequence}=*complete;
            match result {
                Ok(info)=>{
                    log_event(&a.log_tx,format!(
                        "IMPORT_ADD_COMMAND_SENT seq={sequence} ms={} bbox={},{},{},{} visual_score={:.1} result_not_confirmed",
                        info.elapsed_ms,info.box_.left,info.box_.top,info.box_.right,info.box_.bottom,info.visual_change));
                    if a.clear_after_import {
                        match clear_clipboard_if_unchanged(a,sequence) {
                            Ok(true)=>log_event(&a.log_tx,
                                format!("CLIPBOARD_CLEARED seq={sequence} after_add_command=true")),
                            Ok(false)=>log_event(&a.log_tx,
                                format!("CLIPBOARD_CLEAR_SKIPPED seq={sequence} reason=changed")),
                            Err(e)=>log_event(&a.log_tx,format!("CLIPBOARD_CLEAR_FAILED seq={sequence} {e}")),
                        }
                    }
                    text(a.status,"Đã gửi nút + của Media tới CapCut. Kiểm tra timeline; nếu chưa chèn, gửi log.");
                }
                Err(error)=>{
                    log_event(&a.log_tx,format!("IMPORT_FAILED seq={sequence} reason={error}"));
                    text(a.status,&format!("Chưa chèn được ảnh: {error}. Xem log."));
                }
            }
            0
        }
        WM_CLIPBOARDUPDATE => {
            let a=&mut *p;
            let seq=GetClipboardSequenceNumber();
            if seq!=a.last_seq {
                log_event(&a.log_tx,format!("CLIPBOARD_UPDATE seq={seq} {}",clipboard_metadata()));
            }
            // Do not reinterpret CapCut's own Media Ctrl+C as an external
            // picture for ClipBridge to import. This keeps manual internal
            // CapCut copy/paste behavior intact.
            let owner=GetClipboardOwner();
            let mut owner_pid=0u32;
            if !owner.is_null(){GetWindowThreadProcessId(owner,&mut owner_pid);}
            if focused_capcut().is_some_and(|(_,pid)|pid==owner_pid) {
                PASTE_IMAGE_SEQ.store(0,Ordering::Relaxed);
                a.last_seq=seq;
                a.retries=0;
                a.pending_import=None;
                KillTimer(hwnd,1);
                log_event(&a.log_tx,format!("CAPCUT_INTERNAL_COPY_PASS_THROUGH seq={seq}"));
                return 0;
            }
            a.retries=0;
            capture(a);
            0
        }
        WM_TIMER if w == 1 => {
            let a = &mut *p;
            if a.pending_seq != GetClipboardSequenceNumber() {
                a.retries = 0;
            }
            capture(a);
            0
        }
        WM_SAVED => {
            let result = Box::from_raw(l as *mut Saved);
            let a = &mut *p;
            a.outstanding = a.outstanding.saturating_sub(1);
            let Saved { result, sequence } = *result;
            match result {
                Ok(path) => {
                    // Do not let an older save completion replace a newer image.
                    // Never rewrite clipboard here. Other apps may rely on
                    // original formats or delayed rendering owned by the source.
                    if a.latest_sequence.is_none_or(|prev| newer_or_same_sequence(sequence, prev)) {
                        a.latest = Some(path.clone());
                        a.latest_sequence = Some(sequence);
                        if GetClipboardSequenceNumber()==sequence {
                            PASTE_IMAGE_SEQ.store(sequence,Ordering::Relaxed);
                        }
                        log_event(&a.log_tx, format!("SAVE_OK seq={sequence} filename={} clipboard_current_seq={} unchanged_by_app=true", path.file_name().unwrap_or_default().to_string_lossy(), GetClipboardSequenceNumber()));
                        text(a.status, &format!("Đã lưu PNG: {} (giữ clipboard ảnh gốc)", path.display()));
                        a.timeline.on_image_saved(sequence, path.clone());
                        if let Some(&(wanted,editor,pid))=a.pending_import.as_ref(){
                            if wanted==sequence {
                                a.pending_import=None;
                                enqueue_import(a,editor,pid,sequence,path);
                            }
                        }
                    }
                }
                Err(e) => {
                    if PASTE_IMAGE_SEQ.load(Ordering::Relaxed)==sequence {
                        PASTE_IMAGE_SEQ.store(0,Ordering::Relaxed);
                    }
                    if a.pending_import.as_ref().is_some_and(|(seq,_,_)|*seq==sequence){
                        a.pending_import=None;
                    }
                    log_event(&a.log_tx,format!("SAVE_ERROR seq={sequence} error={e}"));
                    text(a.status,&format!("Không lưu được ảnh: {e}"));
                },
            }
            0
        }
        WM_COMMAND => {
            match (w & 0xffff) as isize {
                BROWSE => {
                    if let Some(folder) = choose_folder(hwnd) {
                        SetWindowTextW((*p).edit, wide(folder).as_ptr());
                    }
                }
                APPLY => {
                    if let Err(e) = apply_folder(&mut *p) {
                        message(hwnd, &e);
                    }
                }
                OPEN => {
                    let folder = (*p).folder.clone();
                    open_path(hwnd, &folder);
                }
                COPY => copy_path(&mut *p),
                AUTO_ADD => {
                    let a=&mut *p;
                    a.import_enabled=SendMessageW(GetDlgItem(hwnd,AUTO_ADD as i32),BM_GETCHECK,0,0)!=0;
                    PASTE_AUTOMATION.store(!a.paused && a.import_enabled,Ordering::Relaxed);
                    log_event(&a.log_tx,format!("AUTO_ADD enabled={}",a.import_enabled));
                    if !a.import_enabled {a.pending_import=None;}
                    if let Err(e)=save_preferences(a){log_event(&a.log_tx,format!("PREFS_WRITE_FAILED {e}"));}
                },
                BLOCK_NATIVE => {
                    let a=&mut *p;
                    a.block_native=SendMessageW(a.block_checkbox,BM_GETCHECK,0,0)!=0;
                    PASTE_BLOCK_NATIVE.store(a.block_native,Ordering::Relaxed);
                    log_event(&a.log_tx,format!("BLOCK_NATIVE enabled={}",a.block_native));
                    if let Err(e)=save_preferences(a){log_event(&a.log_tx,format!("PREFS_WRITE_FAILED {e}"));}
                },
                CLEAR_AFTER => {
                    let a=&mut *p;
                    a.clear_after_import=SendMessageW(a.clear_checkbox,BM_GETCHECK,0,0)!=0;
                    log_event(&a.log_tx,format!("CLEAR_AFTER_IMPORT enabled={}",a.clear_after_import));
                    if let Err(e)=save_preferences(a){log_event(&a.log_tx,format!("PREFS_WRITE_FAILED {e}"));}
                },
                PROJECT_NAME_SAVE => {
                    let a=&mut *p;
                    let value=get_text(a.project_edit).trim().to_string();
                    if value.chars().count()>90 {
                        text(a.status,"Tên project quá dài (tối đa 90 ký tự).");
                    } else {
                        let path=&a.project_file;
                        let result=path.parent().map(|d|fs::create_dir_all(d))
                            .unwrap_or(Ok(())).and_then(|_|fs::write(path,&value));
                        match result {
                            Ok(())=>{
                                a.manual_project=value;
                                text(a.status,if a.manual_project.is_empty(){
                                    "Tên PNG: tự nhận diện project CapCut khi copy ảnh."
                                }else{
                                    "Đã lưu tên project cho PNG. Các ảnh copy tiếp theo sẽ dùng tên này."
                                });
                                log_event(&a.log_tx,format!(
                                    "PROJECT_NAME_MODE mode={} (file names only)",
                                    if a.manual_project.is_empty(){"automatic"}else{"manual"}));
                            }
                            Err(e)=>text(a.status,&format!("Không lưu được tên project: {e}"))
                        }
                    }
                },
                LOG => { let path = (*p).log_path.clone(); open_path(hwnd, &path); },
                PAUSE => pause(&mut *p),
                SHOW => show(&*p),
                EXIT => {
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_TRAY => {
            match l as u32 {
                WM_LBUTTONUP => show(&*p),
                WM_RBUTTONUP => {
                    let id = menu(hwnd, (*p).paused);
                    PostMessageW(hwnd, WM_COMMAND, id as usize, 0);
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            let a = &mut *p;
            if a.tray.uID != 0 {
                ShowWindow(hwnd, SW_HIDE);
            } else {
                DestroyWindow(hwnd);
            }
            0
        }
        WM_SIZE if w == SIZE_MINIMIZED as usize => {
            if (*p).tray.uID != 0 {
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }
        WM_DESTROY => {
            let a = &mut *p;
            KillTimer(hwnd, 1);
            if a.clipboard_registered {
                RemoveClipboardFormatListener(hwnd);
            }
            PASTE_AUTOMATION.store(false,Ordering::Relaxed);
            PASTE_IMAGE_SEQ.store(0,Ordering::Relaxed);
            PASTE_TARGET.store(0,Ordering::Relaxed);
            if !a.keyboard_hook.is_null() { UnhookWindowsHookEx(a.keyboard_hook); a.keyboard_hook = null_mut(); }
            if a.tray.uID != 0 {
                Shell_NotifyIconW(NIM_DELETE, &a.tray);
            }
            log_event(&a.log_tx, "SHUTDOWN requested");
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

fn main() {
    unsafe {
        SetProcessDPIAware();
        let mutex = CreateMutexW(null(), 0, wide("Local\\ClipBridgeRust-v2").as_ptr());
        if mutex.is_null() {
            message(null_mut(), "Không tạo được khóa ứng dụng.");
            return;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            CloseHandle(mutex);
            return;
        }
        let com = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        let root = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let config = root.join("settings.txt");
        let folder = fs::read_to_string(&config)
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("Images"));
        // Never fall back to a different drive silently if the configured drive is offline.
        if let Err(e) = prepare_folder(&folder) {
            message(null_mut(),&format!("Thư mục đã chọn chưa dùng được: {e}\nBạn có thể chọn thư mục khác trong cửa sổ ứng dụng."));
        }
        let instance = GetModuleHandleW(null());
        let icon_small = LoadImageW(
            instance,
            1usize as *const u16,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_SHARED,
        ) as HICON;
        let icon_large = LoadImageW(
            instance,
            1usize as *const u16,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXICON),
            GetSystemMetrics(SM_CYICON),
            LR_SHARED,
        ) as HICON;
        let (tx, rx) = mpsc::sync_channel::<Job>(1);
        let (import_tx, import_rx) = mpsc::sync_channel::<ImportJob>(1);
        let log_path = logger_path();
        let (log_tx, log_thread) = start_logger(log_path.clone());
        let preferences=prefs_path();
        let saved_prefs=load_preferences(&preferences);
        let project_file=logger_path().with_file_name("project_override.txt");
        let manual_project=fs::read_to_string(&project_file).unwrap_or_default();
        log_event(&log_tx,format!("START version=0.7.7 auto_add={} block_native={} clear_after_import={}",
            saved_prefs.auto_add,saved_prefs.block_native,saved_prefs.clear_after_import));
        let mut app = Box::new(App {
            folder,
            config,
            preferences,
            project_file,
            manual_project,
            project_edit:null_mut(),
            block_native: saved_prefs.block_native,
            clear_after_import: saved_prefs.clear_after_import,
            latest: None,
            latest_sequence: None,
            tx,
            paused: false,
            outstanding: 0,
            retries: 0,
            pending_seq: 0,
            last_seq: 0,
            clipboard_registered: false,
            main: null_mut(),
            edit: null_mut(),
            status: null_mut(),
            pause: null_mut(),
            tray: zeroed(),
            icon_small: if icon_small.is_null() {
                LoadIconW(null_mut(), IDI_APPLICATION)
            } else {
                icon_small
            },
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            worker: None,
            log_tx: log_tx.clone(),
            log_path,
            timeline: TimelineBridge::new(UnsupportedCapCutAdapter),
            keyboard_hook: null_mut(),
            import_tx,
            import_worker: None,
            import_busy: false,
            import_enabled: saved_prefs.auto_add,
            block_checkbox: null_mut(),
            clear_checkbox: null_mut(),
            pending_import: None,
            last_import_attempt: None,
        });
        let class = wide("ClipBridgeRustWindow");
        let wc = WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: icon_large,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("ClipBridge Rust").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            710,
            590,
            null_mut(),
            null_mut(),
            instance,
            &mut *app as *mut App as *const _,
        );
        if hwnd.is_null() {
            message(
                null_mut(),
                &format!("Không tạo được cửa sổ: {}", std::io::Error::last_os_error()),
            );
            CloseHandle(mutex);
            return;
        }
        SendMessageW(
            hwnd,
            WM_SETICON,
            ICON_SMALL as usize,
            app.icon_small as isize,
        );
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon_large as isize);
        window_shadow(hwnd);
        PASTE_TARGET.store(hwnd as isize,Ordering::Relaxed);
        PASTE_AUTOMATION.store(app.import_enabled && !app.paused,Ordering::Relaxed);
        PASTE_BLOCK_NATIVE.store(app.block_native,Ordering::Relaxed);
        app.keyboard_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(paste_observer), instance, 0);
        log_event(&app.log_tx, format!("PASTE_ROUTER installed={} conditional_suppress={} optional_clear={}",
            !app.keyboard_hook.is_null(),app.block_native,app.clear_after_import));
        let target = hwnd as usize;
        let worker_log = log_tx.clone();
        let import_log=log_tx.clone();
        app.import_worker=Some(std::thread::spawn(move || {
            for job in import_rx {
                let seq=job.sequence;
                log_event(&import_log,format!("IMPORT_STARTED seq={seq}"));
                let result=capcut_bridge::import_then_add(&job, |step| {
                    log_event(&import_log,format!("IMPORT_STEP seq={seq} {step}"));
                });
                let raw=Box::into_raw(Box::new(ImportComplete {result,sequence:seq}));
                if PostMessageW(target as HWND,WM_IMPORT_FINISHED,0,raw as isize)==0{
                    drop(Box::from_raw(raw));
                }
            }
        }));
        app.worker = Some(std::thread::spawn(move || {
            // Cached counters are isolated to this writer. They are rebuilt
            // from existing filenames once per project/day after restart.
            let mut counters:HashMap<(PathBuf,String,String),u32>=HashMap::new();
            for job in rx {
                let start = Instant::now();
                // The name belongs to the PNG SAVED AT COPY TIME,
                // not to any existing filename inside CapCut Media.
                let label=project_name::detect_project(&job.project_override);
                let date=local_date_stamp();
                log_event(&worker_log,format!("NAMING seq={} project={} via={} date={}",
                    job.sequence,label.name,label.source,date));
                let result=encode(job.capture).and_then(|png|
                    store_png_named(&png,&job.folder,&label.name,&date,&mut counters));
                log_event(&worker_log, format!("WORKER_FINISHED seq={} success={} duration_ms={}", job.sequence, result.is_ok(), start.elapsed().as_millis()));
                let raw = Box::into_raw(Box::new(Saved {
                    result,
                    sequence: job.sequence,
                }));
                if PostMessageW(target as HWND, WM_SAVED, 0, raw as isize) == 0 {
                    drop(Box::from_raw(raw));
                }
            }
        }));
        // Normal startup stays hidden: tray only, no taskbar/Alt+Tab entry.
        // A failed tray registration already reveals the settings as a fallback.
        // Do not capture pre-existing clipboard contents at launch; wait for a new copy.
        app.last_seq = GetClipboardSequenceNumber();
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if IsDialogMessageW(hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let worker=app.worker.take();
        let import_worker=app.import_worker.take();
        drop(app);
        if let Some(worker)=worker {let _=worker.join();}
        if let Some(worker)=import_worker {let _=worker.join();}
        // Drain worker results that arrived during shutdown without leaking allocations.
        while PeekMessageW(&mut msg, hwnd, WM_SAVED, WM_SAVED, PM_REMOVE)!=0 {
            drop(Box::from_raw(msg.lParam as *mut Saved));
        }
        while PeekMessageW(&mut msg, hwnd, WM_IMPORT_FINISHED, WM_IMPORT_FINISHED, PM_REMOVE)!=0 {
            drop(Box::from_raw(msg.lParam as *mut ImportComplete));
        }
        if com >= 0 {
            CoUninitialize();
        }
        log_event(&log_tx, "EXIT");
        drop(log_tx);
        let _ = log_thread.join();
        CloseHandle(mutex);
    }
}
