use crate::{
    clipboard::process_name,
    config::{AppConfig, GestureGuardConfig, TriggerButton},
    gesture::Point as GesturePoint,
    gesture_lifecycle::{RecoveryReason, SessionWatchdog, WATCHDOG_INTERVAL_MS},
};
use anyhow::{Result, bail};
use std::{
    ffi::c_void,
    ptr,
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicU32, AtomicUsize, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{GetCurrentProcessId, GetCurrentThreadId},
    },
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
            MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
        },
        WindowsAndMessaging::{
            CallNextHookEx, EVENT_SYSTEM_FOREGROUND, GA_ROOT, GWL_STYLE, GetAncestor,
            GetClassNameW, GetForegroundWindow, GetMessageW, GetShellWindow, GetWindowLongPtrW,
            GetWindowRect, GetWindowThreadProcessId, HHOOK, IsIconic, IsZoomed, KillTimer,
            LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PostMessageW, PostThreadMessageW, SetTimer,
            SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT,
            WINEVENT_SKIPOWNPROCESS, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER,
            WM_XBUTTONDOWN, WM_XBUTTONUP, WS_CAPTION, WindowFromPoint,
        },
    },
};

pub const WM_APP_OVERLAY_BEGIN: u32 = 0x8001;
pub const WM_APP_OVERLAY_POINT: u32 = 0x8002;
pub const WM_APP_OVERLAY_END: u32 = 0x8003;
pub const WM_APP_SHOW_HISTORY: u32 = 0x8004;
pub const WM_APP_TOAST: u32 = 0x8005;
pub const WM_APP_TRAY: u32 = 0x8006;
pub const WM_APP_CAPTURE_DONE: u32 = 0x8007;
pub const WM_APP_STATS_UPDATED: u32 = 0x8009;
const WM_APP_CANCEL_SESSION: u32 = 0x800A;

pub const INJECTED_EVENT_TOKEN: usize = 0x4743_4C49_505F_0001;
const XBUTTON1_VALUE: u16 = 0x0001;
const XBUTTON2_VALUE: u16 = 0x0002;
const HOOK_REFRESH_INTERVAL_MS: u32 = 30_000;

#[derive(Debug)]
pub struct StrokeRequest {
    pub points: Vec<GesturePoint>,
    pub target_hwnd: isize,
}

