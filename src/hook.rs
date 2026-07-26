use crate::{
    clipboard::process_name,
    config::{AppConfig, GestureGuardConfig, TriggerButton},
    gesture::Point as GesturePoint,
};
use anyhow::{Result, bail};
use std::{
    ptr,
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicBool, AtomicIsize, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
            MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput,
        },
        WindowsAndMessaging::{
            CallNextHookEx, GA_ROOT, GetAncestor, GetForegroundWindow, GetMessageW, GetShellWindow,
            GetWindowRect, GetWindowThreadProcessId, IsIconic, IsZoomed, MSG, MSLLHOOKSTRUCT,
            PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_MOUSEMOVE,
            WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP, WindowFromPoint,
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

pub const INJECTED_EVENT_TOKEN: usize = 0x4743_4C49_505F_0001;
const XBUTTON1_VALUE: u16 = 0x0001;
const XBUTTON2_VALUE: u16 = 0x0002;

#[derive(Debug)]
pub struct StrokeRequest {
    pub points: Vec<GesturePoint>,
    pub target_hwnd: isize,
}

#[derive(Debug)]
pub enum HookCommand {
    Stroke(StrokeRequest),
    Replay(TriggerButton),
    Cancelled,
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
    candidate_present: AtomicBool,
}

static CONTEXT: OnceLock<HookContext> = OnceLock::new();

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
            candidate_present: AtomicBool::new(false),
        })
        .map_err(|_| anyhow::anyhow!("鼠标钩子已启动"))?;

    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("xmouse-hook".to_owned())
        .spawn(move || unsafe {
            let module = GetModuleHandleW(ptr::null()) as HINSTANCE;
            let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0);
            let _ = ready_sender.send(!hook.is_null());
            if hook.is_null() {
                return;
            }
            let mut message = MSG::default();
            while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {}
            UnhookWindowsHookEx(hook);
        })?;

    if ready_receiver.recv().unwrap_or(false) {
        Ok(())
    } else {
        bail!("安装全局鼠标钩子失败")
    }
}

pub fn update_ui_hwnd(hwnd: isize) {
    if let Some(context) = CONTEXT.get() {
        context.ui_hwnd.store(hwnd, Ordering::Release);
    }
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
    if event.dwExtraInfo == INJECTED_EVENT_TOKEN {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }

    let message = wparam as u32;
    let event_button = trigger_for_event(message, event.mouseData);
    if is_trigger_up(message, event_button) && context.candidate_present.load(Ordering::Acquire) {
        let mut state = context.state.lock().expect("hook state poisoned");
        let Some(candidate) = state.candidate.take() else {
            context.candidate_present.store(false, Ordering::Release);
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        };
        if event_button != Some(candidate.button) {
            state.candidate = Some(candidate);
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        }
        context.candidate_present.store(false, Ordering::Release);
        drop(state);

        let committed = gesture_committed(
            candidate.path_length,
            candidate.max_distance_px,
            candidate.activation_distance_px,
            candidate.minimum_stroke_length_px,
        );
        if !committed {
            if candidate.active {
                post_simple(context, WM_APP_OVERLAY_END);
            }
            let _ = context
                .command_sender
                .send(HookCommand::Replay(candidate.button));
        } else if candidate.active {
            post_simple(context, WM_APP_OVERLAY_END);
            let _ = context
                .command_sender
                .send(HookCommand::Stroke(StrokeRequest {
                    points: candidate.points,
                    target_hwnd: candidate.target_hwnd,
                }));
        } else {
            let _ = context.command_sender.send(HookCommand::Cancelled);
        }
        return 1;
    }

    if message == WM_MOUSEMOVE {
        if !context.candidate_present.load(Ordering::Acquire) {
            return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
        }
        let mut state = context.state.lock().expect("hook state poisoned");
        let Some(candidate) = state.candidate.as_mut() else {
            context.candidate_present.store(false, Ordering::Release);
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
        if !candidate.active
            && candidate.started_at.elapsed().as_millis() >= candidate.activation_delay_ms as u128
            && candidate.max_distance_px >= candidate.activation_distance_px
        {
            candidate.active = true;
            candidate.last_overlay_post = Instant::now();
            post_stroke_begin(context, &candidate.points);
        }
        if candidate.active && candidate.last_overlay_post.elapsed().as_millis() >= 12 {
            candidate.last_overlay_post = Instant::now();
            post_point(context, WM_APP_OVERLAY_POINT, event.pt.x, event.pt.y);
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
        let config = context.config.read().expect("config poisoned");
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
        || context.candidate_present.load(Ordering::Acquire)
    {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }

    let point = event.pt;
    let target = target_window(point);
    if target.is_null() || is_gesture_blocked(target, &guard) {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }
    let dpi = unsafe { GetDpiForWindow(target) }.max(96);
    let activation_distance_px = activation_distance_dip * dpi as f32 / 96.0;
    let minimum_stroke_length_px = minimum_stroke_length_dip * dpi as f32 / 96.0;
    let now = Instant::now();
    let mut state = context.state.lock().expect("hook state poisoned");
    if state.candidate.is_some() {
        return unsafe { CallNextHookEx(ptr::null_mut(), code, wparam, lparam) };
    }
    state.candidate = Some(Candidate {
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
    });
    context.candidate_present.store(true, Ordering::Release);
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

    rect_covers_monitor(window_rect, monitor_info.rcMonitor, 8)
}

fn rect_covers_monitor(window: RECT, monitor: RECT, tolerance: i32) -> bool {
    window.left <= monitor.left + tolerance
        && window.top <= monitor.top + tolerance
        && window.right >= monitor.right - tolerance
        && window.bottom >= monitor.bottom - tolerance
}

fn post_point(context: &HookContext, message: u32, x: i32, y: i32) {
    let hwnd = context.ui_hwnd.load(Ordering::Acquire) as HWND;
    if hwnd.is_null() {
        return;
    }
    let point = Box::new(UiPoint { x, y });
    let pointer = Box::into_raw(point);
    if unsafe { PostMessageW(hwnd, message, 0, pointer as LPARAM) } == 0 {
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

fn post_stroke_begin(context: &HookContext, points: &[GesturePoint]) {
    let hwnd = context.ui_hwnd.load(Ordering::Acquire) as HWND;
    if hwnd.is_null() {
        return;
    }
    let begin = Box::new(UiStrokeBegin {
        points: stroke_ui_points(points),
    });
    let pointer = Box::into_raw(begin);
    if unsafe { PostMessageW(hwnd, WM_APP_OVERLAY_BEGIN, 0, pointer as LPARAM) } == 0 {
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

fn post_simple(context: &HookContext, message: u32) {
    let hwnd = context.ui_hwnd.load(Ordering::Acquire) as HWND;
    if !hwnd.is_null() {
        unsafe {
            PostMessageW(hwnd, message, 0, 0);
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

#[cfg(test)]
mod tests {
    use super::{
        GesturePoint, RECT, UiPoint, gesture_committed, rect_covers_monitor, stroke_ui_points,
    };

    #[test]
    fn short_or_jittery_drag_is_not_committed() {
        assert!(!gesture_committed(20.0, 18.0, 12.0, 28.0));
        assert!(!gesture_committed(80.0, 8.0, 12.0, 28.0));
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
}
