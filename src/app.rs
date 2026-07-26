use crate::{
    actions::{post_toast, run_worker},
    clipboard::ClipboardService,
    config::{self, AppConfig, TriggerButton},
    gesture::{
        GestureAction, GestureMatch, Point as GesturePoint, Recognizer, UserGestureTemplate,
    },
    hook::{
        self, HookCommand, UiPoint, UiStrokeBegin, WM_APP_CAPTURE_DONE, WM_APP_OVERLAY_BEGIN,
        WM_APP_OVERLAY_END, WM_APP_OVERLAY_POINT, WM_APP_SHOW_HISTORY, WM_APP_TOAST, WM_APP_TRAY,
    },
    logging,
    resources::{ProcessUsage, UsageSampler},
    storage::{ClipKind, ClipPayload, Storage},
    ui::{
        format::{format_bytes, format_uptime},
        gesture_editor,
        history_popup::{self, *},
        history_preview::PreviewImage,
        history_view::{HistoryView, draw_history_item as draw_history_row},
        settings::{self, Controls, Fonts as SettingsFonts, SettingsPage, *},
        theme::{
            ACCENT_COLOR, apply_child_theme, apply_window_theme, create_ui_font, palette, rgb,
        },
        widgets::{self, ButtonRole},
    },
};
use anyhow::{Result, bail};
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
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT,
        POINT, RECT, WPARAM,
    },
    Graphics::{
        Dwm::DwmFlush,
        Gdi::{
            BeginPaint, COLOR_WINDOW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DT_CENTER,
            DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint,
            FW_NORMAL, FW_SEMIBOLD, FillRect, GetMonitorInfoW, HBRUSH, HFONT, InvalidateRect,
            LineTo, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, MoveToEx, PAINTSTRUCT,
            PS_SOLID, RoundRect, ScreenToClient, SelectObject, SetBkColor, SetBkMode, SetTextColor,
            SetWindowRgn, TRANSPARENT, UpdateWindow,
        },
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
            DRAWITEMSTRUCT, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
            WM_MOUSELEAVE,
        },
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        Input::KeyboardAndMouse::{
            EnableWindow, ReleaseCapture, SetCapture, SetFocus, TME_LEAVE, TRACKMOUSEEVENT,
            TrackMouseEvent, VK_DELETE, VK_ESCAPE, VK_RETURN,
        },
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
            Shell_NotifyIconW, ShellExecuteW,
        },
        WindowsAndMessaging::{
            AppendMenuW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
            DestroyMenu, DestroyWindow, DispatchMessageW, EN_CHANGE, FindWindowW, GA_ROOT,
            GW_OWNER, GWLP_USERDATA, GetAncestor, GetClientRect, GetCursorPos, GetForegroundWindow,
            GetMessageW, GetSystemMetrics, GetWindow, GetWindowLongPtrW, GetWindowRect,
            GetWindowTextLengthW, GetWindowTextW, HMENU, HTTRANSPARENT, HWND_TOPMOST, IDC_ARROW,
            IDI_APPLICATION, IDYES, IsWindowVisible, KillTimer, LB_ADDSTRING, LB_GETCURSEL,
            LB_ITEMFROMPOINT, LB_RESETCONTENT, LB_SETCURSEL, LBN_DBLCLK, LBN_SELCHANGE, LWA_ALPHA,
            LWA_COLORKEY, LoadCursorW, LoadIconW, MB_ICONERROR, MB_ICONINFORMATION,
            MB_ICONQUESTION, MB_OK, MB_YESNO, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG,
            MessageBoxW, PostQuitMessage, RegisterClassExW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
            SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE,
            SWP_NOACTIVATE, SWP_SHOWWINDOW, SendMessageW, SetForegroundWindow,
            SetLayeredWindowAttributes, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW,
            ShowWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
            TrackPopupMenu, TranslateMessage, WA_INACTIVE, WINDOW_EX_STYLE, WINDOW_STYLE,
            WM_ACTIVATE, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLOREDIT,
            WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM, WM_ERASEBKGND,
            WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_PAINT,
            WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_CAPTION, WS_CLIPCHILDREN, WS_EX_LAYERED,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_MINIMIZEBOX,
            WS_OVERLAPPED, WS_POPUP, WS_SYSMENU,
        },
    },
};

const MAIN_CLASS: &str = "Xmouse.Settings";
const HISTORY_CLASS: &str = "Xmouse.History";
const PREVIEW_CLASS: &str = "Xmouse.HistoryPreview";
const OVERLAY_CLASS: &str = "Xmouse.Overlay";
const TOAST_CLASS: &str = "Xmouse.Toast";
const APP_ICON_ID: usize = 1;

const TRAY_ID: u32 = 1;
const IDM_SETTINGS: usize = 2001;
const IDM_HISTORY: usize = 2002;
const IDM_PAUSE: usize = 2003;
const IDM_EXIT: usize = 2004;
const RESOURCE_TIMER_ID: usize = 2;
const WM_APP_HISTORY_PREVIEW_READY: u32 = 0x8008;
static APP_STATE: AtomicPtr<AppState> = AtomicPtr::new(ptr::null_mut());

struct PreviewResponse {
    item_id: i64,
    image: Option<PreviewImage>,
}

struct AppState {
    main_hwnd: HWND,
    history_hwnd: HWND,
    history_search: HWND,
    history_list: HWND,
    history_usage: HWND,
    history_pin: HWND,
    history_copy: HWND,
    history_delete: HWND,
    history_clear: HWND,
    preview_hwnd: HWND,
    preview_image: Option<PreviewImage>,
    preview_sender: Sender<i64>,
    history_hovered_id: Option<i64>,
    history_tracking_leave: bool,
    overlay_hwnd: HWND,
    toast_hwnd: HWND,
    config: Arc<RwLock<AppConfig>>,
    config_path: PathBuf,
    clipboard: ClipboardService,
    storage: Storage,
    capture_sender: Sender<()>,
    controls: Controls,
    overlay_points: Vec<UiPoint>,
    gesture_recognizer: Recognizer,
    gesture_preview: Option<GestureMatch>,
    last_gesture_preview_at: Option<Instant>,
    gesture_training_action: GestureAction,
    gesture_training_points: Vec<GesturePoint>,
    gesture_training_drawing: bool,
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
    dark_mode: bool,
    usage_sampler: UsageSampler,
    usage: ProcessUsage,
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
    let mut load_warning = loaded.as_ref().err().map(|error| format!("{error:#}"));
    let config_value = loaded.unwrap_or_default();
    if config_value.autostart
        && let Err(error) = set_autostart(true)
    {
        load_warning = Some(format!("开机自启修复失败：{error:#}"));
    }
    let dark_mode = config_value.dark_mode;
    let mut gesture_recognizer = Recognizer::new();
    gesture_recognizer.set_user_templates(&config_value.custom_gestures);
    let config = Arc::new(RwLock::new(config_value));
    let root = config::app_data_dir()?;
    logging::init(&root)?;
    let storage = Storage::open(root, config.clone())?;
    let clipboard = ClipboardService::new(storage.clone(), config.clone());
    let (capture_sender, capture_receiver) = mpsc::channel();
    let (preview_sender, preview_receiver) = mpsc::channel();