#[derive(Debug)]
pub enum HookCommand {
    Stroke(StrokeRequest),
    Replay {
        button: TriggerButton,
        attempted_samples: Option<usize>,
    },
    Cancelled {
        sample_count: usize,
    },
    Recovered {
        sample_count: usize,
        was_gesture: bool,
        reason: RecoveryReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct UiPoint {
    pub x: i32,
    pub y: i32,
}

pub struct UiStrokeBegin {
    pub points: Vec<UiPoint>,
}

struct Candidate {
    session_id: usize,
    button: TriggerButton,
    started_at: Instant,
    start: UiPoint,
    last: UiPoint,
    points: Vec<GesturePoint>,
    path_length: f32,
    max_distance_px: f32,
    activation_distance_px: f32,
    minimum_stroke_length_px: f32,
    active: bool,
    activation_delay_ms: u32,
    target_hwnd: isize,
    last_overlay_post: Instant,
    watchdog: SessionWatchdog,
}

#[derive(Default)]
struct HookState {
    candidate: Option<Candidate>,
}

struct HookContext {
    state: Mutex<HookState>,
    config: Arc<RwLock<AppConfig>>,
    command_sender: Sender<HookCommand>,
    ui_hwnd: AtomicIsize,
    active_session: AtomicUsize,
    active_button: AtomicU32,
    press_time: AtomicU32,
    released_session: AtomicUsize,
    next_session: AtomicUsize,
    cancel_requested: AtomicUsize,
    watchdog_timer: AtomicUsize,
    thread_id: AtomicU32,
}

static CONTEXT: OnceLock<HookContext> = OnceLock::new();
static MOUSE_HOOK: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static LAST_EXTERNAL_FOREGROUND: AtomicIsize = AtomicIsize::new(0);
static LAST_EXTERNAL_PROCESS_ID: AtomicU32 = AtomicU32::new(0);
static LAST_EXTERNAL_IS_EXCLUDED: AtomicBool = AtomicBool::new(false);

pub fn start(
    config: Arc<RwLock<AppConfig>>,
    command_sender: Sender<HookCommand>,
    ui_hwnd: isize,
) -> Result<()> {
    CONTEXT
        .set(HookContext {
            state: Mutex::new(HookState::default()),
            config,
            command_sender,
            ui_hwnd: AtomicIsize::new(ui_hwnd),
            active_session: AtomicUsize::new(0),
            active_button: AtomicU32::new(0),
            press_time: AtomicU32::new(0),
            released_session: AtomicUsize::new(0),
            next_session: AtomicUsize::new(0),
            cancel_requested: AtomicUsize::new(0),
            watchdog_timer: AtomicUsize::new(0),
            thread_id: AtomicU32::new(0),
        })
        .map_err(|_| anyhow::anyhow!("鼠标钩子已启动"))?;

    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("xmouse-hook".to_owned())
        .spawn(move || unsafe {
            let context = CONTEXT.get().expect("hook context initialized");
            context
                .thread_id
                .store(GetCurrentThreadId(), Ordering::Release);
            remember_external_foreground(GetForegroundWindow());
            let module = GetModuleHandleW(ptr::null()) as HINSTANCE;
            let installed = replace_mouse_hook(module);
            let _ = ready_sender.send(installed);
            if !installed {
                return;
            }
            crate::logging::info("鼠标钩子", "已安装");
            let foreground_hook = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                ptr::null_mut(),
                Some(foreground_event_hook),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if foreground_hook.is_null() {
                crate::logging::error("监控前台窗口", "安装 WinEvent 钩子失败");
            }
            // A low-level hook can be silently removed after a callback timeout.
            // A thread timer gives it a bounded recovery time even when the
            // foreground window does not change. Never refresh mid-stroke.
            let refresh_timer = SetTimer(ptr::null_mut(), 0, HOOK_REFRESH_INTERVAL_MS, None);
            if refresh_timer == 0 {
                crate::logging::error("鼠标钩子", "创建恢复计时器失败");
            }
            let mut message = MSG::default();
            loop {
                let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
                if result <= 0 {
                    if result < 0 {
                        crate::logging::error("鼠标钩子", "消息循环读取失败");
                    }
                    break;
                }
                if message.hwnd.is_null() && message.message == WM_APP_CANCEL_SESSION {
                    if cancel_session(context, message.wParam, RecoveryReason::DeferredCleanup)
                        && !replace_mouse_hook(module)
                    {
                        crate::logging::error("鼠标钩子", "清理遗留手势后恢复失败");
                    }
                    continue;
                }
                if message.hwnd.is_null()
                    && message.message == WM_TIMER
                    && message.wParam == context.watchdog_timer.load(Ordering::Acquire)
                    && message.wParam != 0
                {
                    if recover_session(context) && !replace_mouse_hook(module) {
                        crate::logging::error("鼠标钩子", "清理遗留手势后恢复失败");
                    }
                    continue;
                }
                if refresh_timer != 0
                    && message.hwnd.is_null()
                    && message.message == WM_TIMER
                    && message.wParam == refresh_timer
                    && context.active_session.load(Ordering::Acquire) == 0
                    && !replace_mouse_hook(module)
                {
                    crate::logging::error("鼠标钩子", "定时恢复失败");
                }
            }
            if refresh_timer != 0 {
                KillTimer(ptr::null_mut(), refresh_timer);
            }
            stop_session_watchdog(context);
            if !foreground_hook.is_null() {
                UnhookWinEvent(foreground_hook);
            }
            let hook = MOUSE_HOOK.swap(ptr::null_mut(), Ordering::AcqRel) as HHOOK;
            if !hook.is_null() {
                UnhookWindowsHookEx(hook);
            }
        })?;

    if ready_receiver.recv().unwrap_or(false) {
        Ok(())
    } else {
        bail!("安装全局鼠标钩子失败")
    }
}

unsafe fn replace_mouse_hook(module: HINSTANCE) -> bool {
    let replacement = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
    if replacement.is_null() {
        return false;
    }
    let previous = MOUSE_HOOK.swap(replacement.cast(), Ordering::AcqRel) as HHOOK;
    if !previous.is_null() && unsafe { UnhookWindowsHookEx(previous) } == 0 {
        crate::logging::info("鼠标钩子", "旧钩子句柄已失效，已安装替代钩子");
    }
    true
}

unsafe extern "system" fn foreground_event_hook(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND {
        return;
    }
    remember_external_foreground(hwnd);
    if CONTEXT
        .get()
        .is_some_and(|context| context.active_session.load(Ordering::Acquire) != 0)
    {
        return;
    }
    let module = unsafe { GetModuleHandleW(ptr::null()) as HINSTANCE };
    if !unsafe { replace_mouse_hook(module) } {
        crate::logging::error("恢复鼠标钩子", "前台窗口切换后重新安装失败");
    }
}

pub fn last_external_foreground() -> isize {
    LAST_EXTERNAL_FOREGROUND.load(Ordering::Acquire)
}

fn remember_external_foreground(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    let root = if root.is_null() { hwnd } else { root };
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(root, &mut process_id);
    }
    if process_id == 0
        || process_id == unsafe { GetCurrentProcessId() }
        || is_system_shell_surface(root)
    {
        return;
    }
    LAST_EXTERNAL_FOREGROUND.store(root as isize, Ordering::Release);
    LAST_EXTERNAL_PROCESS_ID.store(process_id, Ordering::Release);
    let excluded = CONTEXT
        .get()
        .and_then(|context| context.config.try_read().ok())
        .and_then(|config| {
            process_name(process_id).map(|name| {
                config
                    .gesture_guard
                    .excluded_processes
                    .iter()
                    .any(|item| item.eq_ignore_ascii_case(&name))
            })
        })
        .unwrap_or(false);
    LAST_EXTERNAL_IS_EXCLUDED.store(excluded, Ordering::Release);
}

pub fn update_ui_hwnd(hwnd: isize) {
    if let Some(context) = CONTEXT.get() {
        context.ui_hwnd.store(hwnd, Ordering::Release);
    }
}

pub fn active_session_id() -> usize {
    CONTEXT
        .get()
        .map(|context| context.active_session.load(Ordering::Acquire))
        .unwrap_or(0)
}

pub fn session_has_released(session_id: usize) -> bool {
    session_id != 0
        && CONTEXT
            .get()
            .is_some_and(|context| context.released_session.load(Ordering::Acquire) == session_id)
}

pub fn note_physical_release(button: TriggerButton, event_time: u32) {
    if let Some(context) = CONTEXT.get() {
        note_release(context, button, event_time);
    }
}

fn button_code(button: TriggerButton) -> u32 {
    match button {
        TriggerButton::Right => 1,
        TriggerButton::X1 => 2,
        TriggerButton::X2 => 3,
    }
}

fn note_release(context: &HookContext, button: TriggerButton, event_time: u32) {
    let session_id = context.active_session.load(Ordering::Acquire);
    if session_id != 0
        && context.active_button.load(Ordering::Acquire) == button_code(button)
        && event_time.wrapping_sub(context.press_time.load(Ordering::Acquire)) <= i32::MAX as u32
    {
        context
            .released_session
            .store(session_id, Ordering::Release);
    }
}

// The UI can hide a stuck overlay immediately; its owning hook thread must
// clear the candidate too, so the next press is not blocked by stale state.
pub fn request_session_cancel(session_id: usize) {
    if let Some(context) = CONTEXT.get()
        && session_id != 0
        && context.active_session.load(Ordering::Acquire) == session_id
    {
        context
            .cancel_requested
            .store(session_id, Ordering::Release);
        unsafe {
            PostThreadMessageW(
                context.thread_id.load(Ordering::Acquire),
                WM_APP_CANCEL_SESSION,
                session_id,
                0,
            );
        }
    }
}

fn start_session_watchdog(context: &HookContext) {
    if context.watchdog_timer.load(Ordering::Acquire) == 0 {
        let timer = unsafe { SetTimer(ptr::null_mut(), 0, WATCHDOG_INTERVAL_MS, None) };
        context.watchdog_timer.store(timer, Ordering::Release);
        if timer == 0 {
            let session_id = context.active_session.load(Ordering::Acquire);
            request_session_cancel(session_id);
        }
    }
}

fn stop_session_watchdog(context: &HookContext) {
    let timer = context.watchdog_timer.swap(0, Ordering::AcqRel);
    if timer != 0 {
        unsafe {
            KillTimer(ptr::null_mut(), timer);
        }
    }
}

fn cancel_session(context: &HookContext, session_id: usize, reason: RecoveryReason) -> bool {
    let active = context.active_session.load(Ordering::Acquire);
    if session_id == 0 || (active != 0 && active != session_id) {
        let _ = context.cancel_requested.compare_exchange(
            session_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        return false;
    }
    context
        .cancel_requested
        .store(session_id, Ordering::Release);
    post_simple(context, WM_APP_OVERLAY_END, session_id);
    let Ok(mut state) = context.state.try_lock() else {
        // Keep the candidate and gate coherent. The timer/next input retries
        // cleanup rather than clearing only the atomic flag on contention.
        return false;
    };
    if state
        .candidate
        .as_ref()
        .is_some_and(|candidate| candidate.session_id != session_id)
    {
        let _ = context.cancel_requested.compare_exchange(
            session_id,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        return false;
    }
    let candidate = state.candidate.take();
    let was_active = context
        .active_session
        .compare_exchange(session_id, 0, Ordering::AcqRel, Ordering::Acquire)
        .is_ok();
    let _ = context.cancel_requested.compare_exchange(
        session_id,
        0,
        Ordering::AcqRel,
        Ordering::Acquire,
    );
    drop(state);
    if !was_active && candidate.is_none() {
        return false;
    }
    stop_session_watchdog(context);
    let (sample_count, was_gesture) = candidate
        .map(|candidate| {
            (
                candidate.points.len(),
                candidate.active
                    || attempted_gesture(
                        candidate.max_distance_px,
                        candidate.activation_distance_px,
                    ),
            )
        })
        .unwrap_or((0, false));
    let _ = context.command_sender.send(HookCommand::Recovered {
        sample_count,
        was_gesture,
        reason,
    });
    true
}

fn recover_session(context: &HookContext) -> bool {
    let requested = context.cancel_requested.load(Ordering::Acquire);
    if requested != 0 {
        return cancel_session(context, requested, RecoveryReason::DeferredCleanup);
    }
    let session_id = context.active_session.load(Ordering::Acquire);
    if session_id == 0 {
        stop_session_watchdog(context);
        return false;
    }
    let reason = {
        let Ok(mut state) = context.state.try_lock() else {
            return false;
        };
        match state.candidate.as_mut() {
            Some(candidate) => candidate.watchdog.poll(
                Instant::now(),
                context.released_session.load(Ordering::Acquire) == candidate.session_id,
            ),
            None => Some(RecoveryReason::InconsistentState),
        }
    };
    reason.is_some_and(|reason| cancel_session(context, session_id, reason))
}

pub fn replay_button(button: TriggerButton) -> Result<()> {
    let (down, up, mouse_data) = match button {
        TriggerButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
        TriggerButton::X1 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON1_VALUE as u32),
        TriggerButton::X2 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON2_VALUE as u32),
    };
    let mut inputs = [
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: mouse_data,
                    dwFlags: down,
                    time: 0,
                    dwExtraInfo: INJECTED_EVENT_TOKEN,
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: mouse_data,
                    dwFlags: up,
                    time: 0,
                    dwExtraInfo: INJECTED_EVENT_TOKEN,
                },
            },
        },
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        bail!("重放鼠标按键失败")
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }
    let Some(context) = CONTEXT.get() else {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    };
    let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
    // Injected clicks must not start another candidate. Matching only our
    // dwExtraInfo token proved insufficient on Windows: replayed right clicks
    // could recursively trigger another replay until Xmouse was stopped.
    // Pass injected input through to its target, but never treat it as a
    // physical gesture.
    if should_ignore_mouse_event(event.flags, event.dwExtraInfo) {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }

    let message = wparam as u32;
    let event_button = trigger_for_event(message, event.mouseData);
    let pending_cancel = context.cancel_requested.load(Ordering::Acquire);
    if pending_cancel != 0 {
        cancel_session(context, pending_cancel, RecoveryReason::DeferredCleanup);
    }
    let session_id = context.active_session.load(Ordering::Acquire);
    if session_id != 0 && matches!(message, WM_RBUTTONDOWN | WM_XBUTTONDOWN) {
        let same_button = context.state.try_lock().ok().is_some_and(|state| {
            state
                .candidate
                .as_ref()
                .is_some_and(|candidate| event_button == Some(candidate.button))
        });
        if same_button {
            cancel_session(context, session_id, RecoveryReason::NewPress);
        }
    }
    if is_trigger_up(message, event_button) && context.active_session.load(Ordering::Acquire) != 0 {
        let session_id = context.active_session.load(Ordering::Acquire);
        let Ok(mut state) = context.state.try_lock() else {
            request_session_cancel(session_id);
            post_simple(context, WM_APP_OVERLAY_END, session_id);
            return 1;
        };
        if context.cancel_requested.load(Ordering::Acquire) == session_id {
            drop(state);
            cancel_session(context, session_id, RecoveryReason::DeferredCleanup);
            return 1;
        }
        let Some(candidate) = state.candidate.take() else {
            drop(state);
            cancel_session(context, session_id, RecoveryReason::InconsistentState);
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        };
        if event_button != Some(candidate.button) {
            state.candidate = Some(candidate);
            drop(state);
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        }
        context.active_session.store(0, Ordering::Release);
        context.cancel_requested.store(0, Ordering::Release);
        drop(state);
        stop_session_watchdog(context);
        post_simple(context, WM_APP_OVERLAY_END, candidate.session_id);

        let committed = gesture_committed(
            candidate.path_length,
            candidate.max_distance_px,
            candidate.activation_distance_px,
            candidate.minimum_stroke_length_px,
        );
        if !committed {
            let _ = context.command_sender.send(HookCommand::Replay {
                button: candidate.button,
                attempted_samples: attempted_gesture(
                    candidate.max_distance_px,
                    candidate.activation_distance_px,
                )
                .then_some(candidate.points.len()),
            });
        } else if candidate.active {
            let _ = context
                .command_sender
                .send(HookCommand::Stroke(StrokeRequest {
                    points: candidate.points,
                    target_hwnd: candidate.target_hwnd,
                }));
        } else {
            let _ = context.command_sender.send(HookCommand::Cancelled {
                sample_count: candidate.points.len(),
            });
        }
        return 1;
    }

    if message == WM_MOUSEMOVE {
        if context.active_session.load(Ordering::Acquire) == 0
            || context.cancel_requested.load(Ordering::Acquire) != 0
        {
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        }
        let Ok(mut state) = context.state.try_lock() else {
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        };
        let Some(candidate) = state.candidate.as_mut() else {
            drop(state);
            let session_id = context.active_session.load(Ordering::Acquire);
            cancel_session(context, session_id, RecoveryReason::InconsistentState);
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        };
        let dx = event.pt.x - candidate.last.x;
        let dy = event.pt.y - candidate.last.y;
        let segment = ((dx * dx + dy * dy) as f32).sqrt();
        let start_dx = event.pt.x - candidate.start.x;
        let start_dy = event.pt.y - candidate.start.y;
        let distance_from_start = ((start_dx * start_dx + start_dy * start_dy) as f32).sqrt();
        candidate.max_distance_px = candidate.max_distance_px.max(distance_from_start);
        if segment >= 1.5 {
            candidate.path_length += segment;
            candidate.last = UiPoint {
                x: event.pt.x,
                y: event.pt.y,
            };
            if candidate.points.len() < 2_048 {
                candidate
                    .points
                    .push(GesturePoint::new(event.pt.x as f32, event.pt.y as f32));
            }
        }
        let mut begin = None;
        let mut point_update = None;
        if !candidate.active
            && candidate.started_at.elapsed().as_millis() >= candidate.activation_delay_ms as u128
            && candidate.max_distance_px >= candidate.activation_distance_px
        {
            candidate.active = true;
            candidate.last_overlay_post = Instant::now();
            begin = Some((
                candidate.session_id,
                UiStrokeBegin {
                    points: stroke_ui_points(&candidate.points),
                },
            ));
        }
        if candidate.active && candidate.last_overlay_post.elapsed().as_millis() >= 12 {
            candidate.last_overlay_post = Instant::now();
            point_update = Some(candidate.session_id);
        }
        drop(state);
        if let Some((session_id, begin)) = begin {
            post_stroke_begin(context, session_id, begin);
        }
        if let Some(session_id) = point_update {
            post_point(context, session_id, event.pt.x, event.pt.y);
        }
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }

    let (
        enabled,
        trigger,
        activation_delay_ms,
        activation_distance_dip,
        minimum_stroke_length_dip,
        guard,
    ) = {
        let Ok(config) = context.config.try_read() else {
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        };
        (
            config.enabled,
            config.trigger,
            config.activation_delay_ms,
            config.activation_distance_dip,
            config.minimum_stroke_length_dip,
            config.gesture_guard.clone(),
        )
    };
    if !enabled
        || !is_trigger_down(message, event_button, trigger)
        || context.active_session.load(Ordering::Acquire) != 0
    {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }

    let point = event.pt;
    let target = target_window(point);
    if target.is_null() || is_system_shell_surface(target) || is_gesture_blocked(target, &guard) {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }
    let dpi = unsafe { GetDpiForWindow(target) }.max(96);
    let activation_distance_px = activation_distance_dip * dpi as f32 / 96.0;
    let minimum_stroke_length_px = minimum_stroke_length_dip * dpi as f32 / 96.0;
    let now = Instant::now();
    let Ok(mut state) = context.state.try_lock() else {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    };
    if state.candidate.is_some() {
        drop(state);
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }
    let session_id = context
        .next_session
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
        .max(1);
    state.candidate = Some(Candidate {
        session_id,
        button: trigger,
        started_at: now,
        start: UiPoint {
            x: point.x,
            y: point.y,
        },
        last: UiPoint {
            x: point.x,
            y: point.y,
        },
        points: vec![GesturePoint::new(point.x as f32, point.y as f32)],
        path_length: 0.0,
        max_distance_px: 0.0,
        activation_distance_px,
        minimum_stroke_length_px,
        active: false,
        activation_delay_ms,
        target_hwnd: target as isize,
        last_overlay_post: now,
        watchdog: SessionWatchdog::new(now),
    });
    context
        .active_button
        .store(button_code(trigger), Ordering::Release);
    context.press_time.store(event.time, Ordering::Release);
    context.released_session.store(0, Ordering::Release);
    context.active_session.store(session_id, Ordering::Release);
    drop(state);
    start_session_watchdog(context);
    1
}

