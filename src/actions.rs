use crate::{
    action::ActionKind,
    clipboard::{ClipboardService, process_name},
    config::AppConfig,
    gesture::{Recognizer, UserGestureTemplate},
    hook::{HookCommand, WM_APP_SHOW_HISTORY, WM_APP_STATS_UPDATED, WM_APP_TOAST, replay_button},
    logging,
    stats::{GestureEvent, GestureStats, Outcome},
};
use anyhow::{Context, Result, bail};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::{
    ptr,
    sync::{Arc, RwLock, mpsc::Receiver},
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{
    System::{
        Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        Ole::OleInitialize,
    },
    UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
    },
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    System::DataExchange::GetClipboardSequenceNumber,
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
            KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW, SendInput, VK_BROWSER_BACK,
            VK_BROWSER_FORWARD, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_LWIN,
            VK_MEDIA_PLAY_PAUSE, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP, VK_VOLUME_DOWN, VK_VOLUME_MUTE,
            VK_VOLUME_UP,
        },
        Shell::ShellExecuteW,
        WindowsAndMessaging::{
            GA_ROOT, GA_ROOTOWNER, GWL_EXSTYLE, GetAncestor, GetForegroundWindow,
            GetWindowLongPtrW, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST, IsWindow,
            IsZoomed, PostMessageW, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOWNORMAL,
            SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow,
            SetWindowPos, ShowWindow, WM_CLOSE, WS_EX_TOPMOST,
        },
    },
};

pub fn run_worker(
    receiver: Receiver<HookCommand>,
    config: Arc<RwLock<AppConfig>>,
    clipboard: ClipboardService,
    stats: GestureStats,
    ui_hwnd: isize,
) {
    thread::Builder::new()
        .name("xmouse-actions".to_owned())
        .spawn(move || {
            unsafe {
                if let Err(error) = OleInitialize(None) {
                    logging::error("初始化 OLE", format!("{error:#}"));
                }
            }
            let mut recognizer = Recognizer::new();
            let mut loaded_user_templates: Vec<UserGestureTemplate> = Vec::new();
            while let Ok(command) = receiver.recv() {
                match command {
                    HookCommand::Replay {
                        button,
                        attempted_samples,
                    } => {
                        if let Err(error) = replay_button(button) {
                            logging::error("重放鼠标按键", &error);
                            post_toast(ui_hwnd, &format!("操作失败：{error:#}"));
                        }
                        if let Some(sample_count) = attempted_samples {
                            record_gesture(
                                &stats,
                                ui_hwnd,
                                GestureEvent {
                                    gesture: None,
                                    action: None,
                                    outcome: Outcome::TooShort,
                                    score: None,
                                    sample_count,
                                },
                            );
                        }
                    }
                    HookCommand::Cancelled { sample_count } => {
                        record_gesture(
                            &stats,
                            ui_hwnd,
                            GestureEvent {
                                gesture: None,
                                action: None,
                                outcome: Outcome::Cancelled,
                                score: None,
                                sample_count,
                            },
                        );
                    }
                    HookCommand::Stroke(stroke) => {
                        let (threshold, user_templates) = {
                            let config = config.read().expect("config poisoned");
                            (config.recognition_threshold, config.custom_gestures.clone())
                        };
                        if user_templates != loaded_user_templates {
                            recognizer.set_user_templates(&user_templates);
                            loaded_user_templates = user_templates;
                        }
                        let Some(matched) = recognizer.recognize(&stroke.points, threshold) else {
                            record_gesture(
                                &stats,
                                ui_hwnd,
                                GestureEvent {
                                    gesture: None,
                                    action: None,
                                    outcome: Outcome::Unrecognized,
                                    score: None,
                                    sample_count: stroke.points.len(),
                                },
                            );
                            post_toast(ui_hwnd, "未识别手势");
                            continue;
                        };
                        let action = config
                            .read()
                            .expect("config poisoned")
                            .action_for(matched.gesture);
                        let result = execute_action(
                            action,
                            stroke.target_hwnd,
                            &config,
                            &clipboard,
                            ui_hwnd,
                        );
                        let outcome = if action == ActionKind::Disabled {
                            Outcome::Disabled
                        } else if result.is_ok() {
                            Outcome::Success
                        } else {
                            Outcome::Failed
                        };
                        record_gesture(
                            &stats,
                            ui_hwnd,
                            GestureEvent {
                                gesture: Some(matched.gesture),
                                action: Some(action),
                                outcome,
                                score: Some(matched.score),
                                sample_count: stroke.points.len(),
                            },
                        );
                        if let Err(error) = result {
                            let detail = format!("{error:#}");
                            logging::error("执行手势", &detail);
                            post_toast(ui_hwnd, &format!("操作失败：{detail}"));
                        }
                    }
                }
            }
        })
        .expect("failed to spawn action worker");
}

