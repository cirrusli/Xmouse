use crate::{
    clipboard::ClipboardService,
    config::AppConfig,
    gesture::{GestureAction, Recognizer, UserGestureTemplate},
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
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL,
            VK_LEFT, VK_LWIN, VK_RIGHT,
        },
        Shell::ShellExecuteW,
        WindowsAndMessaging::{
            GA_ROOT, GWL_EXSTYLE, GetAncestor, GetForegroundWindow, GetWindowLongPtrW,
            HWND_NOTOPMOST, HWND_TOPMOST, IsWindow, PostMessageW, SW_SHOWNORMAL,
            SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow,
            SetWindowPos, WS_EX_TOPMOST,
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
                        execute_action(
                            matched.action,
                            stroke.target_hwnd,
                            &config,
                            &clipboard,
                            ui_hwnd,
                        )
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
    action: GestureAction,
    target_hwnd: isize,
    config: &Arc<RwLock<AppConfig>>,
    clipboard: &ClipboardService,
    ui_hwnd: isize,
) -> Result<()> {
    let target = target_hwnd as HWND;
    match action {
        GestureAction::ToggleTopmost => toggle_topmost(target, ui_hwnd),
        GestureAction::CloseTab => {
            activate_target(target)?;
            send_ctrl_key(b'W' as u16)?;
            post_toast(ui_hwnd, "已发送 Ctrl+W");
            Ok(())
        }
        GestureAction::CopySelection => {
            activate_target(target)?;
            send_ctrl_key(b'C' as u16)?;
            post_toast(ui_hwnd, "已复制");
            Ok(())
        }
        GestureAction::SearchSelection => {
            activate_target(target)?;
            search_selection(target, config, clipboard, ui_hwnd)
        }
        GestureAction::OpenHistory => {
            unsafe {
                PostMessageW(ui_hwnd as HWND, WM_APP_SHOW_HISTORY, 0, target_hwnd);
            }
            Ok(())
        }
        GestureAction::SwitchDesktopLeft => {
            send_virtual_desktop_switch(VK_LEFT)?;
            post_toast(ui_hwnd, "已切换到左侧桌面");
            Ok(())
        }
        GestureAction::SwitchDesktopRight => {
            send_virtual_desktop_switch(VK_RIGHT)?;
            post_toast(ui_hwnd, "已切换到右侧桌面");
            Ok(())
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