fn target_window(point: POINT) -> HWND {
    let window = unsafe { WindowFromPoint(point) };
    if window.is_null() {
        return window;
    }
    let root = unsafe { GetAncestor(window, GA_ROOT) };
    if root.is_null() { window } else { root }
}

fn is_system_shell_surface(hwnd: HWND) -> bool {
    let mut class_name = [0u16; 128];
    let length = unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), class_name.len() as i32) };
    if length <= 0 {
        return false;
    }
    let class_name = String::from_utf16_lossy(&class_name[..length as usize]);
    is_system_shell_class(&class_name)
}

fn is_system_shell_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "NotifyIconOverflowWindow"
            | "TaskListThumbnailWnd"
            | "Xaml_WindowedPopupClass"
            | "#32768"
    )
}

fn is_gesture_blocked(hwnd: HWND, guard: &GestureGuardConfig) -> bool {
    (guard.disable_in_fullscreen_apps && is_fullscreen_target(hwnd))
        || is_process_excluded(hwnd, &guard.excluded_processes)
}

fn is_process_excluded(hwnd: HWND, excluded_processes: &[String]) -> bool {
    if excluded_processes.is_empty() {
        return false;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut process_id);
    }
    if process_id == LAST_EXTERNAL_PROCESS_ID.load(Ordering::Acquire) {
        return LAST_EXTERNAL_IS_EXCLUDED.load(Ordering::Acquire);
    }
    let Some(name) = process_name(process_id) else {
        return false;
    };
    excluded_processes
        .iter()
        .any(|item| item.eq_ignore_ascii_case(&name))
}

