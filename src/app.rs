use crate::{
    actions::{post_toast, run_worker},
    clipboard::ClipboardService,
    config::{self, AppConfig, TriggerButton},
    hook::{
        self, HookCommand, UiPoint, WM_APP_CAPTURE_DONE, WM_APP_OVERLAY_BEGIN, WM_APP_OVERLAY_END,
        WM_APP_OVERLAY_POINT, WM_APP_SHOW_HISTORY, WM_APP_TOAST, WM_APP_TRAY,
    },
    logging,
    storage::{ClipItem, Storage},
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::c_void,
    mem,
    path::PathBuf,
    ptr,
    sync::{
        Arc, RwLock,
        atomic::{AtomicPtr, Ordering},
        mpsc::{self, Sender},
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::{
        COLORREF, CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM,
        LRESULT, POINT, RECT, WPARAM,
    },
    Graphics::Gdi::{
        BITMAPINFO, BITMAPINFOHEADER, BeginPaint, CLEARTYPE_QUALITY, COLOR_WINDOW, CreateFontW,
        CreatePen, CreateRoundRectRgn, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
        DIB_RGB_COLORS, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER,
        DeleteObject, DrawTextW, EndPaint, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, FillRect,
        GetMonitorInfoW, HBRUSH, HFONT, InvalidateRect, LineTo, MONITOR_DEFAULTTONEAREST,
        MONITORINFO, MonitorFromPoint, MoveToEx, PAINTSTRUCT, PS_SOLID, RoundRect, SRCCOPY,
        ScreenToClient, SelectObject, SetBkColor, SetBkMode, SetTextColor, SetWindowRgn,
        StretchDIBits, TRANSPARENT,
    },
    System::{
        Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener},
        LibraryLoader::GetModuleHandleW,
        Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
            RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
        },
        Threading::CreateMutexW,
    },
    UI::{
        Controls::{
            BST_CHECKED, DRAWITEMSTRUCT, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX,
            InitCommonControlsEx, ODS_SELECTED, SetWindowTheme,
        },
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        Input::KeyboardAndMouse::{SetFocus, VK_DELETE, VK_ESCAPE, VK_RETURN},
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
            Shell_NotifyIconW, ShellExecuteW,
        },
        WindowsAndMessaging::{
            AppendMenuW, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_OWNERDRAW, CB_ADDSTRING,
            CB_GETCURSEL, CB_SETCURSEL, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW,
            DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, EN_CHANGE,
            ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN, FindWindowW, GA_ROOT,
            GetAncestor, GetClientRect, GetCursorPos, GetForegroundWindow, GetMessageW,
            GetSystemMetrics, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, HMENU,
            HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW, IDI_APPLICATION, IDYES, IsWindowVisible,
            KillTimer, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, LB_SETITEMHEIGHT,
            LBN_DBLCLK, LBS_HASSTRINGS, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY, LBS_OWNERDRAWFIXED,
            LoadCursorW, LoadIconW, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_YESNO, MF_CHECKED,
            MF_SEPARATOR, MF_STRING, MSG, MessageBoxW, PostQuitMessage, RegisterClassExW,
            SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE,
            SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SendMessageW,
            SetForegroundWindow, SetLayeredWindowAttributes, SetTimer, SetWindowPos,
            SetWindowTextW, ShowWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD,
            TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
            WM_CLIPBOARDUPDATE, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLOREDIT,
            WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP,
            WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SETFONT, WM_TIMER, WNDCLASSEXW, WS_CAPTION,
            WS_CHILD, WS_CLIPCHILDREN, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_POPUP, WS_SYSMENU,
            WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
        },
    },
};

const MAIN_CLASS: &str = "Xmouse.Settings";
const HISTORY_CLASS: &str = "Xmouse.History";
const OVERLAY_CLASS: &str = "Xmouse.Overlay";
const TOAST_CLASS: &str = "Xmouse.Toast";
const APP_ICON_ID: usize = 1;

const IDC_ENABLED: i32 = 1001;
const IDC_TRIGGER: i32 = 1002;
const IDC_DELAY: i32 = 1003;
const IDC_DISTANCE: i32 = 1004;
const IDC_THRESHOLD: i32 = 1005;
const IDC_TRAIL: i32 = 1006;
const IDC_SEARCH_URL: i32 = 1007;
const IDC_AUTOSTART: i32 = 1008;
const IDC_CAPTURE: i32 = 1009;
const IDC_MAX_ITEMS: i32 = 1010;
const IDC_MAX_DISK: i32 = 1011;
const IDC_EXCLUDED: i32 = 1012;
const IDC_SAVE: i32 = 1013;
const IDC_OPEN_HISTORY: i32 = 1014;
const IDC_CLEAR_HISTORY: i32 = 1015;
const IDC_STATUS: i32 = 1016;
const IDC_ENCRYPT_CONTENT: i32 = 1017;
const IDC_NAV_GENERAL: i32 = 1018;
const IDC_NAV_HISTORY: i32 = 1019;
const IDC_OPEN_DATA_DIR: i32 = 1020;

const IDC_HISTORY_SEARCH: i32 = 1101;
const IDC_HISTORY_LIST: i32 = 1102;
const IDC_HISTORY_COPY: i32 = 1103;
const IDC_HISTORY_DELETE: i32 = 1104;
const IDC_HISTORY_CLEAR: i32 = 1105;

const TRAY_ID: u32 = 1;
const IDM_SETTINGS: usize = 2001;
const IDM_HISTORY: usize = 2002;
const IDM_PAUSE: usize = 2003;
const IDM_EXIT: usize = 2004;
const SS_LEFT_STYLE: u32 = 0;
const PAGE_COLOR: COLORREF = rgb(246, 248, 251);
const CARD_COLOR: COLORREF = rgb(255, 255, 255);
const BORDER_COLOR: COLORREF = rgb(225, 229, 236);
const TEXT_COLOR: COLORREF = rgb(31, 41, 55);
const MUTED_COLOR: COLORREF = rgb(107, 114, 128);
const ACCENT_COLOR: COLORREF = rgb(37, 99, 235);
const SIDEBAR_COLOR: COLORREF = rgb(243, 245, 248);

static APP_STATE: AtomicPtr<AppState> = AtomicPtr::new(ptr::null_mut());

struct Thumbnail {
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

struct HistoryView {
    item: ClipItem,
    thumbnail: Option<Thumbnail>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsPage {
    General,
    History,
}

struct Controls {
    status: HWND,
    page_title: HWND,
    page_subtitle: HWND,
    nav_general: HWND,
    nav_history: HWND,
    enabled: HWND,
    trigger: HWND,
    delay: HWND,
    distance: HWND,
    threshold: HWND,
    trail: HWND,
    search_url: HWND,
    autostart: HWND,
    capture: HWND,
    encrypt_content: HWND,
    max_items: HWND,
    max_disk: HWND,
    excluded: HWND,
    general_page: Vec<HWND>,
    history_page: Vec<HWND>,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            status: ptr::null_mut(),
            page_title: ptr::null_mut(),
            page_subtitle: ptr::null_mut(),
            nav_general: ptr::null_mut(),
            nav_history: ptr::null_mut(),
            enabled: ptr::null_mut(),
            trigger: ptr::null_mut(),
            delay: ptr::null_mut(),
            distance: ptr::null_mut(),
            threshold: ptr::null_mut(),
            trail: ptr::null_mut(),
            search_url: ptr::null_mut(),
            autostart: ptr::null_mut(),
            capture: ptr::null_mut(),
            encrypt_content: ptr::null_mut(),
            max_items: ptr::null_mut(),
            max_disk: ptr::null_mut(),
            excluded: ptr::null_mut(),
            general_page: Vec::new(),
            history_page: Vec::new(),
        }
    }
}

struct AppState {
    main_hwnd: HWND,
    history_hwnd: HWND,
    history_search: HWND,
    history_list: HWND,
    overlay_hwnd: HWND,
    toast_hwnd: HWND,
    config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
    clipboard: ClipboardService,
    storage: Storage,
    capture_sender: Sender<()>,
    controls: Controls,
    overlay_points: Vec<UiPoint>,
    toast_text: String,
    history_items: Vec<HistoryView>,
    history_origin: isize,
    tray_added: bool,
    font_body: HFONT,
    font_section: HFONT,
    font_title: HFONT,
    brush_page: HBRUSH,
    brush_card: HBRUSH,
    brush_sidebar: HBRUSH,
    active_settings_page: SettingsPage,
}

