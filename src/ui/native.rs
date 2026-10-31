use std::{mem, ptr};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{BeginPaint, CreateRoundRectRgn, EndPaint, HFONT, PAINTSTRUCT, SetWindowRgn},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Accessibility::NotifyWinEvent,
        Controls::{
            BST_CHECKED, DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_NOFOCUSRECT, ODS_SELECTED,
            ODT_BUTTON,
        },
        Input::KeyboardAndMouse::{GetFocus, IsWindowEnabled},
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            BM_GETCHECK, BM_GETSTATE, BM_SETCHECK, BS_CHECKBOX, BS_RADIOBUTTON, BS_TYPEMASK,
            BST_PUSHED, CreateWindowExW, EVENT_OBJECT_STATECHANGE, GetClientRect, GetDlgCtrlID,
            GetParent, HMENU, OBJID_CLIENT, SendMessageW, UISF_HIDEFOCUS, WINDOW_EX_STYLE,
            WINDOW_STYLE, WM_DRAWITEM, WM_ERASEBKGND, WM_NCDESTROY, WM_PAINT, WM_QUERYUISTATE,
            WM_SETFONT, WS_CHILD, WS_GROUP, WS_VISIBLE,
        },
    },
};

const SS_LEFT_STYLE: u32 = 0;

pub struct ControlFactory {
    parent: HWND,
    font: HFONT,
}

impl ControlFactory {
    pub fn new(parent: HWND, font: HFONT) -> Self {
        Self { parent, font }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn control(
        &self,
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
        let semantic = class.eq_ignore_ascii_case("BUTTON")
            && matches!(style & BS_TYPEMASK as u32, value if value == BS_CHECKBOX as u32 || value == BS_RADIOBUTTON as u32);
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
                self.parent,
                id as usize as HMENU,
                instance,
                ptr::null(),
            )
        };
        set_control_font(control, self.font);
        if semantic && !control.is_null() {
            unsafe {
                SetWindowSubclass(control, Some(semantic_button_proc), 1, 0);
            }
        }
        control
    }

    pub fn label(&self, text: &str, x: i32, y: i32, width: i32, height: i32) -> HWND {
        self.control(
            "STATIC",
            text,
            WS_CHILD | WS_VISIBLE | WS_GROUP | SS_LEFT_STYLE,
            0,
            x,
            y,
            width,
            height,
            0,
        )
    }
}

pub fn set_checked(hwnd: HWND, checked: bool) {
    if !hwnd.is_null() {
        let previous = is_checked(hwnd);
        unsafe {
            SendMessageW(
                hwnd,
                BM_SETCHECK,
                if checked { BST_CHECKED as usize } else { 0 },
                0,
            );
            if previous != checked {
                NotifyWinEvent(EVENT_OBJECT_STATECHANGE, hwnd, OBJID_CLIENT, 0);
            }
        }
    }
}

pub fn is_checked(hwnd: HWND) -> bool {
    !hwnd.is_null() && unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) } == BST_CHECKED as isize
}

pub fn set_accessible_name(hwnd: HWND, name: &str) {
    use windows::{
        Win32::{
            Foundation::HWND as WinHwnd,
            System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
            UI::Accessibility::{CAccPropServices, IAccPropServices, PROPID_ACC_NAME},
        },
        core::PCWSTR,
    };
    let name = wide(name);
    unsafe {
        if let Ok(services) =
            CoCreateInstance::<_, IAccPropServices>(&CAccPropServices, None, CLSCTX_INPROC_SERVER)
        {
            let _ = services.SetHwndPropStr(
                WinHwnd(hwnd),
                OBJID_CLIENT as u32,
                0,
                PROPID_ACC_NAME,
                PCWSTR(name.as_ptr()),
            );
        }
    }
}

unsafe extern "system" fn semantic_button_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
            let mut draw: DRAWITEMSTRUCT = unsafe { mem::zeroed() };
            draw.CtlType = ODT_BUTTON;
            draw.CtlID = unsafe { GetDlgCtrlID(hwnd) } as u32;
            draw.hwndItem = hwnd;
            draw.hDC = hdc;
            if unsafe { GetFocus() } == hwnd {
                draw.itemState |= ODS_FOCUS;
            }
            if unsafe { IsWindowEnabled(hwnd) } == 0 {
                draw.itemState |= ODS_DISABLED;
            }
            if unsafe { SendMessageW(hwnd, BM_GETSTATE, 0, 0) } & BST_PUSHED as isize != 0 {
                draw.itemState |= ODS_SELECTED;
            }
            if unsafe { SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) } & UISF_HIDEFOCUS as isize != 0 {
                draw.itemState |= ODS_NOFOCUSRECT;
            }
            unsafe {
                GetClientRect(hwnd, &mut draw.rcItem);
                SendMessageW(
                    GetParent(hwnd),
                    WM_DRAWITEM,
                    draw.CtlID as usize,
                    &draw as *const _ as LPARAM,
                );
                EndPaint(hwnd, &paint);
            }
            0
        }
        WM_NCDESTROY => {
            unsafe {
                RemoveWindowSubclass(hwnd, Some(semantic_button_proc), 1);
            }
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }
        _ => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    }
}

pub fn set_control_font(control: HWND, font: HFONT) {
    if !control.is_null() && !font.is_null() {
        unsafe {
            SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
        }
    }
}

pub fn round_control(control: HWND, width: i32, height: i32, radius: i32) {
    if control.is_null() {
        return;
    }
    let region = unsafe { CreateRoundRectRgn(0, 0, width + 1, height + 1, radius * 2, radius * 2) };
    unsafe {
        SetWindowRgn(control, region, 1);
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BS_OWNERDRAW, DestroyWindow, GWL_STYLE, GetNextDlgTabItem, GetWindowLongPtrW, SW_HIDE,
        ShowWindow, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP, WS_TABSTOP,
    };

    #[test]
    fn semantic_controls_keep_native_check_state_and_skip_hidden_tab_targets() {
        let class = wide("STATIC");
        let parent = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                ptr::null(),
                WS_POPUP | WS_VISIBLE,
                -32000,
                -32000,
                1,
                1,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        assert!(!parent.is_null());
        let factory = ControlFactory::new(parent, ptr::null_mut());
        let check = factory.control(
            "BUTTON",
            "记录历史",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_CHECKBOX as u32,
            0,
            0,
            0,
            1,
            1,
            1,
        );
        let radio = factory.control(
            "BUTTON",
            "右键",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_RADIOBUTTON as u32,
            0,
            0,
            0,
            1,
            1,
            2,
        );
        let button = factory.control(
            "BUTTON",
            "保存",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            0,
            0,
            0,
            1,
            1,
            3,
        );
        set_checked(check, true);
        set_checked(radio, true);
        let checked = is_checked(check) && is_checked(radio);
        let semantic_type =
            unsafe { GetWindowLongPtrW(check, GWL_STYLE) } as u32 & BS_TYPEMASK as u32;
        let first_next = unsafe { GetNextDlgTabItem(parent, check, 0) };
        unsafe {
            ShowWindow(radio, SW_HIDE);
        }
        let after_hide = unsafe { GetNextDlgTabItem(parent, check, 0) };
        set_checked(check, false);
        let unchecked = !is_checked(check);
        unsafe {
            DestroyWindow(parent);
        }
        assert!(checked && unchecked);
        assert_eq!(semantic_type, BS_CHECKBOX as u32);
        assert_eq!(first_next, radio);
        assert_eq!(after_hide, button);
    }
}