fn is_fullscreen_target(hwnd: HWND) -> bool {
    if hwnd.is_null() || unsafe { IsIconic(hwnd) } != 0 || unsafe { IsZoomed(hwnd) } != 0 {
        return false;
    }
    let shell = unsafe { GetShellWindow() };
    if hwnd == shell {
        return false;
    }

    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return false;
    }
    let foreground_root = unsafe { GetAncestor(foreground, GA_ROOT) };
    let foreground_root = if foreground_root.is_null() {
        foreground
    } else {
        foreground_root
    };
    if foreground_root != hwnd {
        return false;
    }

    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return false;
    }
    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut monitor_info) } == 0 {
        return false;
    }
    let mut window_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut window_rect) } == 0 {
        return false;
    }

    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    is_fullscreen_window(style, window_rect, monitor_info.rcMonitor)
}

fn is_fullscreen_window(style: u32, window: RECT, monitor: RECT) -> bool {
    // IsZoomed is false for a restored browser stretched over a monitor.
    // Its conventional title bar still identifies an ordinary framed window;
    // monitor coverage alone must not disable gestures there. Test the full
    // WS_CAPTION mask: a border-only popup is still eligible for game protection.
    style & WS_CAPTION != WS_CAPTION && rect_covers_monitor(window, monitor, 8)
}