pub fn run() -> Result<()> {
    let start_in_background =
        std::env::args_os().any(|argument| argument == "--background" || argument == "--startup");
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32);
    }
    let Some(_single_instance) = single_instance()? else {
        return Ok(());
    };
    let config_path = config::config_path()?;
    let loaded = config::load_or_create();
    let load_warning = loaded.as_ref().err().map(|error| format!("{error:#}"));
    let config_value = loaded.unwrap_or_default();
    let config = Arc::new(RwLock::new(config_value));
    let root = config::app_data_dir()?;
    logging::init(&root)?;
    let storage = Storage::open(root, config.clone())?;
    let clipboard = ClipboardService::new(storage.clone(), config.clone());
    let (capture_sender, capture_receiver) = mpsc::channel();

    register_classes()?;
    initialize_common_controls();
    let font_body = create_ui_font(-16, FW_NORMAL as i32);
    let font_section = create_ui_font(-18, FW_SEMIBOLD as i32);
    let font_title = create_ui_font(-28, FW_SEMIBOLD as i32);
    let brush_page = unsafe { CreateSolidBrush(PAGE_COLOR) };
    let brush_card = unsafe { CreateSolidBrush(CARD_COLOR) };
    let brush_sidebar = unsafe { CreateSolidBrush(SIDEBAR_COLOR) };

    let state = Box::new(AppState {
        main_hwnd: ptr::null_mut(),
        history_hwnd: ptr::null_mut(),
        history_search: ptr::null_mut(),
        history_list: ptr::null_mut(),
        overlay_hwnd: ptr::null_mut(),
        toast_hwnd: ptr::null_mut(),
        config,
        config_path,
        clipboard: clipboard.clone(),
        storage: storage.clone(),
        capture_sender,
        controls: Controls::default(),
        overlay_points: Vec::with_capacity(512),
        toast_text: String::new(),
        history_items: Vec::new(),
        history_origin: 0,
        tray_added: false,
        font_body,
        font_section,
        font_title,
        brush_page,
        brush_card,
        brush_sidebar,
        active_settings_page: SettingsPage::General,
    });
    let state_pointer = Box::into_raw(state);
    APP_STATE.store(state_pointer, Ordering::Release);

    let main_hwnd = create_top_window(
        MAIN_CLASS,
        "Xmouse 设置",
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN,
        0,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        900,
        720,
    )?;
    unsafe {
        (*state_pointer).main_hwnd = main_hwnd;
    }
    let history_hwnd = create_top_window(
        HISTORY_CLASS,
        "剪贴板历史",
        WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
        WS_EX_TOOLWINDOW,
        0,
        0,
        600,
        520,
    )?;
    let overlay_hwnd = create_top_window(
        OVERLAY_CLASS,
        "",
        WS_POPUP,
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        0,
        0,
        1,
        1,
    )?;
    let toast_hwnd = create_top_window(
        TOAST_CLASS,
        "",
        WS_POPUP,
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
        0,
        0,
        320,
        52,
    )?;
    unsafe {
        (*state_pointer).history_hwnd = history_hwnd;
        (*state_pointer).overlay_hwnd = overlay_hwnd;
        (*state_pointer).toast_hwnd = toast_hwnd;
        SetLayeredWindowAttributes(overlay_hwnd, rgb(0, 0, 0), 255, 1);
        let toast_region = CreateRoundRectRgn(0, 0, 320, 52, 16, 16);
        SetWindowRgn(toast_hwnd, toast_region, 1);
    }
    hook::update_ui_hwnd(main_hwnd as isize);

    let (command_sender, command_receiver) = mpsc::channel::<HookCommand>();
    run_worker(
        command_receiver,
        unsafe { (*state_pointer).config.clone() },
        clipboard.clone(),
        main_hwnd as isize,
    );
    hook::start(
        unsafe { (*state_pointer).config.clone() },
        command_sender,
        main_hwnd as isize,
    )?;

    let capture_ui_hwnd = main_hwnd as isize;
    thread::Builder::new()
        .name("xmouse-clipboard".to_owned())
        .spawn(move || {
            while capture_receiver.recv().is_ok() {
                if let Err(error) = clipboard.capture_current() {
                    logging::error("记录剪贴板", &error);
                    post_toast(capture_ui_hwnd, &format!("剪贴板记录失败：{error:#}"));
                }
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        capture_ui_hwnd as HWND,
                        WM_APP_CAPTURE_DONE,
                        0,
                        0,
                    );
                }
            }
        })?;

    if !start_in_background {
        unsafe {
            ShowWindow(main_hwnd, SW_SHOW);
            SetForegroundWindow(main_hwnd);
        }
    }
    if let Some(warning) = load_warning {
        post_toast(main_hwnd as isize, &warning);
    }

    message_loop();

    APP_STATE.store(ptr::null_mut(), Ordering::Release);
    unsafe {
        DeleteObject((*state_pointer).font_body);
        DeleteObject((*state_pointer).font_section);
        DeleteObject((*state_pointer).font_title);
        DeleteObject((*state_pointer).brush_page);
        DeleteObject((*state_pointer).brush_card);
        DeleteObject((*state_pointer).brush_sidebar);
        drop(Box::from_raw(state_pointer));
        CoUninitialize();
    }
    Ok(())
}

pub fn fatal_error(message: &str) {
    let title = wide("Xmouse 启动失败");
    let message = wide(message);
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn single_instance() -> Result<Option<HANDLE>> {
    let name = wide("Local\\Xmouse.Singleton.v1");
    let handle = unsafe { CreateMutexW(ptr::null(), 1, name.as_ptr()) };
    if handle.is_null() {
        bail!("创建单实例互斥量失败");
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        let class = wide(MAIN_CLASS);
        let existing = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
        if !existing.is_null() {
            unsafe {
                ShowWindow(existing, SW_RESTORE);
                SetForegroundWindow(existing);
            }
        }
        unsafe {
            CloseHandle(handle);
        }
        return Ok(None);
    }
    Ok(Some(handle))
}

fn initialize_common_controls() {
    let controls = INITCOMMONCONTROLSEX {
        dwSize: mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_STANDARD_CLASSES,
    };
    unsafe {
        InitCommonControlsEx(&controls);
    }
}

fn create_ui_font(height: i32, weight: i32) -> HFONT {
    let face = wide("Segoe UI");
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            face.as_ptr(),
        )
    }
}