    register_classes()?;
    initialize_common_controls();
    let font_body = create_ui_font(-16, FW_NORMAL as i32);
    let font_section = create_ui_font(-18, FW_SEMIBOLD as i32);
    let font_title = create_ui_font(-28, FW_SEMIBOLD as i32);
    let colors = palette(dark_mode);
    let brush_page = unsafe { CreateSolidBrush(colors.page) };
    let brush_card = unsafe { CreateSolidBrush(colors.card) };
    let brush_sidebar = unsafe { CreateSolidBrush(colors.sidebar) };

    let state = Box::new(AppState {
        main_hwnd: ptr::null_mut(),
        history_hwnd: ptr::null_mut(),
        history_search: ptr::null_mut(),
        history_list: ptr::null_mut(),
        history_usage: ptr::null_mut(),
        history_pin: ptr::null_mut(),
        history_copy: ptr::null_mut(),
        history_delete: ptr::null_mut(),
        history_clear: ptr::null_mut(),
        preview_hwnd: ptr::null_mut(),
        preview_image: None,
        preview_sender,
        history_hovered_id: None,
        history_tracking_leave: false,
        overlay_hwnd: ptr::null_mut(),
        toast_hwnd: ptr::null_mut(),
        config,
        config_path,
        clipboard: clipboard.clone(),
        storage: storage.clone(),
        capture_sender,
        controls: Controls::default(),
        overlay_points: Vec::with_capacity(512),
        gesture_recognizer,
        gesture_preview: None,
        last_gesture_preview_at: None,
        gesture_training_action: GestureAction::SearchSelection,
        gesture_training_points: Vec::with_capacity(512),
        gesture_training_drawing: false,
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
        dark_mode,
        usage_sampler: UsageSampler::new(),
        usage: ProcessUsage::default(),
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
        620,
        570,
    )?;
    let preview_hwnd = create_top_window(
        PREVIEW_CLASS,
        "",
        WS_POPUP,
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
        0,
        0,
        1,
        1,
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
        360,
        52,
    )?;
    unsafe {
        (*state_pointer).history_hwnd = history_hwnd;
        (*state_pointer).preview_hwnd = preview_hwnd;
        (*state_pointer).overlay_hwnd = overlay_hwnd;
        (*state_pointer).toast_hwnd = toast_hwnd;
        SetLayeredWindowAttributes(overlay_hwnd, rgb(0, 0, 0), 0, LWA_COLORKEY | LWA_ALPHA);
        let toast_region = CreateRoundRectRgn(0, 0, 360, 52, 16, 16);
        SetWindowRgn(toast_hwnd, toast_region, 1);
        apply_window_theme(main_hwnd, dark_mode);
        apply_window_theme(history_hwnd, dark_mode);
    }

    let preview_ui_hwnd = main_hwnd as isize;
    thread::Builder::new()
        .name("xmouse-preview".to_owned())
        .spawn(move || {
            while let Ok(mut item_id) = preview_receiver.recv() {
                while let Ok(newer_id) = preview_receiver.try_recv() {
                    item_id = newer_id;
                }
                let image = match storage.payload(item_id) {
                    Ok(ClipPayload::ImagePng(png)) => PreviewImage::from_png(item_id, &png),
                    _ => None,
                };
                let response = Box::new(PreviewResponse { item_id, image });
                let pointer = Box::into_raw(response);
                if unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        preview_ui_hwnd as HWND,
                        WM_APP_HISTORY_PREVIEW_READY,
                        0,
                        pointer as LPARAM,
                    )
                } == 0
                {
                    unsafe {
                        drop(Box::from_raw(pointer));
                    }
                }
            }
        })?;
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
    let instance_id = std::env::var("XMOUSE_INSTANCE_ID")
        .ok()
        .filter(|value| !value.is_empty());
    let name = wide(&format!(
        "Local\\Xmouse.Singleton.v1{}",
        instance_id
            .as_deref()
            .map(|value| format!(".{value}"))
            .unwrap_or_default()
    ));
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
            PREVIEW_CLASS,
            Some(preview_proc as unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT),
            ptr::null_mut(),
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
    let colors = palette(state.dark_mode);
    let (brush, background) = if parent == state.main_hwnd && origin.x < 190 {
        (state.brush_sidebar, colors.sidebar)
    } else if parent == state.main_hwnd && origin.y < 82 {
        (state.brush_page, colors.page)
    } else if parent == state.main_hwnd {
        (state.brush_card, colors.card)
    } else {
        (state.brush_page, colors.page)
    };
    unsafe {
        SetBkColor(hdc, background);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(
            hdc,
            if control == state.controls.page_subtitle {
                colors.muted
            } else {
                colors.text
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
    if state.history_hwnd.is_null() || unsafe { IsWindowVisible(state.history_hwnd) } == 0 {
        return false;
    }
    let root = unsafe { GetAncestor(message.hwnd, GA_ROOT) };
    if root != state.history_hwnd {
        return false;
    }
    match message.message {
        WM_MOUSEMOVE if message.hwnd == state.history_list => {
            if !state.history_tracking_leave {
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: state.history_list,
                    dwHoverTime: 0,
                };
                unsafe {
                    TrackMouseEvent(&mut tracking);
                }
                state.history_tracking_leave = true;
            }
            update_history_hover(state, message.lParam);
            false
        }
        WM_MOUSELEAVE if message.hwnd == state.history_list => {
            state.history_tracking_leave = false;
            clear_history_hover(state);
            false
        }
        WM_KEYDOWN => match message.wParam as u16 {
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
        },
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
                state.controls = settings::create_controls(
                    hwnd,
                    SettingsFonts {
                        body: state.font_body,
                        section: state.font_section,
                        title: state.font_title,
                    },
                );
                refresh_history_usage(state);
                show_settings_page(state, SettingsPage::General);
                load_config_into_controls(state);
                refresh_gesture_training_ui(state);
                refresh_theme_resources(state);
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
                    IDC_ENABLED | IDC_AUTOSTART | IDC_CAPTURE | IDC_ENCRYPT_CONTENT
                    | IDC_DARK_MODE => {
                        let control = match id {
                            IDC_ENABLED => state.controls.enabled,
                            IDC_AUTOSTART => state.controls.autostart,
                            IDC_CAPTURE => state.controls.capture,
                            IDC_ENCRYPT_CONTENT => state.controls.encrypt_content,
                            IDC_DARK_MODE => state.controls.dark_mode,
                            _ => ptr::null_mut(),
                        };
                        toggle_check(control);
                        if id == IDC_DARK_MODE {
                            state.dark_mode = is_checked(control);
                            refresh_theme_resources(state);
                        }
                    }
                    IDC_TRIGGER_RIGHT => select_trigger(state, TriggerButton::Right),
                    IDC_TRIGGER_X1 => select_trigger(state, TriggerButton::X1),
                    IDC_TRIGGER_X2 => select_trigger(state, TriggerButton::X2),
                    IDC_SAVE => match read_config_from_controls(state) {
                        Ok(config) => {
                            if let Err(error) = config::save(&state.config_path, &config)
                                .and_then(|_| set_autostart(config.autostart))
                            {
                                logging::error("保存设置", &error);
                                post_toast(hwnd as isize, &format!("保存失败：{error:#}"));
                            } else {
                                *state.config.write().expect("config poisoned") = config;
                                refresh_status(state);
                                post_toast(hwnd as isize, "设置已保存");
                            }
                        }
                        Err(error) => post_toast(hwnd as isize, &format!("{error:#}")),
                    },
                    IDC_OPEN_HISTORY => show_history(state, hwnd as isize),
                    IDC_CLEAR_HISTORY => confirm_clear_history(state),
                    IDC_OPEN_DATA_DIR => open_data_directory(state),
                    IDC_NAV_GENERAL => show_settings_page(state, SettingsPage::General),
                    IDC_NAV_HISTORY => show_settings_page(state, SettingsPage::History),
                    IDC_NAV_GESTURES => show_settings_page(state, SettingsPage::Gestures),
                    IDC_NAV_RESOURCES => show_settings_page(state, SettingsPage::Resources),
                    IDC_GESTURE_TOPMOST => {
                        select_gesture_training_action(state, GestureAction::ToggleTopmost)
                    }
                    IDC_GESTURE_CLOSE => {
                        select_gesture_training_action(state, GestureAction::CloseTab)
                    }
                    IDC_GESTURE_SEARCH => {
                        select_gesture_training_action(state, GestureAction::SearchSelection)
                    }
                    IDC_GESTURE_COPY => {
                        select_gesture_training_action(state, GestureAction::CopySelection)
                    }
                    IDC_GESTURE_HISTORY => {
                        select_gesture_training_action(state, GestureAction::OpenHistory)
                    }
                    IDC_GESTURE_CLEAR => clear_current_gesture_samples(state),
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
        WM_APP_CAPTURE_DONE => {
            if let Some(state) = state_mut() {
                refresh_history_usage(state);
            }
            0
        }
        WM_APP_HISTORY_PREVIEW_READY => {
            let pointer = lparam as *mut PreviewResponse;
            if !pointer.is_null() {
                let response = unsafe { *Box::from_raw(pointer) };
                if let Some(state) = state_mut()
                    && state.history_hovered_id == Some(response.item_id)
                {
                    state.preview_image = response.image;
                    show_history_preview(state);
                }
            }
            0
        }
        WM_APP_OVERLAY_BEGIN => {
            if let Some(state) = state_mut() {
                let pointer = lparam as *mut UiStrokeBegin;
                if !pointer.is_null() {
                    let begin = unsafe { *Box::from_raw(pointer) };
                    state.overlay_points.clear();
                    state.overlay_points.extend(begin.points);
                    state.gesture_preview = None;
                    state.last_gesture_preview_at = None;
                    refresh_gesture_preview(state);
                    show_overlay(state);
                }
            }
            0
        }
        WM_APP_OVERLAY_POINT => {
            if let Some(state) = state_mut() {
                let pointer = lparam as *mut UiPoint;
                if !pointer.is_null() {
                    let point = unsafe { *Box::from_raw(pointer) };
                    state.overlay_points.push(point);
                    refresh_gesture_preview(state);
                    unsafe {
                        InvalidateRect(state.overlay_hwnd, ptr::null(), 0);
                    }
                }
            }
            0
        }
        WM_APP_OVERLAY_END => {
            if let Some(state) = state_mut() {
                hide_overlay(state);
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
        WM_LBUTTONDOWN => {
            if let Some(state) = state_mut() {
                let (x, y) = client_point(lparam);
                if state.active_settings_page == SettingsPage::Gestures
                    && gesture_editor::contains(x, y)
                {
                    state.gesture_training_points.clear();
                    state
                        .gesture_training_points
                        .push(GesturePoint::new(x as f32, y as f32));
                    state.gesture_training_drawing = true;
                    unsafe {
                        SetCapture(hwnd);
                        InvalidateRect(hwnd, &gesture_editor::CANVAS_RECT, 0);
                    }
                    return 0;
                }
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_MOUSEMOVE => {
            if let Some(state) = state_mut()
                && state.gesture_training_drawing
            {
                let (x, y) = client_point(lparam);
                append_gesture_training_point(state, x, y);
                unsafe {
                    InvalidateRect(hwnd, &gesture_editor::CANVAS_RECT, 0);
                }
                return 0;
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_LBUTTONUP => {
            if let Some(state) = state_mut()
                && state.gesture_training_drawing
            {
                let (x, y) = client_point(lparam);
                append_gesture_training_point(state, x, y);
                state.gesture_training_drawing = false;
                unsafe {
                    ReleaseCapture();
                }
                finish_gesture_training(state);
                unsafe {
                    InvalidateRect(hwnd, &gesture_editor::CANVAS_RECT, 0);
                }
                return 0;
            }
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_DRAWITEM => {
            let draw = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            match draw.CtlID as i32 {
                IDC_SAVE | IDC_OPEN_HISTORY | IDC_CLEAR_HISTORY | IDC_STATUS | IDC_NAV_GENERAL
                | IDC_NAV_HISTORY | IDC_NAV_GESTURES | IDC_NAV_RESOURCES | IDC_OPEN_DATA_DIR
                | IDC_GESTURE_CLEAR => {
                    draw_button(draw);
                    1
                }
                IDC_ENABLED | IDC_AUTOSTART | IDC_CAPTURE | IDC_ENCRYPT_CONTENT | IDC_DARK_MODE => {
                    draw_toggle(draw);
                    1
                }
                IDC_TRIGGER_RIGHT | IDC_TRIGGER_X1 | IDC_TRIGGER_X2 | IDC_GESTURE_TOPMOST
                | IDC_GESTURE_CLOSE | IDC_GESTURE_SEARCH | IDC_GESTURE_COPY
                | IDC_GESTURE_HISTORY => {
                    draw_choice(draw);
                    1
                }
                _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
            }
        }
        WM_CTLCOLORSTATIC => color_static_control(hwnd, lparam as HWND, wparam as *mut c_void),
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let hdc = wparam as *mut c_void;
            let Some(state) = state_mut() else {
                return (COLOR_WINDOW + 1) as LRESULT;
            };
            let colors = palette(state.dark_mode);
            unsafe {
                SetBkColor(hdc, colors.card);
                SetTextColor(hdc, colors.text);
            }
            state.brush_card as LRESULT
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint_main_window(hwnd);
            0
        }
        WM_TIMER if wparam == RESOURCE_TIMER_ID => {
            if let Some(state) = state_mut()
                && state.active_settings_page == SettingsPage::Resources
            {
                refresh_resource_usage(state);
            }
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
                let controls = history_popup::create_controls(
                    hwnd,
                    SettingsFonts {
                        body: state.font_body,
                        section: state.font_section,
                        title: state.font_title,
                    },
                    state.dark_mode,
                );
                state.history_search = controls.search;
                state.history_list = controls.list;
                state.history_usage = controls.usage;
                state.history_pin = controls.pin;
                state.history_copy = controls.copy;
                state.history_delete = controls.delete;
                state.history_clear = controls.clear;
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
                    IDC_HISTORY_LIST if notification == LBN_SELCHANGE as u16 => {
                        refresh_pin_button(state)
                    }
                    IDC_HISTORY_PIN => toggle_selected_history_pin(state),
                    IDC_HISTORY_COPY => copy_selected_history(state),
                    IDC_HISTORY_DELETE => delete_selected_history(state),
                    IDC_HISTORY_CLEAR => confirm_clear_history(state),
                    _ => {}
                }
            }
            0
        }
        WM_ACTIVATE if loword(wparam) as u32 == WA_INACTIVE => {
            let activated = lparam as HWND;
            if !is_owned_window(activated, hwnd)
                && let Some(state) = state_mut()
            {
                dismiss_history(state);
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
                IDC_HISTORY_PIN | IDC_HISTORY_COPY | IDC_HISTORY_DELETE | IDC_HISTORY_CLEAR
            ) {
                draw_button(draw);
                1
            } else {
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_CTLCOLORSTATIC => color_static_control(hwnd, lparam as HWND, wparam as *mut c_void),
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => {
            let hdc = wparam as *mut c_void;
            let Some(state) = state_mut() else {
                return (COLOR_WINDOW + 1) as LRESULT;
            };
            let colors = palette(state.dark_mode);
            unsafe {
                SetBkColor(hdc, colors.card);
                SetTextColor(hdc, colors.text);
            }
            state.brush_card as LRESULT
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

fn paint_gesture_prediction(hdc: *mut c_void, state: &AppState) {
    let Some(anchor) = state.overlay_points.last() else {
        return;
    };
    let monitor = unsafe {
        MonitorFromPoint(
            POINT {
                x: anchor.x,
                y: anchor.y,
            },
            MONITOR_DEFAULTTONEAREST,
        )
    };
    let mut monitor_info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(monitor, &mut monitor_info);
    }
    let virtual_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let virtual_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = 340;
    let height = 48;
    let left = (monitor_info.rcWork.left + monitor_info.rcWork.right - width) / 2 - virtual_x;
    let top = monitor_info.rcWork.bottom - height - 24 - virtual_y;
    let right = left + width;
    let bottom = top + height;
    let border = if state.gesture_preview.is_some() {
        ACCENT_COLOR
    } else {
        rgb(85, 96, 112)
    };
    let brush = unsafe { CreateSolidBrush(rgb(24, 29, 39)) };
    let pen = unsafe { CreatePen(PS_SOLID, 1, border) };
    let previous_brush = unsafe { SelectObject(hdc, brush) };
    let previous_pen = unsafe { SelectObject(hdc, pen) };
    unsafe {
        RoundRect(hdc, left, top, right, bottom, 18, 18);
        SelectObject(hdc, previous_brush);
        SelectObject(hdc, previous_pen);
        DeleteObject(brush);
        DeleteObject(pen);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, rgb(245, 247, 250));
    }
    let label = state
        .gesture_preview
        .map(|matched| format!("预计：{}", matched.action.preview_label()))
        .unwrap_or_else(|| "正在识别手势…".to_owned());
    let label = wide(&label);
    let mut text_rect = RECT {
        left: left + 14,
        top,
        right: right - 14,
        bottom,
    };
    let previous_font = unsafe { SelectObject(hdc, state.font_body) };
    unsafe {
        DrawTextW(
            hdc,
            label.as_ptr(),
            -1,
            &mut text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        SelectObject(hdc, previous_font);
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
            if let Some(state) = state_mut() {
                if state.overlay_points.len() >= 2 {
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
                paint_gesture_prediction(hdc, state);
            }
            unsafe {
                EndPaint(hwnd, &paint);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

unsafe extern "system" fn preview_proc(
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
            let mut client = RECT::default();
            unsafe {
                GetClientRect(hwnd, &mut client);
            }
            if let Some(state) = state_mut()
                && let Some(preview) = state.preview_image.as_ref()
            {
                preview.draw(hdc, client, palette(state.dark_mode));
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
    for &control in &state.controls.gestures_page {
        unsafe {
            ShowWindow(
                control,
                if page == SettingsPage::Gestures {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
    }
    for &control in &state.controls.resources_page {
        unsafe {
            ShowWindow(
                control,
                if page == SettingsPage::Resources {
                    SW_SHOW
                } else {
                    SW_HIDE
                },
            );
        }
    }
    let (title, subtitle) = match page {
        SettingsPage::General => ("常规", "手势与启动"),
        SettingsPage::History => ("剪贴板历史", "记录与管理"),
        SettingsPage::Gestures => ("个性化手势", "让 Xmouse 适应你的轨迹"),
        SettingsPage::Resources => ("资源占用", "Xmouse 当前进程"),
    };
    set_control_text(state.controls.page_title, title);
    set_control_text(state.controls.page_subtitle, subtitle);
    unsafe {
        ShowWindow(
            state.controls.status,
            if page == SettingsPage::General {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        InvalidateRect(state.controls.nav_general, ptr::null(), 1);
        InvalidateRect(state.controls.nav_history, ptr::null(), 1);
        InvalidateRect(state.controls.nav_gestures, ptr::null(), 1);
        InvalidateRect(state.controls.nav_resources, ptr::null(), 1);
        InvalidateRect(state.main_hwnd, ptr::null(), 1);
        if page == SettingsPage::Resources {
            refresh_resource_usage(state);
            SetTimer(state.main_hwnd, RESOURCE_TIMER_ID, 1_000, None);
        } else {
            KillTimer(state.main_hwnd, RESOURCE_TIMER_ID);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn load_config_into_controls(state: &mut AppState) {
    let config = state.config.read().expect("config poisoned").clone();
    state.dark_mode = config.dark_mode;
    set_check(state.controls.enabled, config.enabled);
    set_check(state.controls.dark_mode, config.dark_mode);
    set_check(state.controls.autostart, config.autostart);
    set_check(state.controls.capture, config.history.capture);
    set_check(
        state.controls.encrypt_content,
        config.history.encrypt_content,
    );
    select_trigger(state, config.trigger);
    refresh_gesture_training_ui(state);
    refresh_status(state);
    refresh_history_usage(state);
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

fn toggle_check(control: HWND) {
    if control.is_null() {
        return;
    }
    set_check(control, !is_checked(control));
    unsafe {
        InvalidateRect(control, ptr::null(), 1);
    }
}

fn select_trigger(state: &AppState, trigger: TriggerButton) {
    for (control, button) in [
        (state.controls.trigger_right, TriggerButton::Right),
        (state.controls.trigger_x1, TriggerButton::X1),
        (state.controls.trigger_x2, TriggerButton::X2),
    ] {
        set_check(control, button == trigger);
        unsafe {
            InvalidateRect(control, ptr::null(), 1);
        }
    }
}

fn gesture_action_control(state: &AppState, action: GestureAction) -> HWND {
    match action {
        GestureAction::ToggleTopmost => state.controls.gesture_topmost,
        GestureAction::CloseTab => state.controls.gesture_close,
        GestureAction::SearchSelection => state.controls.gesture_search,
        GestureAction::CopySelection => state.controls.gesture_copy,
        GestureAction::OpenHistory => state.controls.gesture_history,
    }
}

fn select_gesture_training_action(state: &mut AppState, action: GestureAction) {
    state.gesture_training_action = action;
    state.gesture_training_points.clear();
    refresh_gesture_training_ui(state);
    unsafe {
        InvalidateRect(state.main_hwnd, &gesture_editor::CANVAS_RECT, 0);
    }
}

fn refresh_gesture_training_ui(state: &AppState) {
    for action in GestureAction::ALL {
        let control = gesture_action_control(state, action);
        set_check(control, action == state.gesture_training_action);
        if !control.is_null() {
            unsafe {
                InvalidateRect(control, ptr::null(), 1);
            }
        }
    }
    let count = state
        .config
        .read()
        .expect("config poisoned")
        .custom_gestures
        .iter()
        .filter(|sample| sample.action == state.gesture_training_action)
        .count();
    set_control_text(
        state.controls.gesture_status,
        &format!(
            "已选择 {} · 已学习 {count}/3 份样本",
            state.gesture_training_action.short_label()
        ),
    );
    unsafe {
        EnableWindow(state.controls.gesture_clear, i32::from(count > 0));
    }
}

fn append_gesture_training_point(state: &mut AppState, x: i32, y: i32) {
    if state.gesture_training_points.len() >= 512 {
        return;
    }
    let x = x.clamp(
        gesture_editor::CANVAS_RECT.left + 2,
        gesture_editor::CANVAS_RECT.right - 2,
    );
    let y = y.clamp(
        gesture_editor::CANVAS_RECT.top + 2,
        gesture_editor::CANVAS_RECT.bottom - 2,
    );
    let next = GesturePoint::new(x as f32, y as f32);
    let should_add = state
        .gesture_training_points
        .last()
        .map(|previous| {
            let dx = previous.x - next.x;
            let dy = previous.y - next.y;
            dx * dx + dy * dy >= 2.25
        })
        .unwrap_or(true);
    if should_add {
        state.gesture_training_points.push(next);
    }
}

fn finish_gesture_training(state: &mut AppState) {
    let length: f32 = state
        .gesture_training_points
        .windows(2)
        .map(|pair| {
            let dx = pair[1].x - pair[0].x;
            let dy = pair[1].y - pair[0].y;
            (dx * dx + dy * dy).sqrt()
        })
        .sum();
    if length < 60.0 {
        state.gesture_training_points.clear();
        set_control_text(state.controls.gesture_status, "轨迹太短，请重新绘制");
        post_toast(state.main_hwnd as isize, "轨迹太短，未保存");
        return;
    }
    let Some(sample) = UserGestureTemplate::from_stroke(
        state.gesture_training_action,
        &state.gesture_training_points,
    ) else {
        state.gesture_training_points.clear();
        set_control_text(state.controls.gesture_status, "轨迹无效，请重新绘制");
        return;
    };

    let mut next = state.config.read().expect("config poisoned").clone();
    while next
        .custom_gestures
        .iter()
        .filter(|stored| stored.action == sample.action)
        .count()
        >= 3
    {
        if let Some(index) = next
            .custom_gestures
            .iter()
            .position(|stored| stored.action == sample.action)
        {
            next.custom_gestures.remove(index);
        }
    }
    next.custom_gestures.push(sample);
    if let Err(error) = save_gesture_config(state, next) {
        logging::error("保存个性化手势", &error);
        post_toast(
            state.main_hwnd as isize,
            &format!("手势保存失败：{error:#}"),
        );
        return;
    }
    refresh_gesture_training_ui(state);
    post_toast(
        state.main_hwnd as isize,
        &format!("已学习 {}", state.gesture_training_action.short_label()),
    );
}

fn clear_current_gesture_samples(state: &mut AppState) {
    let action = state.gesture_training_action;
    let count = state
        .config
        .read()
        .expect("config poisoned")
        .custom_gestures
        .iter()
        .filter(|sample| sample.action == action)
        .count();
    if count == 0 {
        post_toast(state.main_hwnd as isize, "当前动作还没有个性化样本");
        return;
    }
    let prompt = wide(&format!(
        "确定清除 {} 的 {count} 份个性化样本吗？\n清除后仍会使用内置模板。",
        action.short_label()
    ));
    let title = wide("清除个性化手势");
    let answer = unsafe {
        MessageBoxW(
            state.main_hwnd,
            prompt.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        )
    };
    if answer != IDYES {
        return;
    }
    let mut next = state.config.read().expect("config poisoned").clone();
    next.custom_gestures
        .retain(|sample| sample.action != action);
    if let Err(error) = save_gesture_config(state, next) {
        logging::error("清除个性化手势", &error);
        post_toast(state.main_hwnd as isize, &format!("清除失败：{error:#}"));
        return;
    }
    state.gesture_training_points.clear();
    refresh_gesture_training_ui(state);
    unsafe {
        InvalidateRect(state.main_hwnd, &gesture_editor::CANVAS_RECT, 0);
    }
    post_toast(state.main_hwnd as isize, "已恢复使用内置模板");
}

fn save_gesture_config(state: &mut AppState, next: AppConfig) -> Result<()> {
    config::save(&state.config_path, &next)?;
    state
        .gesture_recognizer
        .set_user_templates(&next.custom_gestures);
    *state.config.write().expect("config poisoned") = next;
    Ok(())
}

fn refresh_resource_usage(state: &mut AppState) {
    state.usage = state.usage_sampler.sample();
    set_control_text(
        state.controls.resource_cpu,
        &format!("{:.3}%", state.usage.cpu_percent),
    );
    set_control_text(
        state.controls.resource_private,
        &format!("{:.2} MiB", state.usage.private_bytes as f64 / 1_048_576.0),
    );
    set_control_text(
        state.controls.resource_working_set,
        &format!(
            "{:.2} MiB",
            state.usage.working_set_bytes as f64 / 1_048_576.0
        ),
    );
    set_control_text(state.controls.resource_gpu, "0%");
    set_control_text(
        state.controls.resource_details,
        &format!(
            "PID {}    句柄 {}    运行 {}",
            std::process::id(),
            state.usage.handle_count,
            format_uptime(state.usage.uptime)
        ),
    );
}

fn refresh_theme_resources(state: &mut AppState) {
    let colors = palette(state.dark_mode);
    unsafe {
        DeleteObject(state.brush_page);
        DeleteObject(state.brush_card);
        DeleteObject(state.brush_sidebar);
    }
    state.brush_page = unsafe { CreateSolidBrush(colors.page) };
    state.brush_card = unsafe { CreateSolidBrush(colors.card) };
    state.brush_sidebar = unsafe { CreateSolidBrush(colors.sidebar) };
    apply_window_theme(state.main_hwnd, state.dark_mode);
    apply_window_theme(state.history_hwnd, state.dark_mode);
    apply_child_theme(state.history_search, state.dark_mode);
    apply_child_theme(state.history_list, state.dark_mode);
    unsafe {
        InvalidateRect(state.main_hwnd, ptr::null(), 1);
        InvalidateRect(state.history_hwnd, ptr::null(), 1);
        InvalidateRect(state.preview_hwnd, ptr::null(), 1);
    }
    for control in state
        .controls
        .general_page
        .iter()
        .chain(state.controls.history_page.iter())
        .chain(state.controls.gestures_page.iter())
        .chain(state.controls.resources_page.iter())
        .copied()
        .chain([
            state.controls.page_title,
            state.controls.page_subtitle,
            state.controls.status,
            state.controls.nav_general,
            state.controls.nav_history,
            state.controls.nav_gestures,
            state.controls.nav_resources,
            state.controls.save,
            state.history_search,
            state.history_list,
            state.history_usage,
        ])
    {
        if !control.is_null() {
            unsafe {
                InvalidateRect(control, ptr::null(), 1);
            }
        }
    }
}

fn read_config_from_controls(state: &AppState) -> Result<AppConfig> {
    let current = state.config.read().expect("config poisoned").clone();
    let trigger = if is_checked(state.controls.trigger_x1) {
        TriggerButton::X1
    } else if is_checked(state.controls.trigger_x2) {
        TriggerButton::X2
    } else {
        TriggerButton::Right
    };
    let config = AppConfig {
        schema_version: 1,
        enabled: is_checked(state.controls.enabled),
        dark_mode: is_checked(state.controls.dark_mode),
        trigger,
        activation_delay_ms: current.activation_delay_ms,
        activation_distance_dip: current.activation_distance_dip,
        minimum_stroke_length_dip: current.minimum_stroke_length_dip,
        recognition_threshold: current.recognition_threshold,
        show_trail: current.show_trail,
        search_url_template: current.search_url_template.clone(),
        autostart: is_checked(state.controls.autostart),
        custom_gestures: current.custom_gestures.clone(),
        history: crate::config::HistoryConfig {
            capture: is_checked(state.controls.capture),
            encrypt_content: is_checked(state.controls.encrypt_content),
            ..current.history
        },
    };
    config.validate()?;
    Ok(config)
}

fn show_history(state: &mut AppState, origin: isize) {
    state.history_origin = origin;
    clear_history_hover(state);
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
    let width = 620;
    let height = 570;
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
    dismiss_history(state);
    let origin = state.history_origin as HWND;
    if !origin.is_null() {
        unsafe {
            SetForegroundWindow(origin);
        }
    }
}

fn dismiss_history(state: &mut AppState) {
    clear_history_hover(state);
    unsafe {
        ShowWindow(state.history_hwnd, SW_HIDE);
    }
}

fn is_owned_window(mut window: HWND, owner: HWND) -> bool {
    while !window.is_null() {
        window = unsafe { GetWindow(window, GW_OWNER) };
        if window == owner {
            return true;
        }
    }
    false
}

fn rebuild_history(state: &mut AppState, query: &str) {
    rebuild_history_with_selection(state, query, None);
}

fn rebuild_history_with_selection(state: &mut AppState, query: &str, selected_id: Option<i64>) {
    clear_history_hover(state);
    match state.storage.list(query) {
        Ok(items) => {
            state.history_items = items.into_iter().map(HistoryView::new).collect();
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
                let selected_index = selected_id
                    .and_then(|id| {
                        state
                            .history_items
                            .iter()
                            .position(|view| view.item.id == id)
                    })
                    .unwrap_or(0);
                unsafe {
                    SendMessageW(state.history_list, LB_SETCURSEL, selected_index, 0);
                }
            }
            refresh_pin_button(state);
            refresh_history_usage(state);
        }
        Err(error) => post_toast(
            state.main_hwnd as isize,
            &format!("读取历史失败：{error:#}"),
        ),
    }
}

fn update_history_hover(state: &mut AppState, position: LPARAM) {
    let result =
        unsafe { SendMessageW(state.history_list, LB_ITEMFROMPOINT, 0, position) } as usize;
    let hovered_id = if hiword(result) == 0 {
        state
            .history_items
            .get(loword(result) as usize)
            .filter(|view| view.item.kind == ClipKind::Image)
            .map(|view| view.item.id)
    } else {
        None
    };
    if hovered_id == state.history_hovered_id {
        return;
    }
    state.history_hovered_id = hovered_id;
    state.preview_image = None;
    unsafe {
        ShowWindow(state.preview_hwnd, SW_HIDE);
    }
    if let Some(item_id) = hovered_id {
        let _ = state.preview_sender.send(item_id);
    }
}

fn clear_history_hover(state: &mut AppState) {
    state.history_hovered_id = None;
    state.preview_image = None;
    state.history_tracking_leave = false;
    unsafe {
        ShowWindow(state.preview_hwnd, SW_HIDE);
    }
}

fn show_history_preview(state: &mut AppState) {
    let Some(preview) = state.preview_image.as_ref() else {
        unsafe {
            ShowWindow(state.preview_hwnd, SW_HIDE);
        }
        return;
    };
    if state.history_hovered_id != Some(preview.item_id) {
        return;
    }
    let (width, height) = preview.window_size();
    let mut history_rect = RECT::default();
    let mut cursor = POINT::default();
    unsafe {
        GetWindowRect(state.history_hwnd, &mut history_rect);
        GetCursorPos(&mut cursor);
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    let mut monitor_info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        GetMonitorInfoW(monitor, &mut monitor_info);
    }
    let right_x = history_rect.right + 12;
    let x = if right_x + width <= monitor_info.rcWork.right {
        right_x
    } else {
        (history_rect.left - width - 12).max(monitor_info.rcWork.left)
    };
    let y = (cursor.y - height / 2)
        .max(monitor_info.rcWork.top)
        .min(monitor_info.rcWork.bottom - height);
    let region = unsafe { CreateRoundRectRgn(0, 0, width + 1, height + 1, 18, 18) };
    unsafe {
        SetWindowRgn(state.preview_hwnd, region, 0);
        SetWindowPos(
            state.preview_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        InvalidateRect(state.preview_hwnd, ptr::null(), 1);
        ShowWindow(state.preview_hwnd, SW_SHOWNOACTIVATE);
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

fn toggle_selected_history_pin(state: &mut AppState) {
    let Some(index) = selected_history_index(state) else {
        return;
    };
    let item = &state.history_items[index].item;
    let id = item.id;
    let pinned = !item.pinned;
    match state.storage.set_pinned(id, pinned) {
        Ok(()) => {
            let query = window_text(state.history_search);
            rebuild_history_with_selection(state, &query, Some(id));
            post_toast(
                state.main_hwnd as isize,
                if pinned {
                    "已置顶"
                } else {
                    "已取消置顶"
                },
            );
        }
        Err(error) => post_toast(
            state.main_hwnd as isize,
            &format!("更新置顶状态失败：{error:#}"),
        ),
    }
}

fn refresh_pin_button(state: &AppState) {
    let selected = selected_history_index(state);
    let pinned = selected
        .and_then(|index| state.history_items.get(index))
        .is_some_and(|view| view.item.pinned);
    set_control_text(state.history_pin, if pinned { "取消置顶" } else { "置顶" });
    let has_selection = selected.is_some();
    let has_items = state
        .storage
        .stats()
        .is_ok_and(|stats| stats.item_count > 0);
    unsafe {
        EnableWindow(state.history_pin, has_selection as i32);
        EnableWindow(state.history_copy, has_selection as i32);
        EnableWindow(state.history_delete, has_selection as i32);
        EnableWindow(state.history_clear, has_items as i32);
        InvalidateRect(state.history_pin, ptr::null(), 1);
    }
}

fn delete_selected_history(state: &mut AppState) {
    let Some(index) = selected_history_index(state) else {
        return;
    };
    let item = &state.history_items[index].item;
    if item.pinned {
        let text = wide("置顶记录受保护，请先取消置顶再删除。");
        let title = wide("Xmouse");
        unsafe {
            MessageBoxW(
                history_dialog_owner(state),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        return;
    }
    let text = wide("确定删除这条剪贴板记录吗？");
    let title = wide("Xmouse");
    if unsafe {
        MessageBoxW(
            history_dialog_owner(state),
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONQUESTION,
        )
    } != IDYES
    {
        return;
    }
    let id = item.id;
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
    let pinned_count = state
        .storage
        .stats()
        .map(|stats| stats.pinned_count)
        .unwrap_or(0);
    if pinned_count > 0 {
        let text = wide(&format!(
            "历史中有 {pinned_count} 条置顶记录，请先取消置顶再清空。"
        ));
        let title = wide("Xmouse");
        unsafe {
            MessageBoxW(
                history_dialog_owner(state),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION,
            );
        }
        return;
    }
    let text = wide("确定删除全部剪贴板历史吗？此操作无法撤销。");
    let title = wide("Xmouse");
    let answer = unsafe {
        MessageBoxW(
            history_dialog_owner(state),
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
                refresh_pin_button(state);
                refresh_history_usage(state);
                post_toast(state.main_hwnd as isize, "剪贴板历史已清空");
            }
            Err(error) => post_toast(state.main_hwnd as isize, &format!("清空失败：{error:#}")),
        }
    }
}

fn history_dialog_owner(state: &AppState) -> HWND {
    if unsafe { IsWindowVisible(state.history_hwnd) } != 0 {
        state.history_hwnd
    } else {
        state.main_hwnd
    }
}

fn refresh_history_usage(state: &AppState) {
    let Ok(stats) = state.storage.stats() else {
        return;
    };
    let bytes = if stats.disk_bytes > 0 {
        stats.disk_bytes
    } else {
        stats.content_bytes
    };
    let text = if stats.pinned_count > 0 {
        format!(
            "{} 条 · {} 置顶 · {}",
            stats.item_count,
            stats.pinned_count,
            format_bytes(bytes)
        )
    } else {
        format!("{} 条 · {}", stats.item_count, format_bytes(bytes))
    };
    set_control_text(state.controls.history_usage, &text);
    set_control_text(state.history_usage, &text);
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
    let colors = palette(state.dark_mode);
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
        FillRect(hdc, &content, state.brush_page);
    }
    let separator = unsafe { CreatePen(PS_SOLID, 1, colors.border) };
    let old_separator = unsafe { SelectObject(hdc, separator) };
    unsafe {
        MoveToEx(hdc, 189, 0, ptr::null_mut());
        LineTo(hdc, 189, client.bottom);
        MoveToEx(hdc, 190, 82, ptr::null_mut());
        LineTo(hdc, client.right, 82);
    }
    unsafe {
        SelectObject(hdc, old_separator);
        DeleteObject(separator);
    }
    let cards: &[(i32, i32, i32, i32, i32)] = match state.active_settings_page {
        SettingsPage::General => &[
            (214, 100, 858, 246, 18),
            (214, 262, 858, 372, 18),
            (214, 394, 858, 548, 18),
        ],
        SettingsPage::History => &[(214, 100, 858, 218, 18), (214, 244, 858, 410, 18)],
        SettingsPage::Gestures => &[(214, 100, 858, 230, 18), (214, 244, 858, 620, 18)],
        SettingsPage::Resources => &[
            (214, 104, 522, 244, 18),
            (536, 104, 858, 244, 18),
            (214, 266, 522, 406, 18),
            (536, 266, 858, 406, 18),
            (214, 436, 858, 558, 18),
        ],
    };
    for &(left, top, right, bottom, radius) in cards {
        widgets::rounded_panel(hdc, left, top, right, bottom, radius, colors);
    }

    if state.active_settings_page == SettingsPage::Gestures {
        let sample_count = state
            .config
            .read()
            .expect("config poisoned")
            .custom_gestures
            .iter()
            .filter(|sample| sample.action == state.gesture_training_action)
            .count();
        gesture_editor::draw(
            hdc,
            colors,
            state.font_body,
            state.gesture_training_action,
            sample_count,
            &state.gesture_training_points,
            state.gesture_training_drawing,
        );
    }

    settings::draw_sidebar_identity(
        hdc,
        colors,
        SettingsFonts {
            body: state.font_body,
            section: state.font_section,
            title: state.font_title,
        },
    );
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
    let dark = state_mut().map(|state| state.dark_mode).unwrap_or(false);
    let colors = palette(dark);
    let page = unsafe { CreateSolidBrush(colors.page) };
    unsafe {
        FillRect(hdc, &client, page);
        DeleteObject(page);
    }
    widgets::rounded_panel(hdc, 16, 88, 604, 144, 18, colors);
    widgets::rounded_panel(hdc, 23, 97, 597, 138, 30, colors);
    widgets::rounded_panel(hdc, 16, 142, 604, 484, 18, colors);
    widgets::rounded_panel(hdc, 23, 149, 597, 477, 24, colors);
    unsafe {
        EndPaint(hwnd, &paint);
    }
}

fn draw_button(draw: &DRAWITEMSTRUCT) {
    let Some(state) = state_mut() else {
        return;
    };
    let id = draw.CtlID as i32;
    let colors = palette(state.dark_mode);
    let enabled = state.config.read().expect("config poisoned").enabled;
    let nav_active = matches!(
        (id, state.active_settings_page),
        (IDC_NAV_GENERAL, SettingsPage::General)
            | (IDC_NAV_HISTORY, SettingsPage::History)
            | (IDC_NAV_GESTURES, SettingsPage::Gestures)
            | (IDC_NAV_RESOURCES, SettingsPage::Resources)
    );
    let role = if id == IDC_STATUS {
        ButtonRole::Status { enabled }
    } else if matches!(
        id,
        IDC_NAV_GENERAL | IDC_NAV_HISTORY | IDC_NAV_GESTURES | IDC_NAV_RESOURCES
    ) {
        ButtonRole::Navigation { active: nav_active }
    } else if matches!(id, IDC_SAVE | IDC_HISTORY_COPY) {
        ButtonRole::Primary
    } else if matches!(
        id,
        IDC_CLEAR_HISTORY | IDC_HISTORY_DELETE | IDC_HISTORY_CLEAR | IDC_GESTURE_CLEAR
    ) {
        ButtonRole::Danger
    } else {
        ButtonRole::Secondary
    };
    let corner_color = if matches!(
        id,
        IDC_NAV_GENERAL | IDC_NAV_HISTORY | IDC_NAV_GESTURES | IDC_NAV_RESOURCES
    ) {
        colors.sidebar
    } else if matches!(
        id,
        IDC_SAVE
            | IDC_STATUS
            | IDC_HISTORY_PIN
            | IDC_HISTORY_COPY
            | IDC_HISTORY_DELETE
            | IDC_HISTORY_CLEAR
    ) {
        colors.page
    } else {
        colors.card
    };
    widgets::draw_button(draw, colors, state.font_body, role, corner_color);
}

fn draw_toggle(draw: &DRAWITEMSTRUCT) {
    let Some(state) = state_mut() else {
        return;
    };
    widgets::draw_toggle(
        draw,
        palette(state.dark_mode),
        state.font_body,
        is_checked(draw.hwndItem),
    );
}

fn draw_choice(draw: &DRAWITEMSTRUCT) {
    let Some(state) = state_mut() else {
        return;
    };
    widgets::draw_choice(
        draw,
        palette(state.dark_mode),
        state.font_body,
        is_checked(draw.hwndItem),
    );
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
    draw_history_row(draw, view, palette(state.dark_mode), state.font_body);
}

fn refresh_gesture_preview(state: &mut AppState) {
    if state.overlay_points.len() < 6 {
        return;
    }
    let now = Instant::now();
    if state
        .last_gesture_preview_at
        .is_some_and(|previous| now.duration_since(previous) < Duration::from_millis(70))
    {
        return;
    }
    let path_length: f32 = state
        .overlay_points
        .windows(2)
        .map(|pair| {
            let dx = (pair[1].x - pair[0].x) as f32;
            let dy = (pair[1].y - pair[0].y) as f32;
            (dx * dx + dy * dy).sqrt()
        })
        .sum();
    if path_length < 24.0 {
        return;
    }
    let points: Vec<GesturePoint> = state
        .overlay_points
        .iter()
        .map(|point| GesturePoint::new(point.x as f32, point.y as f32))
        .collect();
    let threshold = state
        .config
        .read()
        .expect("config poisoned")
        .recognition_threshold;
    state.gesture_preview = state.gesture_recognizer.recognize(&points, threshold);
    state.last_gesture_preview_at = Some(now);
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
        // A hidden layered window can retain its last compositor surface. Keep it fully
        // transparent while making it visible and repainting the new stroke, then reveal it
        // only after DWM has consumed the cleared surface.
        SetLayeredWindowAttributes(
            state.overlay_hwnd,
            rgb(0, 0, 0),
            0,
            LWA_COLORKEY | LWA_ALPHA,
        );
        SetWindowPos(
            state.overlay_hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        InvalidateRect(state.overlay_hwnd, ptr::null(), 1);
        UpdateWindow(state.overlay_hwnd);
        DwmFlush();
        SetLayeredWindowAttributes(
            state.overlay_hwnd,
            rgb(0, 0, 0),
            255,
            LWA_COLORKEY | LWA_ALPHA,
        );
    }
}

fn hide_overlay(state: &mut AppState) {
    let was_visible = unsafe { IsWindowVisible(state.overlay_hwnd) } != 0;
    unsafe {
        SetLayeredWindowAttributes(
            state.overlay_hwnd,
            rgb(0, 0, 0),
            0,
            LWA_COLORKEY | LWA_ALPHA,
        );
    }
    state.overlay_points.clear();
    state.gesture_preview = None;
    state.last_gesture_preview_at = None;
    unsafe {
        InvalidateRect(state.overlay_hwnd, ptr::null(), 1);
        if was_visible {
            UpdateWindow(state.overlay_hwnd);
            DwmFlush();
        }
        ShowWindow(state.overlay_hwnd, SW_HIDE);
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
    let width = 360;
    let height = 52;
    let x = info.rcWork.left + (info.rcWork.right - info.rcWork.left - width) / 2;
    let y = info.rcWork.bottom - height - 28;
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
    if !hwnd.is_null() {
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, isize::from(checked));
        }
    }
}

fn is_checked(hwnd: HWND) -> bool {
    !hwnd.is_null() && unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) != 0 }
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

fn client_point(value: LPARAM) -> (i32, i32) {
    let packed = value as u32;
    (
        (packed as u16 as i16) as i32,
        ((packed >> 16) as u16 as i16) as i32,
    )
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