fn record_gesture(stats: &GestureStats, ui_hwnd: isize, event: GestureEvent) {
    logging::info(
        "手势",
        format!(
            "轨迹={} 动作={} 结果={} 得分={} 采样点={}",
            event
                .gesture
                .map(|gesture| format!("{gesture:?}"))
                .unwrap_or_else(|| "none".to_owned()),
            event
                .action
                .map(|action| format!("{action:?}"))
                .unwrap_or_else(|| "none".to_owned()),
            event.outcome.key(),
            event
                .score
                .map(|score| format!("{score:.3}"))
                .unwrap_or_else(|| "-".to_owned()),
            event.sample_count,
        ),
    );
    match stats.record(event) {
        Ok(()) => unsafe {
            PostMessageW(ui_hwnd as HWND, WM_APP_STATS_UPDATED, 0, 0);
        },
        Err(error) => logging::error("手势统计", &error),
    }
}

fn execute_action(
    action: ActionKind,
    target_hwnd: isize,
    config: &Arc<RwLock<AppConfig>>,
    clipboard: &ClipboardService,
    ui_hwnd: isize,
) -> Result<()> {
    let target = target_hwnd as HWND;
    match action {
        ActionKind::Disabled => Ok(()),
        ActionKind::ToggleTopmost => toggle_topmost(target, ui_hwnd),
        ActionKind::CloseTab => close_tab_or_compatible_window(target, ui_hwnd),
        ActionKind::CopySelection => {
            activate_target(target)?;
            send_ctrl_key(b'C' as u16)?;
            post_toast(ui_hwnd, "已复制");
            Ok(())
        }
        ActionKind::SearchSelection => {
            activate_target(target)?;
            search_selection(target, config, clipboard, ui_hwnd)
        }
        ActionKind::OpenHistory => {
            unsafe {
                PostMessageW(ui_hwnd as HWND, WM_APP_SHOW_HISTORY, 0, target_hwnd);
            }
            Ok(())
        }
        ActionKind::SwitchDesktopLeft => {
            send_virtual_desktop_switch(VK_LEFT)?;
            post_toast(ui_hwnd, "已切换到左侧桌面");
            Ok(())
        }
        ActionKind::SwitchDesktopRight => {
            send_virtual_desktop_switch(VK_RIGHT)?;
            post_toast(ui_hwnd, "已切换到右侧桌面");
            Ok(())
        }
        ActionKind::Paste => {
            activate_target(target)?;
            send_ctrl_key(b'V' as u16)?;
            post_toast(ui_hwnd, "已粘贴");
            Ok(())
        }
        ActionKind::BrowserBack => send_target_key(target, VK_BROWSER_BACK, ui_hwnd, "已后退"),
        ActionKind::BrowserForward => {
            send_target_key(target, VK_BROWSER_FORWARD, ui_hwnd, "已前进")
        }
        ActionKind::MinimizeWindow => {
            set_window_show_state(target, SW_MINIMIZE, "已最小化", ui_hwnd)
        }
        ActionKind::MaximizeRestore => {
            let command = if unsafe { IsZoomed(target) } != 0 {
                SW_RESTORE
            } else {
                SW_MAXIMIZE
            };
            set_window_show_state(target, command, "已切换窗口大小", ui_hwnd)
        }
        ActionKind::ShowDesktop => {
            send_windows_key(b'D' as u16)?;
            post_toast(ui_hwnd, "已显示桌面");
            Ok(())
        }
        ActionKind::TaskView => {
            send_windows_key(VK_TAB)?;
            post_toast(ui_hwnd, "已打开任务视图");
            Ok(())
        }
        ActionKind::TaskManager => {
            send_task_manager_shortcut()?;
            post_toast(ui_hwnd, "已打开任务管理器");
            Ok(())
        }
        ActionKind::ScreenSnip => send_screen_snip_shortcut(),
        ActionKind::VolumeMute => send_global_key(VK_VOLUME_MUTE, ui_hwnd, "已切换静音"),
        ActionKind::VolumeDown => send_global_key(VK_VOLUME_DOWN, ui_hwnd, "已降低音量"),
        ActionKind::VolumeUp => send_global_key(VK_VOLUME_UP, ui_hwnd, "已提高音量"),
        ActionKind::MediaPlayPause => {
            send_global_key(VK_MEDIA_PLAY_PAUSE, ui_hwnd, "已切换播放状态")
        }
    }
}

