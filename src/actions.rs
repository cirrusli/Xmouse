use crate::{
    action::ActionKind,
    clipboard::ClipboardService,
    config::AppConfig,
    gesture::{Recognizer, UserGestureTemplate},
    hook::{HookCommand, INJECTED_EVENT_TOKEN, WM_APP_SHOW_HISTORY, WM_APP_TOAST, replay_button},
    logging,
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
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
            VK_BROWSER_BACK, VK_BROWSER_FORWARD, VK_CONTROL, VK_LEFT, VK_LWIN, VK_MEDIA_PLAY_PAUSE,
            VK_RIGHT, VK_TAB, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
        },
        Shell::ShellExecuteW,
        WindowsAndMessaging::{
            GA_ROOT, GWL_EXSTYLE, GetAncestor, GetForegroundWindow, GetWindowLongPtrW,
            HWND_NOTOPMOST, HWND_TOPMOST, IsWindow, IsZoomed, PostMessageW, SW_MAXIMIZE,
            SW_MINIMIZE, SW_RESTORE, SW_SHOWNORMAL, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE,
            SWP_NOSIZE, SetForegroundWindow, SetWindowPos, ShowWindow, WS_EX_TOPMOST,
        },
    },
};

pub fn run_worker(
    receiver: Receiver<HookCommand>,
    config: Arc<RwLock<AppConfig>>,
    clipboard: ClipboardService,
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
                let result = match command {
                    HookCommand::Replay(button) => replay_button(button),
                    HookCommand::Cancelled => Ok(()),
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
                            post_toast(ui_hwnd, "未识别手势");
                            continue;
                        };
                        let _recognition_score = matched.score;
                        let action = config
                            .read()
                            .expect("config poisoned")
                            .action_for(matched.gesture);
                        execute_action(action, stroke.target_hwnd, &config, &clipboard, ui_hwnd)
                    }
                };
                if let Err(error) = result {
                    let detail = format!("{error:#}");
                    logging::error("执行手势", &detail);
                    post_toast(ui_hwnd, &format!("操作失败：{detail}"));
                }
            }
        })
        .expect("failed to spawn action worker");
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
        ActionKind::CloseTab => {
            activate_target(target)?;
            send_ctrl_key(b'W' as u16)?;
            post_toast(ui_hwnd, "已发送 Ctrl+W");
            Ok(())
        }
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
    let foreground_root = if foreground.is_null() {
        foreground
    } else {
        unsafe { GetAncestor(foreground, GA_ROOT) }
    };
    if foreground_root != target && unsafe { SetForegroundWindow(target) } == 0 {
        bail!("Windows 阻止了目标窗口激活");
    }
    thread::sleep(Duration::from_millis(30));
    let foreground = unsafe { GetForegroundWindow() };
    let foreground_root = if foreground.is_null() {
        foreground
    } else {
        unsafe { GetAncestor(foreground, GA_ROOT) }
    };
    if foreground_root != target {
        bail!("目标窗口未获得焦点");
    }
    Ok(())
}

fn send_ctrl_key(key: u16) -> Result<()> {
    let mut inputs = [
        keyboard_input(VK_CONTROL, 0),
        keyboard_input(key, 0),
        keyboard_input(key, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        bail!("SendInput 被系统或目标程序拒绝");
    }
    Ok(())
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
    let mut inputs = [
        keyboard_input(VK_LWIN, 0),
        keyboard_input(VK_CONTROL, 0),
        keyboard_input(direction, 0),
        keyboard_input(direction, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
        keyboard_input(VK_LWIN, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_mut_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        bail!("切换桌面的快捷键被系统拒绝");
    }
    Ok(())
}

fn keyboard_input(key: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: INJECTED_EVENT_TOKEN,
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
