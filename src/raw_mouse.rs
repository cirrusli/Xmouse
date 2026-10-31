use crate::{config::TriggerButton, hook};
use anyhow::{Result, bail};
use std::mem::size_of;
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND, LPARAM},
    UI::{
        Input::{
            GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
            RIDEV_INPUTSINK, RIM_TYPEMOUSE, RegisterRawInputDevices,
        },
        WindowsAndMessaging::{
            GetSystemMetrics, RI_MOUSE_BUTTON_4_UP, RI_MOUSE_BUTTON_5_UP, RI_MOUSE_LEFT_BUTTON_UP,
            RI_MOUSE_RIGHT_BUTTON_UP, SM_SWAPBUTTON,
        },
    },
};

pub fn register(hwnd: HWND) -> Result<()> {
    let device = RAWINPUTDEVICE {
        usUsagePage: 0x01,
        usUsage: 0x02,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };
    if unsafe { RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) } == 0 {
        bail!("注册原始鼠标释放监听失败，Windows 错误 {}", unsafe {
            GetLastError()
        });
    }
    Ok(())
}

pub fn handle_input(lparam: LPARAM, event_time: u32) {
    // No data parsing/allocation while idle or while applications are excluded.
    if hook::active_session_id() == 0 {
        return;
    }
    let mut input = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let read = unsafe {
        GetRawInputData(
            lparam as HRAWINPUT,
            RID_INPUT,
            &mut input as *mut RAWINPUT as *mut _,
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if read == u32::MAX
        || read < size_of::<RAWINPUT>() as u32
        || input.header.dwType != RIM_TYPEMOUSE
    {
        return;
    }
    let mouse = unsafe { input.data.mouse };
    if mouse.ulExtraInformation == hook::INJECTED_EVENT_TOKEN as u32 {
        return;
    }
    let flags = unsafe { mouse.Anonymous.Anonymous.usButtonFlags } as u32;
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    for button in released_buttons(flags, swapped).into_iter().flatten() {
        hook::note_physical_release(button, event_time);
    }
}

fn released_buttons(flags: u32, swapped: bool) -> [Option<TriggerButton>; 3] {
    let right_flag = if swapped {
        RI_MOUSE_LEFT_BUTTON_UP
    } else {
        RI_MOUSE_RIGHT_BUTTON_UP
    };
    [
        (flags & right_flag != 0).then_some(TriggerButton::Right),
        (flags & RI_MOUSE_BUTTON_4_UP != 0).then_some(TriggerButton::X1),
        (flags & RI_MOUSE_BUTTON_5_UP != 0).then_some(TriggerButton::X2),
    ]
}

#[cfg(test)]
mod tests {
    use super::released_buttons;
    use crate::config::TriggerButton;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        RI_MOUSE_BUTTON_4_UP, RI_MOUSE_BUTTON_5_UP, RI_MOUSE_LEFT_BUTTON_UP,
        RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP,
    };

    #[test]
    fn raw_releases_map_to_logical_trigger_buttons_without_treating_down_as_release() {
        assert_eq!(
            released_buttons(RI_MOUSE_RIGHT_BUTTON_DOWN, false),
            [None; 3]
        );
        assert_eq!(released_buttons(RI_MOUSE_RIGHT_BUTTON_UP, true), [None; 3]);
        assert_eq!(
            released_buttons(
                RI_MOUSE_LEFT_BUTTON_UP | RI_MOUSE_BUTTON_4_UP | RI_MOUSE_BUTTON_5_UP,
                true
            ),
            [
                Some(TriggerButton::Right),
                Some(TriggerButton::X1),
                Some(TriggerButton::X2)
            ]
        );
        assert_eq!(
            released_buttons(RI_MOUSE_RIGHT_BUTTON_UP, false)[0],
            Some(TriggerButton::Right)
        );
    }
}