fn toggle_topmost(target: HWND, ui_hwnd: isize) -> Result<()> {
    if target.is_null() || unsafe { IsWindow(target) } == 0 {
        bail!("目标窗口已经关闭");
    }
    let style = unsafe { GetWindowLongPtrW(target, GWL_EXSTYLE) } as u32;
    let currently_topmost = style & WS_EX_TOPMOST != 0;
    let insert_after = if currently_topmost {
        HWND_NOTOPMOST
    } else {
        HWND_TOPMOST
    };
    let success = unsafe {
        SetWindowPos(
            target,
            insert_after,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    };
    if success == 0 {
        bail!("无法修改窗口置顶状态");
    }
    post_toast(
        ui_hwnd,
        if currently_topmost {
            "已取消置顶"
        } else {
            "已置顶"
        },
    );
    Ok(())
}

fn activate_target(target: HWND) -> Result<()> {
    if target.is_null() || unsafe { IsWindow(target) } == 0 {
        bail!("目标窗口已经关闭");
    }
    let foreground = unsafe { GetForegroundWindow() };
    if windows_share_input_target(target, foreground) {
        return Ok(());
    }

    // SetForegroundWindow may report failure while a foreground transition is
    // already in flight. Verify the actual foreground window for a short,
    // bounded period instead of treating the return value as the final state.
    let activation_requested = unsafe { SetForegroundWindow(target) } != 0;
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
        let foreground = unsafe { GetForegroundWindow() };
        if windows_share_input_target(target, foreground) {
            return Ok(());
        }
    }

    let target_process = window_process_id(target).unwrap_or_default();
    let foreground = unsafe { GetForegroundWindow() };
    let foreground_process = window_process_id(foreground).unwrap_or_default();
    if activation_requested {
        bail!("目标窗口未获得焦点（目标 PID {target_process}，前台 PID {foreground_process}）");
    }
    bail!("Windows 阻止了目标窗口激活（目标 PID {target_process}，前台 PID {foreground_process}）")
}

fn close_tab_or_compatible_window(target: HWND, ui_hwnd: isize) -> Result<()> {
    if target.is_null() || unsafe { IsWindow(target) } == 0 {
        bail!("目标窗口已经关闭");
    }
    if let Some(close_target) = wallpaper_ui_close_target(target) {
        if unsafe { PostMessageW(close_target, WM_CLOSE, 0, 0) } == 0 {
            bail!("Wallpaper UI 拒绝了关闭请求");
        }
        post_toast(ui_hwnd, "已关闭 Wallpaper UI");
        return Ok(());
    }

    activate_target(target)?;
    send_ctrl_key(b'W' as u16)?;
    post_toast(ui_hwnd, "已发送 Ctrl+W");
    Ok(())
}