fn rect_covers_monitor(window: RECT, monitor: RECT, tolerance: i32) -> bool {
    window.left <= monitor.left + tolerance
        && window.top <= monitor.top + tolerance
        && window.right >= monitor.right - tolerance
        && window.bottom >= monitor.bottom - tolerance
}

fn post_point(context: &HookContext, session_id: usize, x: i32, y: i32) {
    let hwnd = context.ui_hwnd.load(Ordering::Acquire) as HWND;
    if hwnd.is_null() {
        return;
    }
    let point = Box::new(UiPoint { x, y });
    let pointer = Box::into_raw(point);
    if unsafe { PostMessageW(hwnd, WM_APP_OVERLAY_POINT, session_id, pointer as LPARAM) } == 0 {
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

fn post_stroke_begin(context: &HookContext, session_id: usize, begin: UiStrokeBegin) {
    let hwnd = context.ui_hwnd.load(Ordering::Acquire) as HWND;
    if hwnd.is_null() {
        return;
    }
    let begin = Box::new(begin);
    let pointer = Box::into_raw(begin);
    if unsafe { PostMessageW(hwnd, WM_APP_OVERLAY_BEGIN, session_id, pointer as LPARAM) } == 0 {
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

fn stroke_ui_points(points: &[GesturePoint]) -> Vec<UiPoint> {
    points
        .iter()
        .map(|point| UiPoint {
            x: point.x as i32,
            y: point.y as i32,
        })
        .collect()
}

fn post_simple(context: &HookContext, message: u32, session_id: usize) {
    let hwnd = context.ui_hwnd.load(Ordering::Acquire) as HWND;
    if !hwnd.is_null() {
        unsafe {
            PostMessageW(hwnd, message, session_id, 0);
        }
    }
}

fn trigger_for_event(message: u32, mouse_data: u32) -> Option<TriggerButton> {
    match message {
        WM_RBUTTONDOWN | WM_RBUTTONUP => Some(TriggerButton::Right),
        WM_XBUTTONDOWN | WM_XBUTTONUP => match (mouse_data >> 16) as u16 {
            value if value == XBUTTON1_VALUE => Some(TriggerButton::X1),
            value if value == XBUTTON2_VALUE => Some(TriggerButton::X2),
            _ => None,
        },
        _ => None,
    }
}

fn is_trigger_down(
    message: u32,
    event_button: Option<TriggerButton>,
    configured: TriggerButton,
) -> bool {
    matches!(message, WM_RBUTTONDOWN | WM_XBUTTONDOWN)
        && event_button.is_some_and(|button| button == configured)
}

fn is_trigger_up(message: u32, event_button: Option<TriggerButton>) -> bool {
    matches!(message, WM_RBUTTONUP | WM_XBUTTONUP) && event_button.is_some()
}

fn gesture_committed(
    path_length: f32,
    max_distance: f32,
    activation_distance: f32,
    minimum_stroke_length: f32,
) -> bool {
    path_length >= minimum_stroke_length && max_distance >= activation_distance * 1.5
}

fn attempted_gesture(max_distance: f32, activation_distance: f32) -> bool {
    max_distance >= activation_distance * 1.5
}

fn should_ignore_mouse_event(flags: u32, extra_info: usize) -> bool {
    flags & LLMHF_INJECTED != 0 || extra_info == INJECTED_EVENT_TOKEN
}

#[cfg(test)]
mod tests {
    use super::{
        Candidate, GesturePoint, HookCommand, HookContext, HookState, INJECTED_EVENT_TOKEN, RECT,
        UiPoint, WM_APP_OVERLAY_END, attempted_gesture, cancel_session, gesture_committed,
        is_fullscreen_window, is_system_shell_class, note_release, recover_session,
        rect_covers_monitor, should_ignore_mouse_event, start_session_watchdog,
        stop_session_watchdog, stroke_ui_points,
    };
    use crate::{
        config::{AppConfig, TriggerButton},
        gesture_lifecycle::{RecoveryReason, SessionWatchdog},
    };
    use std::{
        ptr,
        sync::{
            Arc, Mutex, RwLock,
            atomic::{AtomicIsize, AtomicU32, AtomicUsize, Ordering},
            mpsc::{self, Receiver},
        },
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, LLMHF_INJECTED, MSG, PM_REMOVE, PeekMessageW,
        WM_TIMER, WS_CAPTION,
    };

    #[test]
    fn native_watchdog_recovers_a_missing_release_and_stops_its_timer() {
        let (context, receiver) = test_context(42);
        let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                ptr::null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        assert!(!hwnd.is_null());
        context.ui_hwnd.store(hwnd as isize, Ordering::Release);
        // Deliberately omit the hook's button-up path. Only raw release arrives.
        context.released_session.store(42, Ordering::Release);
        start_session_watchdog(&context);
        let mut received_end = false;
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let mut message = MSG::default();
            while unsafe { PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
                if message.message == WM_TIMER
                    && message.wParam == context.watchdog_timer.load(Ordering::Acquire)
                {
                    recover_session(&context);
                } else if message.hwnd == hwnd && message.message == WM_APP_OVERLAY_END {
                    received_end |= message.wParam == 42;
                }
            }
            if received_end && context.active_session.load(Ordering::Acquire) == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let timer_stopped = context.watchdog_timer.load(Ordering::Acquire) == 0;
        stop_session_watchdog(&context);
        unsafe {
            DestroyWindow(hwnd);
        }
        assert!(
            received_end,
            "overlay end was not delivered by the native timer"
        );
        assert!(timer_stopped, "watchdog must not remain polling while idle");
        assert!(context.state.lock().unwrap().candidate.is_none());
        assert!(matches!(
            receiver.try_recv().unwrap(),
            HookCommand::Recovered {
                reason: RecoveryReason::MissingRelease,
                ..
            }
        ));
        assert!(
            receiver.try_recv().is_err(),
            "recovery must not execute or replay input"
        );
        let (next, _) = test_context(43);
        context.state.lock().unwrap().candidate = next.state.into_inner().unwrap().candidate;
        context.active_session.store(43, Ordering::Release);
        assert!(cancel_session(
            &context,
            43,
            RecoveryReason::DeferredCleanup
        ));
        assert_eq!(context.active_session.load(Ordering::Acquire), 0);
    }

    fn test_context(session_id: usize) -> (HookContext, Receiver<HookCommand>) {
        let now = Instant::now();
        let (sender, receiver) = mpsc::channel();
        let context = HookContext {
            state: Mutex::new(HookState {
                candidate: Some(Candidate {
                    session_id,
                    button: TriggerButton::Right,
                    started_at: now,
                    start: UiPoint { x: 10, y: 10 },
                    last: UiPoint { x: 110, y: 10 },
                    points: vec![
                        GesturePoint::new(10.0, 10.0),
                        GesturePoint::new(110.0, 10.0),
                    ],
                    path_length: 100.0,
                    max_distance_px: 100.0,
                    activation_distance_px: 12.0,
                    minimum_stroke_length_px: 28.0,
                    active: true,
                    activation_delay_ms: 180,
                    target_hwnd: 0,
                    last_overlay_post: now,
                    watchdog: SessionWatchdog::new(now),
                }),
            }),
            config: Arc::new(RwLock::new(AppConfig::default())),
            command_sender: sender,
            ui_hwnd: AtomicIsize::new(0),
            active_session: AtomicUsize::new(session_id),
            active_button: AtomicU32::new(1),
            press_time: AtomicU32::new(100),
            released_session: AtomicUsize::new(0),
            next_session: AtomicUsize::new(session_id),
            cancel_requested: AtomicUsize::new(0),
            watchdog_timer: AtomicUsize::new(0),
            thread_id: AtomicU32::new(0),
        };
        (context, receiver)
    }

    #[test]
    fn contended_release_is_deferred_and_clears_the_entire_session_without_execution() {
        let (context, receiver) = test_context(1);
        let guard = context.state.lock().unwrap();
        assert!(!cancel_session(
            &context,
            1,
            RecoveryReason::DeferredCleanup
        ));
        assert_eq!(context.active_session.load(Ordering::Acquire), 1);
        assert_eq!(context.cancel_requested.load(Ordering::Acquire), 1);
        drop(guard);
        assert!(cancel_session(&context, 1, RecoveryReason::DeferredCleanup));
        assert_eq!(context.active_session.load(Ordering::Acquire), 0);
        assert_eq!(context.cancel_requested.load(Ordering::Acquire), 0);
        assert!(context.state.lock().unwrap().candidate.is_none());
        assert!(matches!(
            receiver.try_recv().unwrap(),
            HookCommand::Recovered {
                was_gesture: true,
                ..
            }
        ));
        assert!(receiver.try_recv().is_err());
        // Repeated cleanup cannot replay the click or execute the abandoned stroke.
        assert!(!cancel_session(&context, 1, RecoveryReason::MissingRelease));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn a_delayed_cancellation_cannot_cancel_the_next_gesture() {
        let (context, receiver) = test_context(2);
        context.cancel_requested.store(1, Ordering::Release);
        assert!(!cancel_session(
            &context,
            1,
            RecoveryReason::DeferredCleanup
        ));
        assert_eq!(context.cancel_requested.load(Ordering::Acquire), 0);
        assert_eq!(context.active_session.load(Ordering::Acquire), 2);
        assert_eq!(
            context
                .state
                .lock()
                .unwrap()
                .candidate
                .as_ref()
                .unwrap()
                .session_id,
            2
        );
        assert!(receiver.try_recv().is_err());
        assert!(cancel_session(&context, 2, RecoveryReason::MissingRelease));
        assert!(context.state.lock().unwrap().candidate.is_none());
    }

    #[test]
    fn raw_release_ignores_the_wrong_button_and_previous_stroke_even_across_tick_wrap() {
        let (context, _) = test_context(2);
        note_release(&context, TriggerButton::X1, 110);
        note_release(&context, TriggerButton::Right, 99);
        assert_eq!(context.released_session.load(Ordering::Acquire), 0);
        note_release(&context, TriggerButton::Right, 110);
        assert_eq!(context.released_session.load(Ordering::Acquire), 2);
        context.active_session.store(3, Ordering::Release);
        context.press_time.store(200, Ordering::Release);
        note_release(&context, TriggerButton::Right, 150);
        assert_ne!(context.released_session.load(Ordering::Acquire), 3);
        context.press_time.store(u32::MAX - 5, Ordering::Release);
        note_release(&context, TriggerButton::Right, 4);
        assert_eq!(context.released_session.load(Ordering::Acquire), 3);
    }

    #[test]
    fn injected_mouse_events_never_reenter_gesture_recognition() {
        assert!(should_ignore_mouse_event(LLMHF_INJECTED, 0));
        assert!(should_ignore_mouse_event(
            LLMHF_INJECTED,
            INJECTED_EVENT_TOKEN
        ));
        assert!(should_ignore_mouse_event(0, INJECTED_EVENT_TOKEN));
        assert!(!should_ignore_mouse_event(0, 0));
    }

    #[test]
    fn short_or_jittery_drag_is_not_committed() {
        assert!(!gesture_committed(20.0, 18.0, 12.0, 28.0));
        assert!(!gesture_committed(80.0, 8.0, 12.0, 28.0));
    }

    #[test]
    fn only_clear_short_drags_are_logged_as_attempted_gestures() {
        assert!(!attempted_gesture(2.0, 12.0));
        assert!(!attempted_gesture(17.0, 12.0));
        assert!(attempted_gesture(18.0, 12.0));
    }

    #[test]
    fn deliberate_drag_is_committed() {
        assert!(gesture_committed(40.0, 24.0, 12.0, 28.0));
    }

    #[test]
    fn overlay_activation_keeps_the_buffered_path() {
        let points = [
            GesturePoint::new(10.0, 20.0),
            GesturePoint::new(16.0, 28.0),
            GesturePoint::new(25.0, 31.0),
        ];
        assert_eq!(
            stroke_ui_points(&points),
            vec![
                UiPoint { x: 10, y: 20 },
                UiPoint { x: 16, y: 28 },
                UiPoint { x: 25, y: 31 },
            ]
        );
    }

    #[test]
    fn fullscreen_rect_accepts_exact_and_small_overscan_bounds() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        };
        assert!(rect_covers_monitor(monitor, monitor, 8));
        assert!(rect_covers_monitor(
            RECT {
                left: -2,
                top: -2,
                right: 2562,
                bottom: 1442,
            },
            monitor,
            8,
        ));
    }

    #[test]
    fn fullscreen_rect_rejects_work_area_and_partial_windows() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(!rect_covers_monitor(
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            monitor,
            8,
        ));
        assert!(!rect_covers_monitor(
            RECT {
                left: 160,
                top: 90,
                right: 1760,
                bottom: 990,
            },
            monitor,
            8,
        ));
    }

    #[test]
    fn restored_edge_with_monitor_sized_bounds_is_not_fullscreen() {
        // Captured from the user's normal Edge window: not IsZoomed, but
        // the old one-sided geometry predicate still covered the monitor.
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1440,
        };
        let edge = RECT {
            left: 2,
            top: 2,
            right: 2578,
            bottom: 1458,
        };
        assert!(rect_covers_monitor(edge, monitor, 8));
        assert!(!is_fullscreen_window(0x16CF_0000, edge, monitor));
    }

    #[test]
    fn framed_windows_are_not_fullscreen_even_with_exact_or_oversized_bounds() {
        let monitor = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        for bounds in [
            monitor,
            RECT {
                left: -8,
                top: -8,
                right: 1928,
                bottom: 1088,
            },
        ] {
            assert!(!is_fullscreen_window(WS_CAPTION, bounds, monitor));
            assert!(!is_fullscreen_window(
                WS_CAPTION | 0x8000_0000,
                bounds,
                monitor
            ));
        }
    }

    #[test]
    fn borderless_games_remain_protected_on_scaled_and_negative_origin_monitors() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WS_BORDER, WS_POPUP, WS_THICKFRAME};
        for monitor in [
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            RECT {
                left: -3840,
                top: -1080,
                right: 0,
                bottom: 1080,
            },
            RECT {
                left: 2560,
                top: 0,
                right: 6400,
                bottom: 2160,
            },
        ] {
            for style in [0, WS_POPUP, WS_POPUP | WS_THICKFRAME, WS_POPUP | WS_BORDER] {
                assert!(is_fullscreen_window(style, monitor, monitor));
                let overscan = RECT {
                    left: monitor.left - 8,
                    top: monitor.top - 8,
                    right: monitor.right + 8,
                    bottom: monitor.bottom + 8,
                };
                assert!(is_fullscreen_window(style, overscan, monitor));
            }
        }
    }

    #[test]
    fn taskbar_and_system_popup_classes_never_start_gestures() {
        for class_name in [
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
            "NotifyIconOverflowWindow",
            "TaskListThumbnailWnd",
            "Xaml_WindowedPopupClass",
            "#32768",
        ] {
            assert!(is_system_shell_class(class_name), "{class_name}");
        }
        for class_name in ["Chrome_WidgetWin_1", "Notepad", "CabinetWClass", "WorkerW"] {
            assert!(!is_system_shell_class(class_name), "{class_name}");
        }
    }
}
