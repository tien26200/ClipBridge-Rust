#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use clipbridge::{encode, prepare_folder, save_setting, store_png, Capture, MAX_BYTES};
use std::{
    ffi::{OsStr, OsString},
    fs,
    fs::OpenOptions,
    io::Write,
    time::{Instant, SystemTime, UNIX_EPOCH},
    mem::{size_of, zeroed},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::mpsc::{self, SyncSender},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{Com::*, DataExchange::*, LibraryLoader::*, Memory::*, Threading::*},
    UI::{Controls::MARGINS, Shell::*, WindowsAndMessaging::*},
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
const LOG_LIMIT: u64 = 2 * 1024 * 1024;
struct Job {
    capture: Capture,
    folder: PathBuf,
    sequence: u32,
}
struct Saved {
    result: Result<PathBuf, String>,
    sequence: u32,
}
struct App {
    folder: PathBuf,
    config: PathBuf,
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
            }) {
                Ok(()) => {
                    log_event(&app.log_tx, format!("CAPTURE_ENQUEUED seq={seq} outstanding={} elapsed_ms={}", app.outstanding+1, began.elapsed().as_millis()));
                    app.outstanding += 1;
                    app.last_seq = seq;
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
            control(hwnd, "BUTTON", "Thoát", EXIT, 575, 280, 95, 32, WS_TABSTOP);
            control(
                hwnd,
                "STATIC",
                "Log: %LOCALAPPDATA%\\ClipBridge\\clipbridge.log (không lưu nội dung clipboard).",
                0,
                20,
                330,
                650,
                30,
                0,
            );
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
        WM_CLIPBOARDUPDATE => {
            let a = &mut *p;
            let seq = GetClipboardSequenceNumber();
            if seq != a.last_seq { log_event(&a.log_tx, format!("CLIPBOARD_UPDATE seq={seq} {}", clipboard_metadata())); }
            a.retries = 0;
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
                        log_event(&a.log_tx, format!("SAVE_OK seq={sequence} filename={} clipboard_current_seq={} unchanged_by_app=true", path.file_name().unwrap_or_default().to_string_lossy(), GetClipboardSequenceNumber()));
                        text(a.status, &format!("Đã lưu PNG: {} (giữ clipboard ảnh gốc)", path.display()));
                    }
                }
                Err(e) => { log_event(&a.log_tx, format!("SAVE_ERROR seq={sequence} error={e}")); text(a.status, &format!("Không lưu được ảnh: {e}")); },
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
        let log_path = logger_path();
        let (log_tx, log_thread) = start_logger(log_path.clone());
        log_event(&log_tx, "START version=0.5.1 logging=metadata_only clipboard_auto_write=false");
        let mut app = Box::new(App {
            folder,
            config,
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
            410,
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
        let target = hwnd as usize;
        let worker_log = log_tx.clone();
        app.worker = Some(std::thread::spawn(move || {
            for job in rx {
                let start = Instant::now();
                let result = encode(job.capture).and_then(|png| store_png(&png, &job.folder));
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
        let worker = app.worker.take();
        drop(app);
        if let Some(worker) = worker {
            let _ = worker.join();
        }
        // Drain worker results that arrived during shutdown without leaking allocations.
        while PeekMessageW(&mut msg, hwnd, WM_SAVED, WM_SAVED, PM_REMOVE) != 0 {
            drop(Box::from_raw(msg.lParam as *mut Saved));
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