fn wallpaper_ui_close_target(target: HWND) -> Option<HWND> {
    let candidates = [
        window_ancestor_or_self(target, GA_ROOTOWNER),
        window_ancestor_or_self(target, GA_ROOT),
        target,
    ];
    candidates.into_iter().find(|candidate| {
        window_process_id(*candidate)
            .and_then(process_name)
            .is_some_and(|name| is_wallpaper_ui_process_name(&name))
    })
}

fn is_wallpaper_ui_process_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("wallpaperui.exe")
}

fn windows_share_input_target(target: HWND, foreground: HWND) -> bool {
    if target.is_null() || foreground.is_null() {
        return false;
    }
    if target == foreground {
        return true;
    }

    let target_root = window_ancestor_or_self(target, GA_ROOT);
    let foreground_root = window_ancestor_or_self(foreground, GA_ROOT);
    if target_root == foreground_root {
        return true;
    }

    let target_owner = window_ancestor_or_self(target, GA_ROOTOWNER);
    let foreground_owner = window_ancestor_or_self(foreground, GA_ROOTOWNER);
    if target_owner == foreground_owner {
        return true;
    }

    matches!(
        (window_process_id(target_root), window_process_id(foreground_root)),
        (Some(target_process), Some(foreground_process)) if target_process == foreground_process
    )
}

fn window_ancestor_or_self(window: HWND, flag: u32) -> HWND {
    let ancestor = unsafe { GetAncestor(window, flag) };
    if ancestor.is_null() { window } else { ancestor }
}

fn window_process_id(window: HWND) -> Option<u32> {
    if window.is_null() {
        return None;
    }
    let mut process_id = 0;
    unsafe {
        GetWindowThreadProcessId(window, &mut process_id);
    }
    (process_id != 0).then_some(process_id)
}

fn send_ctrl_key(key: u16) -> Result<()> {
    let mut inputs = ctrl_key_inputs(key);
    let delays_ms = [8, 24, 8];
    for (index, input) in inputs.iter_mut().enumerate() {
        let sent = unsafe { SendInput(1, input, std::mem::size_of::<INPUT>() as i32) };
        if sent != 1 {
            best_effort_release_ctrl_chord(key);
            bail!("SendInput 被系统或目标程序拒绝");
        }
        if let Some(delay_ms) = delays_ms.get(index) {
            thread::sleep(Duration::from_millis(*delay_ms));
        }
    }
    Ok(())
}