fn register_classes() -> Result<()> {
    let instance = unsafe { GetModuleHandleW(ptr::null()) } as HINSTANCE;
    let app_icon = load_app_icon(instance);
    for (name, proc, background) in [
        (
            MAIN_CLASS,
            Some(main_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
            (COLOR_WINDOW + 1) as HBRUSH,
        ),
        (
            HISTORY_CLASS,
            Some(history_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
            (COLOR_WINDOW + 1) as HBRUSH,
        ),
        (
            OVERLAY_CLASS,
            Some(overlay_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
            ptr::null_mut(),
        ),
        (
            TOAST_CLASS,
            Some(toast_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
            ptr::null_mut(),
        ),
    ] {
        let class_name = wide(name);
        let class = WNDCLASSEXW {
            cbSize: mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: app_icon,
            hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
            hbrBackground: background,
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: app_icon,
        };
        if unsafe { RegisterClassExW(&class) } == 0 {
            bail!("注册窗口类失败：{name}");
        }
    }
    Ok(())
}

fn load_app_icon(instance: HINSTANCE) -> *mut c_void {
    let icon = unsafe { LoadIconW(instance, APP_ICON_ID as *const u16) };
    if icon.is_null() {
        unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) }
    } else {
        icon
    }
}

#[allow(clippy::too_many_arguments)]
fn create_top_window(
    class: &str,
    title: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND> {
    let class = wide(class);
    let title = wide(title);
    let instance = unsafe { GetModuleHandleW(ptr::null()) } as HINSTANCE;
    let hwnd = unsafe {
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            title.as_ptr(),
            style,
            x,
            y,
            width,
            height,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    if hwnd.is_null() {
        bail!("创建原生窗口失败");
    }
    Ok(hwnd)
}

fn state_mut() -> Option<&'static mut AppState> {
    let pointer = APP_STATE.load(Ordering::Acquire);
    unsafe { pointer.as_mut() }
}

fn color_static_control(parent: HWND, control: HWND, hdc: *mut c_void) -> LRESULT {
    let Some(state) = state_mut() else {
        return (COLOR_WINDOW + 1) as LRESULT;
    };
    let mut rect = RECT::default();
    unsafe {
        GetWindowRect(control, &mut rect);
    }
    let mut origin = POINT {
        x: rect.left,
        y: rect.top,
    };
    unsafe {
        ScreenToClient(parent, &mut origin);
    }
    let (brush, background) = if parent == state.main_hwnd && origin.x < 190 {
        (state.brush_sidebar, SIDEBAR_COLOR)
    } else if parent == state.main_hwnd {
        (state.brush_card, CARD_COLOR)
    } else {
        (state.brush_page, PAGE_COLOR)
    };
    unsafe {
        SetBkColor(hdc, background);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(
            hdc,
            if control == state.controls.page_subtitle {
                MUTED_COLOR
            } else {
                TEXT_COLOR
            },
        );
    }
    brush as LRESULT
}

fn message_loop() {
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
        if pretranslate_history(&message) {
            continue;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn pretranslate_history(message: &MSG) -> bool {
    let Some(state) = state_mut() else {
        return false;
    };
    if state.history_hwnd.is_null()
        || unsafe { IsWindowVisible(state.history_hwnd) } == 0
        || message.message != WM_KEYDOWN
    {
        return false;
    }
    let root = unsafe { GetAncestor(message.hwnd, GA_ROOT) };
    if root != state.history_hwnd {
        return false;
    }
    match message.wParam as u16 {
        VK_RETURN => {
            copy_selected_history(state);
            true
        }
        VK_DELETE => {
            delete_selected_history(state);
            true
        }
        VK_ESCAPE => {
            hide_history(state);
            true
        }
        _ => false,
    }
}

unsafe extern "system" fn main_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CREATE => {
            if let Some(state) = state_mut() {
                state.main_hwnd = hwnd;
                create_settings_controls(state);
                load_config_into_controls(state);
                if unsafe { AddClipboardFormatListener(hwnd) } == 0 {
                    post_toast(hwnd as isize, "无法监听剪贴板更新");
                }
                add_tray_icon(state);
            }
            0
        }
        WM_COMMAND => {
            if let Some(state) = state_mut() {
                let id = loword(wparam) as i32;
                match id {
                    IDC_SAVE => match read_config_from_controls(state) {
                        Ok(config) => {
                            let encryption_changed = state
                                .config
                                .read()
                                .expect("config poisoned")
                                .history
                                .encrypt_content
                                != config.history.encrypt_content;
                            if let Err(error) = config::save(&state.config_path, &config)
                                .and_then(|_| set_autostart(config.autostart))
                            {
                                logging::error("保存设置", &error);
                                post_toast(hwnd as isize, &format!("保存失败：{error:#}"));
                            } else {
                                *state.config.write().expect("config poisoned") = config;
                                refresh_status(state);
                                post_toast(
                                    hwnd as isize,
                                    if encryption_changed {
                                        "设置已保存；加密选项仅影响新记录"
                                    } else {
                                        "设置已保存"
                                    },
                                );
                            }
                        }
                        Err(error) => post_toast(hwnd as isize, &format!("{error:#}")),
                    },
                    IDC_OPEN_HISTORY => show_history(state, hwnd as isize),
                    IDC_CLEAR_HISTORY => confirm_clear_history(state),
                    IDC_OPEN_DATA_DIR => open_data_directory(state),
                    IDC_NAV_GENERAL => show_settings_page(state, SettingsPage::General),
                    IDC_NAV_HISTORY => show_settings_page(state, SettingsPage::History),
                    id if id as usize == IDM_SETTINGS => unsafe {
                        ShowWindow(hwnd, SW_RESTORE);
                        SetForegroundWindow(hwnd);
                    },
                    id if id as usize == IDM_HISTORY => show_history(state, hwnd as isize),
                    id if id as usize == IDM_PAUSE => {
                        let mut config = state.config.write().expect("config poisoned");
                        config.enabled = !config.enabled;
                        let enabled = config.enabled;
                        drop(config);
                        load_config_into_controls(state);
                        post_toast(
                            hwnd as isize,
                            if enabled {
                                "手势已启用"
                            } else {
                                "手势已暂停"
                            },
                        );
                    }
                    id if id as usize == IDM_EXIT => unsafe {
                        DestroyWindow(hwnd);
                    },
                    _ => {}
                }
            }
            0
        }
        WM_CLIPBOARDUPDATE => {
            if let Some(state) = state_mut()
                && !state.clipboard.consume_ignored_update()
                && !state.clipboard.is_capture_suspended()
            {
                let _ = state.capture_sender.send(());
            }
            0
        }
        WM_APP_OVERLAY_BEGIN | WM_APP_OVERLAY_POINT => {
            if let Some(state) = state_mut() {
                let pointer = lparam as *mut UiPoint;
                if !pointer.is_null() {
                    let point = unsafe { *Box::from_raw(pointer) };
                    if message == WM_APP_OVERLAY_BEGIN {
                        state.overlay_points.clear();
                        show_overlay(state);
                    }
                    state.overlay_points.push(point);
                    unsafe {
                        InvalidateRect(state.overlay_hwnd, ptr::null(), 0);
                    }
                }
            }
            0
        }
        WM_APP_OVERLAY_END => {
            if let Some(state) = state_mut() {
                unsafe {
                    ShowWindow(state.overlay_hwnd, SW_HIDE);
                }
                state.overlay_points.clear();
            }
            0
        }
        WM_APP_SHOW_HISTORY => {
            if let Some(state) = state_mut() {
                show_history(state, lparam);
            }
            0
        }
        WM_APP_TOAST => {
            if let Some(state) = state_mut() {
                let pointer = lparam as *mut String;
                if !pointer.is_null() {
                    state.toast_text = unsafe { *Box::from_raw(pointer) };
                    show_toast(state);
                }
            }
            0
        }
        WM_APP_TRAY => {
            if let Some(state) = state_mut() {
                match lparam as u32 {
                    WM_LBUTTONUP => show_history(state, unsafe { GetForegroundWindow() } as isize),
                    WM_RBUTTONUP => show_tray_menu(state),
                    _ => {}
                }
            }
            0
        }
        WM_DRAWITEM => {
            let draw = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            match draw.CtlID as i32 {
                IDC_SAVE | IDC_OPEN_HISTORY | IDC_CLEAR_HISTORY | IDC_STATUS | IDC_NAV_GENERAL
                | IDC_NAV_HISTORY | IDC_OPEN_DATA_DIR => {
                    draw_button(draw);
                    1
                }
                _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        WM_CTLCOLORSTATIC => color_static_control(hwnd, lparam as HWND, wparam as *mut c_void),
        WM_CTLCOLOREDIT => {
            let hdc = wparam as *mut c_void;
            unsafe {
                SetBkColor(hdc, CARD_COLOR);
                SetTextColor(hdc, TEXT_COLOR);
            }
            state_mut()
                .map(|state| state.brush_card as LRESULT)
                .unwrap_or((COLOR_WINDOW + 1) as LRESULT)
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_main_window(hwnd);
            0
        }
        WM_CLOSE => {
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }
        WM_DESTROY => {
            if let Some(state) = state_mut() {
                unsafe {
                    RemoveClipboardFormatListener(hwnd);
                }
                remove_tray_icon(state);
            }
            unsafe {
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe extern "system" fn history_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_CREATE => {
            if let Some(state) = state_mut() {
                state.history_hwnd = hwnd;
                let title = label(hwnd, "剪贴板历史", 24, 18, 280, 30);
                set_control_font(title, state.font_title);
                let subtitle = label(hwnd, "搜索并复制文本或图片", 25, 50, 360, 22);
                set_control_font(subtitle, state.font_body);
                let search = create_control(
                    hwnd,
                    "EDIT",
                    "",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                    0,
                    24,
                    80,
                    552,
                    34,
                    IDC_HISTORY_SEARCH,
                );
                let list = create_control(
                    hwnd,
                    "LISTBOX",
                    "",
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_TABSTOP
                        | WS_VSCROLL
                        | LBS_NOTIFY as u32
                        | LBS_NOINTEGRALHEIGHT as u32
                        | LBS_OWNERDRAWFIXED as u32
                        | LBS_HASSTRINGS as u32,
                    0,
                    24,
                    128,
                    552,
                    304,
                    IDC_HISTORY_LIST,
                );
                create_control(
                    hwnd,
                    "BUTTON",
                    "复制",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                    0,
                    318,
                    448,
                    82,
                    38,
                    IDC_HISTORY_COPY,
                );
                create_control(
                    hwnd,
                    "BUTTON",
                    "删除",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                    0,
                    408,
                    448,
                    78,
                    38,
                    IDC_HISTORY_DELETE,
                );
                create_control(
                    hwnd,
                    "BUTTON",
                    "清空",
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                    0,
                    494,
                    448,
                    82,
                    38,
                    IDC_HISTORY_CLEAR,
                );
                state.history_search = search;
                state.history_list = list;
                round_control(search, 552, 34, 14);
                round_control(list, 552, 304, 12);
                unsafe {
                    SendMessageW(list, LB_SETITEMHEIGHT, 0, 64);
                    let cue = wide("搜索剪贴板内容或来源程序");
                    SendMessageW(search, 0x1501, 1, cue.as_ptr() as LPARAM);
                    let explorer = wide("Explorer");
                    SetWindowTheme(search, explorer.as_ptr(), ptr::null());
                    SetWindowTheme(list, explorer.as_ptr(), ptr::null());
                    let region = CreateRoundRectRgn(0, 0, 600, 520, 18, 18);
                    SetWindowRgn(hwnd, region, 1);
                }
            }
            0
        }
        WM_COMMAND => {
            if let Some(state) = state_mut() {
                let id = loword(wparam) as i32;
                let notification = hiword(wparam);
                match id {
                    IDC_HISTORY_SEARCH if notification == EN_CHANGE as u16 => {
                        let query = window_text(state.history_search);
                        rebuild_history(state, &query);
                    }
                    IDC_HISTORY_LIST if notification == LBN_DBLCLK as u16 => {
                        copy_selected_history(state)
                    }
                    IDC_HISTORY_COPY => copy_selected_history(state),
                    IDC_HISTORY_DELETE => delete_selected_history(state),
                    IDC_HISTORY_CLEAR => confirm_clear_history(state),
                    _ => {}
                }
            }
            0
        }
        WM_DRAWITEM => {
            let draw = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            if draw.CtlID as i32 == IDC_HISTORY_LIST {
                draw_history_item(draw);
                1
            } else if matches!(
                draw.CtlID as i32,
                IDC_HISTORY_COPY | IDC_HISTORY_DELETE | IDC_HISTORY_CLEAR
            ) {
                draw_button(draw);
                1
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_CTLCOLORSTATIC => color_static_control(hwnd, lparam as HWND, wparam as *mut c_void),
        WM_CTLCOLOREDIT => {
            let hdc = wparam as *mut c_void;
            unsafe {
                SetBkColor(hdc, CARD_COLOR);
                SetTextColor(hdc, TEXT_COLOR);
            }
            state_mut()
                .map(|state| state.brush_card as LRESULT)
                .unwrap_or((COLOR_WINDOW + 1) as LRESULT)
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_history_window(hwnd);
            0
        }
        WM_CLOSE => {
            if let Some(state) = state_mut() {
                hide_history(state);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            let mut rect = RECT::default();
            unsafe {
                GetClientRect(hwnd, &mut rect);
            }
            let background = unsafe { CreateSolidBrush(rgb(0, 0, 0)) };
            unsafe {
                FillRect(hdc, &rect, background);
                DeleteObject(background);
            }
            if let Some(state) = state_mut()
                && state.overlay_points.len() >= 2
            {
                let pen = unsafe { CreatePen(PS_SOLID, 4, rgb(25, 145, 255)) };
                let old = unsafe { SelectObject(hdc, pen) };
                let virtual_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
                let virtual_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
                let first = &state.overlay_points[0];
                unsafe {
                    MoveToEx(
                        hdc,
                        first.x - virtual_x,
                        first.y - virtual_y,
                        ptr::null_mut(),
                    );
                }
                for point in state.overlay_points.iter().skip(1) {
                    unsafe {
                        LineTo(hdc, point.x - virtual_x, point.y - virtual_y);
                    }
                }
                unsafe {
                    SelectObject(hdc, old);
                    DeleteObject(pen);
                }
            }
            unsafe {
                EndPaint(hwnd, &paint);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe extern "system" fn toast_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            let mut rect = RECT::default();
            unsafe {
                GetClientRect(hwnd, &mut rect);
            }
            let brush = unsafe { CreateSolidBrush(rgb(38, 38, 42)) };
            unsafe {
                FillRect(hdc, &rect, brush);
                DeleteObject(brush);
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, rgb(245, 245, 245));
            }
            if let Some(state) = state_mut() {
                let old_font = unsafe { SelectObject(hdc, state.font_body) };
                let text = wide(&state.toast_text);
                let mut text_rect = RECT {
                    left: 12,
                    top: 0,
                    right: rect.right - 12,
                    bottom: rect.bottom,
                };
                unsafe {
                    DrawTextW(
                        hdc,
                        text.as_ptr(),
                        -1,
                        &mut text_rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                    );
                    SelectObject(hdc, old_font);
                }
            }
            unsafe {
                EndPaint(hwnd, &paint);
            }
            0
        }
        WM_TIMER => {
            unsafe {
                KillTimer(hwnd, 1);
                ShowWindow(hwnd, SW_HIDE);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_settings_controls(state: &mut AppState) {
    let hwnd = state.main_hwnd;
    let brand = label(hwnd, "Xmouse", 72, 25, 100, 30);
    set_control_font(brand, state.font_section);
    label(hwnd, "轻量效率工具", 72, 54, 100, 20);

    state.controls.nav_general = create_control(
        hwnd,
        "BUTTON",
        "常规",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        14,
        104,
        162,
        42,
        IDC_NAV_GENERAL,
    );
    state.controls.nav_history = create_control(
        hwnd,
        "BUTTON",
        "剪贴板历史",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        14,
        154,
        162,
        42,
        IDC_NAV_HISTORY,
    );
    label(hwnd, "v0.3.0", 22, 636, 120, 22);

    state.controls.page_title = label(hwnd, "常规", 220, 24, 300, 34);
    set_control_font(state.controls.page_title, state.font_title);
    state.controls.page_subtitle = label(hwnd, "配置鼠标手势、触发参数与搜索", 221, 57, 440, 22);
    state.controls.status = create_control(
        hwnd,
        "BUTTON",
        "运行中",
        WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32,
        0,
        758,
        27,
        104,
        32,
        IDC_STATUS,
    );

    let mut general_page = Vec::new();
    let mut history_page = Vec::new();

    let startup_title = label(hwnd, "启动", 220, 110, 160, 26);
    set_control_font(startup_title, state.font_section);
    general_page.push(startup_title);
    state.controls.enabled = create_control(
        hwnd,
        "BUTTON",
        "启用鼠标手势",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        0,
        220,
        149,
        180,
        24,
        IDC_ENABLED,
    );
    general_page.push(state.controls.enabled);
    state.controls.autostart = create_control(
        hwnd,
        "BUTTON",
        "登录后自动启动",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        0,
        430,
        149,
        180,
        24,
        IDC_AUTOSTART,
    );
    general_page.push(state.controls.autostart);

    let trigger_title = label(hwnd, "操作触发参数", 220, 204, 220, 26);
    set_control_font(trigger_title, state.font_section);
    general_page.push(trigger_title);
    general_page.push(label(hwnd, "触发按键", 220, 244, 100, 22));
    state.controls.trigger = create_control(
        hwnd,
        "COMBOBOX",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | 0x0003u32,
        0,
        220,
        269,
        176,
        160,
        IDC_TRIGGER,
    );
    round_control(state.controls.trigger, 176, 34, 14);
    general_page.push(state.controls.trigger);
    for button in [TriggerButton::Right, TriggerButton::X1, TriggerButton::X2] {
        let value = wide(button.display_name());
        unsafe {
            SendMessageW(
                state.controls.trigger,
                CB_ADDSTRING,
                0,
                value.as_ptr() as LPARAM,
            );
        }
    }

    general_page.push(label(hwnd, "触发延时（ms）", 430, 244, 130, 22));
    state.controls.delay = edit(hwnd, 430, 269, 150, 34, IDC_DELAY);
    general_page.push(state.controls.delay);
    general_page.push(label(hwnd, "触发距离（DIP）", 614, 244, 140, 22));
    state.controls.distance = edit(hwnd, 614, 269, 150, 34, IDC_DISTANCE);
    general_page.push(state.controls.distance);
    state.controls.trail = create_control(
        hwnd,
        "BUTTON",
        "显示手势轨迹",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        0,
        220,
        323,
        180,
        24,
        IDC_TRAIL,
    );
    general_page.push(state.controls.trail);
    general_page.push(label(hwnd, "识别阈值", 430, 323, 92, 22));
    state.controls.threshold = edit(hwnd, 526, 318, 96, 34, IDC_THRESHOLD);
    general_page.push(state.controls.threshold);

    let search_title = label(hwnd, "搜索", 220, 380, 160, 26);
    set_control_font(search_title, state.font_section);
    general_page.push(search_title);
    general_page.push(label(
        hwnd,
        "搜索网址模板  ·  必须包含 {query}",
        220,
        418,
        360,
        22,
    ));
    state.controls.search_url = edit(hwnd, 220, 443, 620, 36, IDC_SEARCH_URL);
    general_page.push(state.controls.search_url);
    general_page.push(label(
        hwnd,
        "Edge 内置手势请在“设置 → 外观 → 鼠标手势”中关闭，或将 Xmouse 改用侧键。",
        220,
        487,
        630,
        22,
    ));

    let mapping_title = label(hwnd, "快捷手势", 220, 526, 160, 26);
    set_control_font(mapping_title, state.font_section);
    general_page.push(mapping_title);
    general_page.push(label(
        hwnd,
        "↑ 置顶切换    L 关闭页面    S 搜索    C 复制    V 剪贴板历史",
        220,
        564,
        630,
        24,
    ));

    let history_title = label(hwnd, "记录", 220, 110, 160, 26);
    set_control_font(history_title, state.font_section);
    history_page.push(history_title);
    state.controls.capture = create_control(
        hwnd,
        "BUTTON",
        "记录文本和图片",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        0,
        220,
        149,
        180,
        24,
        IDC_CAPTURE,
    );
    history_page.push(state.controls.capture);
    state.controls.encrypt_content = create_control(
        hwnd,
        "BUTTON",
        "使用 Windows DPAPI 加密新记录",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        0,
        430,
        149,
        280,
        24,
        IDC_ENCRYPT_CONTENT,
    );
    history_page.push(state.controls.encrypt_content);
    history_page.push(label(
        hwnd,
        "默认明文：文本直接保存在 history.db，图片保存在 media 文件夹，便于个人调试查看。",
        220,
        180,
        630,
        22,
    ));

    let capacity_title = label(hwnd, "容量", 220, 229, 160, 26);
    set_control_font(capacity_title, state.font_section);
    history_page.push(capacity_title);
    history_page.push(label(hwnd, "最多条目", 220, 269, 90, 22));
    state.controls.max_items = edit(hwnd, 220, 294, 160, 34, IDC_MAX_ITEMS);
    history_page.push(state.controls.max_items);
    history_page.push(label(hwnd, "磁盘上限（MiB）", 430, 269, 140, 22));
    state.controls.max_disk = edit(hwnd, 430, 294, 160, 34, IDC_MAX_DISK);
    history_page.push(state.controls.max_disk);

    let exclude_title = label(hwnd, "排除程序", 220, 359, 180, 26);
    set_control_font(exclude_title, state.font_section);
    history_page.push(exclude_title);
    history_page.push(label(
        hwnd,
        "不记录以下进程的剪贴板内容  ·  exe 名称，每行或逗号分隔",
        220,
        397,
        420,
        22,
    ));
    state.controls.excluded = create_control(
        hwnd,
        "EDIT",
        "",
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_WANTRETURN as u32,
        0,
        220,
        423,
        620,
        92,
        IDC_EXCLUDED,
    );
    round_control(state.controls.excluded, 620, 92, 12);
    history_page.push(state.controls.excluded);

    let open_history = create_control(
        hwnd,
        "BUTTON",
        "打开历史",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        220,
        547,
        128,
        40,
        IDC_OPEN_HISTORY,
    );
    history_page.push(open_history);
    let clear_history = create_control(
        hwnd,
        "BUTTON",
        "清空历史",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        360,
        547,
        128,
        40,
        IDC_CLEAR_HISTORY,
    );
    history_page.push(clear_history);
    let open_data = create_control(
        hwnd,
        "BUTTON",
        "打开数据目录",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        500,
        547,
        138,
        40,
        IDC_OPEN_DATA_DIR,
    );
    history_page.push(open_data);
    history_page.push(label(
        hwnd,
        "切换加密选项只影响之后的新记录；旧记录保持原格式，仍可正常读取。",
        220,
        596,
        600,
        22,
    ));

    create_control(
        hwnd,
        "BUTTON",
        "保存设置",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        734,
        625,
        128,
        40,
        IDC_SAVE,
    );

    let explorer = wide("Explorer");
    for control in [
        state.controls.enabled,
        state.controls.autostart,
        state.controls.trigger,
        state.controls.delay,
        state.controls.distance,
        state.controls.threshold,
        state.controls.trail,
        state.controls.search_url,
        state.controls.capture,
        state.controls.encrypt_content,
        state.controls.max_items,
        state.controls.max_disk,
        state.controls.excluded,
    ] {
        unsafe {
            SetWindowTheme(control, explorer.as_ptr(), ptr::null());
        }
    }
    state.controls.general_page = general_page;
    state.controls.history_page = history_page;
    show_settings_page(state, SettingsPage::General);
}

fn show_settings_page(state: &mut AppState, page: SettingsPage) {
    state.active_settings_page = page;
    for &control in &state.controls.general_page {
        unsafe {
            ShowWindow(
                control,
                if page == SettingsPage::General {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
    }
    for &control in &state.controls.history_page {
        unsafe {
            ShowWindow(
                control,
                if page == SettingsPage::History {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
    }
    let (title, subtitle) = match page {
        SettingsPage::General => ("常规", "配置鼠标手势、触发参数与搜索"),
        SettingsPage::History => ("剪贴板历史", "管理记录方式、容量与排除程序"),
    };
    set_control_text(state.controls.page_title, title);
    set_control_text(state.controls.page_subtitle, subtitle);
    unsafe {
        InvalidateRect(state.controls.nav_general, ptr::null(), 1);
        InvalidateRect(state.controls.nav_history, ptr::null(), 1);
        InvalidateRect(state.main_hwnd, ptr::null(), 1);
    }
}

#[allow(clippy::too_many_arguments)]
fn create_control(
    parent: HWND,
    class: &str,
    text: &str,
    style: WINDOW_STYLE,
    ex_style: WINDOW_EX_STYLE,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: i32,
) -> HWND {
    let class = wide(class);
    let text = wide(text);
    let instance = unsafe { GetModuleHandleW(ptr::null()) } as HINSTANCE;
    let control = unsafe {
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            text.as_ptr(),
            style,
            x,
            y,
            width,
            height,
            parent,
            id as usize as HMENU,
            instance,
            ptr::null(),
        )
    };
    if !control.is_null()
        && let Some(state) = state_mut()
    {
        set_control_font(control, state.font_body);
    }
    control
}

fn set_control_font(control: HWND, font: HFONT) {
    if !control.is_null() && !font.is_null() {
        unsafe {
            SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
        }
    }
}

fn label(parent: HWND, text: &str, x: i32, y: i32, width: i32, height: i32) -> HWND {
    create_control(
        parent,
        "STATIC",
        text,
        WS_CHILD | WS_VISIBLE | SS_LEFT_STYLE,
        0,
        x,
        y,
        width,
        height,
        0,
    )
}

fn edit(parent: HWND, x: i32, y: i32, width: i32, height: i32, id: i32) -> HWND {
    let control = create_control(
        parent,
        "EDIT",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        0,
        x,
        y,
        width,
        height,
        id,
    );
    round_control(control, width, height, height.min(16));
    control
}

fn round_control(control: HWND, width: i32, height: i32, radius: i32) {
    if control.is_null() {
        return;
    }
    let region = unsafe { CreateRoundRectRgn(0, 0, width + 1, height + 1, radius * 2, radius * 2) };
    unsafe {
        SetWindowRgn(control, region, 1);
    }
}

fn load_config_into_controls(state: &mut AppState) {
    let config = state.config.read().expect("config poisoned").clone();
    set_check(state.controls.enabled, config.enabled);
    set_check(state.controls.autostart, config.autostart);
    set_check(state.controls.trail, config.show_trail);
    set_check(state.controls.capture, config.history.capture);
    set_check(
        state.controls.encrypt_content,
        config.history.encrypt_content,
    );
    unsafe {
        SendMessageW(
            state.controls.trigger,
            CB_SETCURSEL,
            config.trigger.index(),
            0,
        );
    }
    set_control_text(
        state.controls.delay,
        &config.activation_delay_ms.to_string(),
    );
    set_control_text(
        state.controls.distance,
        &format!("{:.1}", config.activation_distance_dip),
    );
    set_control_text(
        state.controls.threshold,
        &format!("{:.2}", config.recognition_threshold),
    );
    set_control_text(state.controls.search_url, &config.search_url_template);
    set_control_text(
        state.controls.max_items,
        &config.history.max_items.to_string(),
    );
    set_control_text(
        state.controls.max_disk,
        &config.history.max_disk_mib.to_string(),
    );
    set_control_text(
        state.controls.excluded,
        &config.history.excluded_processes.join("\r\n"),
    );
    refresh_status(state);
}

fn refresh_status(state: &AppState) {
    let enabled = state.config.read().expect("config poisoned").enabled;
    set_control_text(
        state.controls.status,
        if enabled {
            "● 运行中"
        } else {
            "● 已暂停"
        },
    );
    unsafe {
        InvalidateRect(state.controls.status, ptr::null(), 1);
    }
}

fn read_config_from_controls(state: &AppState) -> Result<AppConfig> {
    let current = state.config.read().expect("config poisoned").clone();
    let trigger_index =
        unsafe { SendMessageW(state.controls.trigger, CB_GETCURSEL, 0, 0) }.max(0) as usize;
    let excluded_processes = window_text(state.controls.excluded)
        .split([',', ';', '\r', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    let config = AppConfig {
        schema_version: 1,
        enabled: is_checked(state.controls.enabled),
        trigger: TriggerButton::from_index(trigger_index),
        activation_delay_ms: window_text(state.controls.delay)
            .parse()
            .context("触发延时不是有效整数")?,
        activation_distance_dip: window_text(state.controls.distance)
            .parse()
            .context("触发距离不是有效数字")?,
        recognition_threshold: window_text(state.controls.threshold)
            .parse()
            .context("识别阈值不是有效数字")?,
        show_trail: is_checked(state.controls.trail),
        search_url_template: window_text(state.controls.search_url),
        autostart: is_checked(state.controls.autostart),
        history: crate::config::HistoryConfig {
            capture: is_checked(state.controls.capture),
            encrypt_content: is_checked(state.controls.encrypt_content),
            max_items: window_text(state.controls.max_items)
                .parse()
                .context("历史条目数量不是有效整数")?,
            max_disk_mib: window_text(state.controls.max_disk)
                .parse()
                .context("磁盘上限不是有效整数")?,
            excluded_processes,
            ..current.history
        },
    };
    config.validate()?;
    Ok(config)
}

fn show_history(state: &mut AppState, origin: isize) {
    state.history_origin = origin;
    set_control_text(state.history_search, "");
    rebuild_history(state, "");
    let mut cursor = POINT::default();
    unsafe {
        GetCursorPos(&mut cursor);
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(monitor, &mut info);
    }
    let width = 600;
    let height = 520;
    let x = cursor
        .x
        .min(info.rcWork.right - width)
        .max(info.rcWork.left);
    let y = cursor
        .y
        .min(info.rcWork.bottom - height)
        .max(info.rcWork.top);
    unsafe {
        SetWindowPos(
            state.history_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_SHOWWINDOW,
        );
        ShowWindow(state.history_hwnd, SW_SHOW);
        SetForegroundWindow(state.history_hwnd);
        SetFocus(state.history_search);
    }
}

fn hide_history(state: &mut AppState) {
    unsafe {
        ShowWindow(state.history_hwnd, SW_HIDE);
    }
    let origin = state.history_origin as HWND;
    if !origin.is_null() {
        unsafe {
            SetForegroundWindow(origin);
        }
    }
}

fn rebuild_history(state: &mut AppState, query: &str) {
    match state.storage.list(query) {
        Ok(items) => {
            state.history_items = items
                .into_iter()
                .map(|item| {
                    let thumbnail = item.thumbnail_png.as_deref().and_then(decode_thumbnail);
                    HistoryView { item, thumbnail }
                })
                .collect();
            unsafe {
                SendMessageW(state.history_list, LB_RESETCONTENT, 0, 0);
            }
            for view in &state.history_items {
                let text = wide(&view.item.display_text());
                unsafe {
                    SendMessageW(state.history_list, LB_ADDSTRING, 0, text.as_ptr() as LPARAM);
                }
            }
            if !state.history_items.is_empty() {
                unsafe {
                    SendMessageW(state.history_list, LB_SETCURSEL, 0, 0);
                }
            }
        }
        Err(error) => post_toast(
            state.main_hwnd as isize,
            &format!("读取历史失败：{error:#}"),
        ),
    }
}

fn selected_history_index(state: &AppState) -> Option<usize> {
    let index = unsafe { SendMessageW(state.history_list, LB_GETCURSEL, 0, 0) };
    (index >= 0 && (index as usize) < state.history_items.len()).then_some(index as usize)
}

fn copy_selected_history(state: &mut AppState) {
    let Some(index) = selected_history_index(state) else {
        return;
    };
    let id = state.history_items[index].item.id;
    match state
        .storage
        .payload(id)
        .and_then(|payload| state.clipboard.set_payload(&payload))
        .and_then(|_| state.storage.touch(id))
    {
        Ok(()) => {
            hide_history(state);
            post_toast(state.main_hwnd as isize, "已复制历史内容");
        }
        Err(error) => post_toast(
            state.main_hwnd as isize,
            &format!("复制历史失败：{error:#}"),
        ),
    }
}

fn delete_selected_history(state: &mut AppState) {
    let Some(index) = selected_history_index(state) else {
        return;
    };
    let id = state.history_items[index].item.id;
    match state.storage.remove(id) {
        Ok(()) => {
            let query = window_text(state.history_search);
            rebuild_history(state, &query);
        }
        Err(error) => post_toast(
            state.main_hwnd as isize,
            &format!("删除历史失败：{error:#}"),
        ),
    }
}

fn confirm_clear_history(state: &mut AppState) {
    let text = wide("确定删除全部剪贴板历史吗？");
    let title = wide("Xmouse");
    let answer = unsafe {
        MessageBoxW(
            state.main_hwnd,
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        )
    };
    if answer == IDYES {
        match state.storage.clear() {
            Ok(()) => {
                state.history_items.clear();
                unsafe {
                    SendMessageW(state.history_list, LB_RESETCONTENT, 0, 0);
                }
                post_toast(state.main_hwnd as isize, "剪贴板历史已清空");
            }
            Err(error) => post_toast(state.main_hwnd as isize, &format!("清空失败：{error:#}")),
        }
    }
}

fn open_data_directory(state: &AppState) {
    let Ok(path) = config::app_data_dir() else {
        post_toast(state.main_hwnd as isize, "无法定位数据目录");
        return;
    };
    let verb = wide("open");
    let path = wide(&path.to_string_lossy());
    let result = unsafe {
        ShellExecuteW(
            state.main_hwnd,
            verb.as_ptr(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            1,
        )
    };
    if result as isize <= 32 {
        post_toast(state.main_hwnd as isize, "打开数据目录失败");
    }
}

fn paint_main_window(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
    }
    let Some(state) = state_mut() else {
        unsafe {
            EndPaint(hwnd, &paint);
        }
        return;
    };
    unsafe {
        FillRect(hdc, &client, state.brush_sidebar);
    }
    let content = RECT {
        left: 190,
        top: 0,
        right: client.right,
        bottom: client.bottom,
    };
    unsafe {
        FillRect(hdc, &content, state.brush_card);
    }
    let separator = unsafe { CreatePen(PS_SOLID, 1, BORDER_COLOR) };
    let old_separator = unsafe { SelectObject(hdc, separator) };
    unsafe {
        MoveToEx(hdc, 189, 0, ptr::null_mut());
        LineTo(hdc, 189, client.bottom);
        MoveToEx(hdc, 190, 82, ptr::null_mut());
        LineTo(hdc, client.right, 82);
    }
    let section_lines: &[i32] = match state.active_settings_page {
        SettingsPage::General => &[190, 366, 512],
        SettingsPage::History => &[214, 346, 529],
    };
    for y in section_lines {
        unsafe {
            MoveToEx(hdc, 220, *y, ptr::null_mut());
            LineTo(hdc, 840, *y);
        }
    }
    unsafe {
        SelectObject(hdc, old_separator);
        DeleteObject(separator);
    }
    let input_frames: &[(i32, i32, i32, i32, i32)] = match state.active_settings_page {
        SettingsPage::General => &[
            (216, 265, 400, 307, 34),
            (426, 265, 584, 307, 34),
            (610, 265, 768, 307, 34),
            (522, 314, 626, 356, 34),
            (216, 439, 844, 483, 36),
        ],
        SettingsPage::History => &[
            (216, 290, 384, 332, 34),
            (426, 290, 594, 332, 34),
            (216, 419, 844, 519, 28),
        ],
    };
    for &(left, top, right, bottom, radius) in input_frames {
        draw_rounded_panel(hdc, left, top, right, bottom, radius);
    }

    let logo_brush = unsafe { CreateSolidBrush(ACCENT_COLOR) };
    let logo_pen = unsafe { CreatePen(PS_SOLID, 1, ACCENT_COLOR) };
    let old_brush = unsafe { SelectObject(hdc, logo_brush) };
    let old_pen = unsafe { SelectObject(hdc, logo_pen) };
    unsafe {
        RoundRect(hdc, 22, 24, 58, 60, 12, 12);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(logo_brush);
        DeleteObject(logo_pen);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, rgb(255, 255, 255));
    }
    let old_font = unsafe { SelectObject(hdc, state.font_section) };
    let mut logo_rect = RECT {
        left: 22,
        top: 24,
        right: 58,
        bottom: 60,
    };
    let logo = wide("X");
    unsafe {
        DrawTextW(
            hdc,
            logo.as_ptr(),
            -1,
            &mut logo_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, old_font);
    }
    unsafe {
        EndPaint(hwnd, &paint);
    }
}

fn paint_history_window(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    let mut client = RECT::default();
    unsafe {
        GetClientRect(hwnd, &mut client);
    }
    let page = unsafe { CreateSolidBrush(PAGE_COLOR) };
    unsafe {
        FillRect(hdc, &client, page);
        DeleteObject(page);
    }
    draw_rounded_panel(hdc, 16, 70, 584, 440, 16);
    draw_rounded_panel(hdc, 23, 79, 577, 115, 30);
    draw_rounded_panel(hdc, 23, 127, 577, 433, 26);
    unsafe {
        EndPaint(hwnd, &paint);
    }
}

fn draw_rounded_panel(hdc: *mut c_void, left: i32, top: i32, right: i32, bottom: i32, radius: i32) {
    let brush = unsafe { CreateSolidBrush(CARD_COLOR) };
    let pen = unsafe { CreatePen(PS_SOLID, 1, BORDER_COLOR) };
    let old_brush = unsafe { SelectObject(hdc, brush) };
    let old_pen = unsafe { SelectObject(hdc, pen) };
    unsafe {
        RoundRect(hdc, left, top, right, bottom, radius, radius);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}

fn draw_button(draw: &DRAWITEMSTRUCT) {
    let id = draw.CtlID as i32;
    let pressed = draw.itemState & ODS_SELECTED != 0;
    let (enabled, active_page) = state_mut()
        .map(|state| {
            (
                state.config.read().expect("config poisoned").enabled,
                state.active_settings_page,
            )
        })
        .unwrap_or((true, SettingsPage::General));
    let nav_active = matches!(
        (id, active_page),
        (IDC_NAV_GENERAL, SettingsPage::General) | (IDC_NAV_HISTORY, SettingsPage::History)
    );
    let (fill, border, text_color, radius) = if id == IDC_STATUS {
        if enabled {
            (rgb(231, 248, 239), rgb(191, 229, 207), rgb(22, 125, 74), 16)
        } else {
            (
                rgb(255, 247, 225),
                rgb(244, 218, 151),
                rgb(164, 104, 16),
                16,
            )
        }
    } else if matches!(id, IDC_NAV_GENERAL | IDC_NAV_HISTORY) {
        (
            if nav_active {
                rgb(229, 237, 255)
            } else if pressed {
                rgb(232, 235, 240)
            } else {
                SIDEBAR_COLOR
            },
            if nav_active {
                rgb(229, 237, 255)
            } else {
                SIDEBAR_COLOR
            },
            if nav_active { ACCENT_COLOR } else { TEXT_COLOR },
            10,
        )
    } else if matches!(id, IDC_SAVE | IDC_HISTORY_COPY) {
        (
            if pressed {
                rgb(29, 78, 216)
            } else {
                ACCENT_COLOR
            },
            if pressed {
                rgb(29, 78, 216)
            } else {
                ACCENT_COLOR
            },
            rgb(255, 255, 255),
            10,
        )
    } else {
        (
            if pressed {
                rgb(238, 242, 247)
            } else {
                CARD_COLOR
            },
            BORDER_COLOR,
            if matches!(
                id,
                IDC_CLEAR_HISTORY | IDC_HISTORY_DELETE | IDC_HISTORY_CLEAR
            ) {
                rgb(190, 45, 55)
            } else {
                TEXT_COLOR
            },
            10,
        )
    };

    let brush = unsafe { CreateSolidBrush(fill) };
    let pen = unsafe { CreatePen(PS_SOLID, 1, border) };
    let old_brush = unsafe { SelectObject(draw.hDC, brush) };
    let old_pen = unsafe { SelectObject(draw.hDC, pen) };
    unsafe {
        RoundRect(
            draw.hDC,
            draw.rcItem.left,
            draw.rcItem.top,
            draw.rcItem.right - 1,
            draw.rcItem.bottom - 1,
            radius,
            radius,
        );
        SelectObject(draw.hDC, old_brush);
        SelectObject(draw.hDC, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(draw.hDC, text_color);
    }
    if nav_active {
        let accent = unsafe { CreateSolidBrush(ACCENT_COLOR) };
        let marker = RECT {
            left: draw.rcItem.left + 1,
            top: draw.rcItem.top + 9,
            right: draw.rcItem.left + 4,
            bottom: draw.rcItem.bottom - 9,
        };
        unsafe {
            FillRect(draw.hDC, &marker, accent);
            DeleteObject(accent);
        }
    }

    let text = wide(&window_text(draw.hwndItem));
    let mut text_rect = draw.rcItem;
    let text_flags = if matches!(id, IDC_NAV_GENERAL | IDC_NAV_HISTORY) {
        text_rect.left += 20;
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
    } else {
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
    };
    if pressed && id != IDC_STATUS {
        text_rect.top += 1;
    }
    if let Some(state) = state_mut() {
        let old_font = unsafe { SelectObject(draw.hDC, state.font_body) };
        unsafe {
            DrawTextW(draw.hDC, text.as_ptr(), -1, &mut text_rect, text_flags);
            SelectObject(draw.hDC, old_font);
        }
    }
}

fn draw_history_item(draw: &DRAWITEMSTRUCT) {
    if draw.itemID == u32::MAX {
        return;
    }
    let Some(state) = state_mut() else {
        return;
    };
    let Some(view) = state.history_items.get(draw.itemID as usize) else {
        return;
    };
    let selected = draw.itemState & ODS_SELECTED != 0;
    let background_color = if selected {
        rgb(239, 246, 255)
    } else {
        CARD_COLOR
    };
    let brush = unsafe { CreateSolidBrush(background_color) };
    unsafe {
        FillRect(draw.hDC, &draw.rcItem, brush);
        DeleteObject(brush);
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(draw.hDC, TEXT_COLOR);
    }
    if selected {
        let accent = unsafe { CreateSolidBrush(ACCENT_COLOR) };
        let stripe = RECT {
            left: draw.rcItem.left,
            top: draw.rcItem.top + 6,
            right: draw.rcItem.left + 3,
            bottom: draw.rcItem.bottom - 6,
        };
        unsafe {
            FillRect(draw.hDC, &stripe, accent);
            DeleteObject(accent);
        }
    }

    let image_left = draw.rcItem.left + 10;
    let content_left = if let Some(thumbnail) = &view.thumbnail {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: thumbnail.width,
                biHeight: -thumbnail.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default(); 1],
        };
        unsafe {
            StretchDIBits(
                draw.hDC,
                image_left,
                draw.rcItem.top + 8,
                48,
                48,
                0,
                0,
                thumbnail.width,
                thumbnail.height,
                thumbnail.bgra.as_ptr() as *const c_void,
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
        draw.rcItem.left + 68
    } else {
        draw.rcItem.left + 14
    };

    let old_font = state.font_body;
    let previous_font = unsafe { SelectObject(draw.hDC, old_font) };
    let text = wide(&view.item.display_text());
    let mut first_line = RECT {
        left: content_left,
        top: draw.rcItem.top + 7,
        right: draw.rcItem.right - 8,
        bottom: draw.rcItem.top + 35,
    };
    unsafe {
        DrawTextW(
            draw.hDC,
            text.as_ptr(),
            -1,
            &mut first_line,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        SetTextColor(draw.hDC, MUTED_COLOR);
    }
    let relative_time = relative_history_time(view.item.last_used_at);
    let source = if view.item.source_exe.is_empty() {
        format!(
            "未知来源 · {} · {} KiB",
            relative_time,
            view.item.byte_size.div_ceil(1024)
        )
    } else {
        format!(
            "{} · {} · {} KiB",
            view.item.source_exe,
            relative_time,
            view.item.byte_size.div_ceil(1024)
        )
    };
    let source = wide(&source);
    let mut second_line = RECT {
        left: content_left,
        top: draw.rcItem.top + 35,
        right: draw.rcItem.right - 8,
        bottom: draw.rcItem.bottom - 4,
    };
    unsafe {
        DrawTextW(
            draw.hDC,
            source.as_ptr(),
            -1,
            &mut second_line,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        SelectObject(draw.hDC, previous_font);
    }
}

fn decode_thumbnail(png: &[u8]) -> Option<Thumbnail> {
    let image = image::load_from_memory(png)
        .ok()?
        .thumbnail(48, 48)
        .to_rgba8();
    let (width, height) = image.dimensions();
    let mut bgra = image.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(Thumbnail {
        width: width as i32,
        height: height as i32,
        bgra,
    })
}

fn show_overlay(state: &mut AppState) {
    if !state.config.read().expect("config poisoned").show_trail {
        return;
    }
    let x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    unsafe {
        SetWindowPos(
            state.overlay_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        ShowWindow(state.overlay_hwnd, SW_SHOWNOACTIVATE);
    }
}

fn show_toast(state: &mut AppState) {
    let mut cursor = POINT::default();
    unsafe {
        GetCursorPos(&mut cursor);
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(monitor, &mut info);
    }
    let width = 320;
    let height = 52;
    let x = (cursor.x + 20)
        .min(info.rcWork.right - width)
        .max(info.rcWork.left);
    let y = (cursor.y + 24)
        .min(info.rcWork.bottom - height)
        .max(info.rcWork.top);
    unsafe {
        InvalidateRect(state.toast_hwnd, ptr::null(), 0);
        SetWindowPos(
            state.toast_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        ShowWindow(state.toast_hwnd, SW_SHOWNOACTIVATE);
        SetTimer(state.toast_hwnd, 1, 1_300, None);
    }
}

fn add_tray_icon(state: &mut AppState) {
    let mut data = tray_data(state.main_hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_APP_TRAY;
    let instance = unsafe { GetModuleHandleW(ptr::null()) } as HINSTANCE;
    data.hIcon = load_app_icon(instance);
    copy_wide_fixed(&mut data.szTip, "Xmouse");
    if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0 {
        state.tray_added = true;
    }
}

fn remove_tray_icon(state: &mut AppState) {
    if state.tray_added {
        let data = tray_data(state.main_hwnd);
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &data);
        }
        state.tray_added = false;
    }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

fn show_tray_menu(state: &mut AppState) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    append_menu(menu, MF_STRING, IDM_HISTORY, "剪贴板历史");
    append_menu(menu, MF_STRING, IDM_SETTINGS, "设置");
    let paused = !state.config.read().expect("config poisoned").enabled;
    append_menu(
        menu,
        MF_STRING | if paused { MF_CHECKED } else { 0 },
        IDM_PAUSE,
        "暂停手势",
    );
    unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    }
    append_menu(menu, MF_STRING, IDM_EXIT, "退出 Xmouse");
    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point);
        SetForegroundWindow(state.main_hwnd);
    }
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
            point.x,
            point.y,
            0,
            state.main_hwnd,
            ptr::null(),
        )
    };
    if command != 0 {
        unsafe {
            SendMessageW(state.main_hwnd, WM_COMMAND, command as WPARAM, 0);
        }
    }
    unsafe {
        DestroyMenu(menu);
    }
}

fn append_menu(menu: HMENU, flags: u32, id: usize, text: &str) {
    let text = wide(text);
    unsafe {
        AppendMenuW(menu, flags, id, text.as_ptr());
    }
}

fn set_autostart(enabled: bool) -> Result<()> {
    let key_path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let value_name = wide("Xmouse");
    let mut key: HKEY = ptr::null_mut();
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    };
    if result != 0 {
        bail!("无法打开开机启动注册表项（错误 {result}）");
    }
    let operation_result = if enabled {
        let executable = std::env::current_exe()?;
        let quoted = format!("\"{}\" --background", executable.display());
        let bytes: Vec<u16> = quoted.encode_utf16().chain(Some(0)).collect();
        unsafe {
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                bytes.as_ptr() as *const u8,
                (bytes.len() * 2) as u32,
            )
        }
    } else {
        let result = unsafe { RegDeleteValueW(key, value_name.as_ptr()) };
        if result == 2 { 0 } else { result }
    };
    unsafe {
        RegCloseKey(key);
    }
    if operation_result != 0 {
        bail!("更新开机启动失败（错误 {operation_result}）");
    }
    Ok(())
}

fn window_text(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return String::new();
    }
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let mut buffer = vec![0u16; length.max(0) as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..written.max(0) as usize])
}

fn set_control_text(hwnd: HWND, value: &str) {
    if hwnd.is_null() {
        return;
    }
    let value = wide(value);
    unsafe {
        SetWindowTextW(hwnd, value.as_ptr());
    }
}

fn set_check(hwnd: HWND, checked: bool) {
    unsafe {
        SendMessageW(
            hwnd,
            BM_SETCHECK,
            if checked { BST_CHECKED as usize } else { 0 },
            0,
        );
    }
}

fn is_checked(hwnd: HWND) -> bool {
    unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED }
}

fn copy_wide_fixed<const N: usize>(destination: &mut [u16; N], value: &str) {
    destination.fill(0);
    for (target, source) in destination
        .iter_mut()
        .zip(value.encode_utf16().take(N.saturating_sub(1)))
    {
        *target = source;
    }
}

fn loword(value: usize) -> u16 {
    (value & 0xffff) as u16
}

fn hiword(value: usize) -> u16 {
    ((value >> 16) & 0xffff) as u16
}

fn relative_history_time(timestamp: i64) -> String {
    let timestamp = if timestamp > 10_000_000_000 {
        timestamp / 1_000
    } else {
        timestamp
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(timestamp);
    let seconds = now.saturating_sub(timestamp).max(0);
    match seconds {
        0..=59 => "刚刚".to_owned(),
        60..=3_599 => format!("{} 分钟前", seconds / 60),
        3_600..=86_399 => format!("{} 小时前", seconds / 3_600),
        _ => format!("{} 天前", seconds / 86_400),
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