fn ctrl_key_inputs(key: u16) -> [INPUT; 4] {
    [
        keyboard_input(VK_CONTROL, 0),
        keyboard_input(key, 0),
        keyboard_input(key, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ]
}

fn best_effort_release_ctrl_chord(key: u16) {
    let mut releases = [
        keyboard_input(key, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    unsafe {
        SendInput(
            releases.len() as u32,
            releases.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        );
    }
}

pub fn paste_into_target(target_hwnd: isize) -> Result<()> {
    activate_target(target_hwnd as HWND)?;
    send_ctrl_key(b'V' as u16)
}

fn send_target_key(target: HWND, key: u16, ui_hwnd: isize, message: &str) -> Result<()> {
    activate_target(target)?;
    send_key(key)?;
    post_toast(ui_hwnd, message);
    Ok(())
}

fn send_global_key(key: u16, ui_hwnd: isize, message: &str) -> Result<()> {
    send_key(key)?;
    post_toast(ui_hwnd, message);
    Ok(())
}

fn send_key(key: u16) -> Result<()> {
    let mut inputs = [keyboard_input(key, 0), keyboard_input(key, KEYEVENTF_KEYUP)];
    send_inputs(&mut inputs, "快捷键被系统拒绝")
}

fn send_windows_key(key: u16) -> Result<()> {
    let mut inputs = [
        keyboard_input(VK_LWIN, 0),
        keyboard_input(key, 0),
        keyboard_input(key, KEYEVENTF_KEYUP),
        keyboard_input(VK_LWIN, KEYEVENTF_KEYUP),
    ];
    send_inputs(&mut inputs, "Windows 快捷键被系统拒绝")
}

fn send_task_manager_shortcut() -> Result<()> {
    let mut inputs = [
        keyboard_input(VK_CONTROL, 0),
        keyboard_input(VK_SHIFT, 0),
        keyboard_input(VK_ESCAPE, 0),
        keyboard_input(VK_ESCAPE, KEYEVENTF_KEYUP),
        keyboard_input(VK_SHIFT, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    send_inputs(&mut inputs, "任务管理器快捷键被系统拒绝")
}

fn send_screen_snip_shortcut() -> Result<()> {
    let mut inputs = [
        keyboard_input(VK_LWIN, 0),
        keyboard_input(VK_SHIFT, 0),
        keyboard_input(b'S' as u16, 0),
        keyboard_input(b'S' as u16, KEYEVENTF_KEYUP),
        keyboard_input(VK_SHIFT, KEYEVENTF_KEYUP),
        keyboard_input(VK_LWIN, KEYEVENTF_KEYUP),
    ];
    // Do not show an Xmouse toast here: a topmost toast could be captured by the snipping layer.
    send_inputs(&mut inputs, "截图快捷键被系统拒绝")
}

fn set_window_show_state(target: HWND, command: i32, message: &str, ui_hwnd: isize) -> Result<()> {
    if target.is_null() || unsafe { IsWindow(target) } == 0 {
        bail!("目标窗口已经关闭");
    }
    unsafe {
        ShowWindow(target, command);
    }
    post_toast(ui_hwnd, message);
    Ok(())
}

fn send_inputs(inputs: &mut [INPUT], error_message: &str) -> Result<()> {
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        bail!("{error_message}");
    }
    Ok(())
}

fn send_virtual_desktop_switch(direction: u16) -> Result<()> {
    let mut inputs = desktop_switch_inputs(direction);
    send_inputs(&mut inputs, "切换桌面的快捷键被系统拒绝")?;
    Ok(())
}

fn desktop_switch_inputs(direction: u16) -> [INPUT; 6] {
    [
        keyboard_input(VK_LWIN, 0),
        keyboard_input(VK_CONTROL, 0),
        keyboard_input(direction, 0),
        keyboard_input(direction, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
        keyboard_input(VK_LWIN, KEYEVENTF_KEYUP),
    ]
}

fn keyboard_input(key: u16, flags: u32) -> INPUT {
    let mapped_scan = unsafe { MapVirtualKeyW(key as u32, MAPVK_VK_TO_VSC_EX) };
    let (virtual_key, scan_code, scan_flags) = if mapped_scan == 0 {
        (key, 0, 0)
    } else {
        // Some keyboard layouts map VK_LEFT/RIGHT to the shared numpad scan
        // codes (0x4B/0x4D) without an E0 prefix. The dedicated arrow keys
        // still require KEYEVENTF_EXTENDEDKEY for shell shortcuts.
        let extended =
            if mapped_scan & 0xFF00 != 0 || matches!(key, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN) {
                KEYEVENTF_EXTENDEDKEY
            } else {
                0
            };
        (
            0,
            (mapped_scan & 0xFF) as u16,
            KEYEVENTF_SCANCODE | extended,
        )
    };
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: scan_code,
                dwFlags: flags | scan_flags,
                time: 0,
                // Keyboard events never enter Xmouse's mouse-only hook. Leaving
                // extra info at zero makes the sequence match physical input
                // more closely for Flutter and other framework key handlers.
                dwExtraInfo: 0,
            },
        },
    }
}

fn search_selection(
    target: HWND,
    config: &Arc<RwLock<AppConfig>>,
    clipboard: &ClipboardService,
    ui_hwnd: isize,
) -> Result<()> {
    let text = match selected_text_via_uia(target) {
        Ok(Some(text)) if !text.trim().is_empty() => Some(text),
        _ => selected_text_via_clipboard(clipboard)?.filter(|text| !text.trim().is_empty()),
    }
    .context("没有读取到选中文本")?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        bail!("没有选中文本");
    }
    let template = config
        .read()
        .expect("config poisoned")
        .search_url_template
        .clone();
    let encoded = utf8_percent_encode(trimmed, NON_ALPHANUMERIC).to_string();
    let url = template.replace("{query}", &encoded);
    let url_wide = wide(&url);
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            ptr::null(),
            url_wide.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        bail!("系统无法打开默认浏览器");
    }
    post_toast(ui_hwnd, "已搜索选中内容");
    Ok(())
}

fn selected_text_via_uia(_target: HWND) -> Result<Option<String>> {
    unsafe {
        let automation: IUIAutomation =
            CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .context("创建 UI Automation 失败")?;
        let focused = automation.GetFocusedElement()?;
        let pattern: IUIAutomationTextPattern = focused.GetCurrentPatternAs(UIA_TextPatternId)?;
        let ranges = pattern.GetSelection()?;
        if ranges.Length()? <= 0 {
            return Ok(None);
        }
        let range = ranges.GetElement(0)?;
        let text = range.GetText(8_192)?.to_string();
        Ok((!text.trim().is_empty()).then_some(text))
    }
}

fn selected_text_via_clipboard(clipboard: &ClipboardService) -> Result<Option<String>> {
    let snapshot = clipboard.snapshot_current().context("保存当前剪贴板失败")?;
    let _suspension = clipboard.suspend_capture();
    clipboard.ignore_next_updates(2);
    let capture_result = (|| {
        let before = unsafe { GetClipboardSequenceNumber() };
        send_ctrl_key(b'C' as u16)?;
        let deadline = Instant::now() + Duration::from_millis(1_500);
        let mut clipboard_changed = false;
        let mut last_read_error = None;
        while Instant::now() < deadline {
            clipboard_changed |= unsafe { GetClipboardSequenceNumber() } != before;
            if clipboard_changed {
                match clipboard.read_current_text() {
                    Ok(Some(text)) if !text.trim().is_empty() => return Ok(Some(text)),
                    Ok(_) => {}
                    Err(error) => last_read_error = Some(error),
                }
            }
            thread::sleep(Duration::from_millis(15));
        }
        if let Some(error) = last_read_error {
            return Err(error).context("剪贴板已更新，但文本仍不可读");
        }
        Ok(None)
    })();
    let restore_result = clipboard.restore_snapshot(&snapshot);
    thread::sleep(Duration::from_millis(80));
    clipboard.clear_ignored_updates();
    restore_result.context("恢复原剪贴板失败")?;
    capture_result
}

pub fn post_toast(ui_hwnd: isize, text: &str) {
    let boxed = Box::new(text.to_owned());
    let pointer = Box::into_raw(boxed);
    if unsafe { PostMessageW(ui_hwnd as HWND, WM_APP_TOAST, 0, pointer as LPARAM) } == 0 {
        unsafe {
            drop(Box::from_raw(pointer));
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ctrl_key_inputs, desktop_switch_inputs, is_wallpaper_ui_process_name, keyboard_input,
        windows_share_input_target,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, VK_LEFT, VK_LWIN, VK_RIGHT,
    };

    #[test]
    fn keyboard_shortcuts_use_unmarked_hardware_scan_codes() {
        let letter = keyboard_input(b'W' as u16, 0);
        let letter = unsafe { letter.Anonymous.ki };
        assert_eq!(letter.wVk, 0);
        assert_ne!(letter.wScan, 0);
        assert_ne!(letter.dwFlags & KEYEVENTF_SCANCODE, 0);
        assert_eq!(letter.dwExtraInfo, 0);

        let windows_key = keyboard_input(VK_LWIN, KEYEVENTF_KEYUP);
        let windows_key = unsafe { windows_key.Anonymous.ki };
        assert_ne!(windows_key.dwFlags & KEYEVENTF_SCANCODE, 0);
        assert_ne!(windows_key.dwFlags & KEYEVENTF_EXTENDEDKEY, 0);
        assert_ne!(windows_key.dwFlags & KEYEVENTF_KEYUP, 0);
        assert_eq!(windows_key.dwExtraInfo, 0);
    }

    #[test]
    fn ctrl_chord_keeps_modifier_pressed_until_the_key_is_released() {
        let inputs = ctrl_key_inputs(b'W' as u16);
        let keys = inputs.map(|input| unsafe { input.Anonymous.ki });

        assert_eq!(keys[0].wScan, keys[3].wScan);
        assert_eq!(keys[1].wScan, keys[2].wScan);
        assert_ne!(keys[0].wScan, keys[1].wScan);
        assert_eq!(keys[0].dwFlags & KEYEVENTF_KEYUP, 0);
        assert_eq!(keys[1].dwFlags & KEYEVENTF_KEYUP, 0);
        assert_ne!(keys[2].dwFlags & KEYEVENTF_KEYUP, 0);
        assert_ne!(keys[3].dwFlags & KEYEVENTF_KEYUP, 0);
    }

    #[test]
    fn desktop_switch_uses_extended_arrows_with_modifiers_held() {
        for direction in [VK_LEFT, VK_RIGHT] {
            let keys = desktop_switch_inputs(direction).map(|input| unsafe { input.Anonymous.ki });
            assert_eq!(keys[0].wScan, keys[5].wScan);
            assert_eq!(keys[1].wScan, keys[4].wScan);
            assert_eq!(keys[2].wScan, keys[3].wScan);
            let expected_scan =
                unsafe { super::MapVirtualKeyW(direction as u32, super::MAPVK_VK_TO_VSC_EX) };
            assert_ne!(expected_scan, 0);
            assert_eq!(keys[2].wScan, (expected_scan & 0xFF) as u16);
            assert_ne!(keys[2].dwFlags & KEYEVENTF_SCANCODE, 0);
            assert_ne!(keys[2].dwFlags & KEYEVENTF_EXTENDEDKEY, 0);
            assert_ne!(keys[3].dwFlags & KEYEVENTF_EXTENDEDKEY, 0);
            assert_eq!(keys[2].dwFlags & KEYEVENTF_KEYUP, 0);
            assert_ne!(keys[3].dwFlags & KEYEVENTF_KEYUP, 0);
            assert_eq!(keys[0].dwFlags & KEYEVENTF_KEYUP, 0);
            assert_eq!(keys[1].dwFlags & KEYEVENTF_KEYUP, 0);
            assert_ne!(keys[4].dwFlags & KEYEVENTF_KEYUP, 0);
            assert_ne!(keys[5].dwFlags & KEYEVENTF_KEYUP, 0);
            assert_eq!(keys[0].dwExtraInfo, 0);
            assert_eq!(keys[1].dwExtraInfo, 0);
        }
    }

    #[test]
    fn null_windows_never_share_an_input_target() {
        assert!(!windows_share_input_target(
            std::ptr::null_mut(),
            std::ptr::null_mut()
        ));
    }

    #[test]
    fn wallpaper_ui_compatibility_rule_is_process_specific() {
        assert!(is_wallpaper_ui_process_name("wallpaperui.exe"));
        assert!(is_wallpaper_ui_process_name("WallpaperUI.EXE"));
        assert!(!is_wallpaper_ui_process_name("wallpaper64.exe"));
        assert!(!is_wallpaper_ui_process_name("msedge.exe"));
    }
}
