use std::ptr;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, WPARAM},
    Graphics::Gdi::{CreateRoundRectRgn, HFONT, SetWindowRgn},
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CreateWindowExW, HMENU, SendMessageW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT, WS_CHILD,
        WS_VISIBLE,
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
        control
    }

    pub fn label(&self, text: &str, x: i32, y: i32, width: i32, height: i32) -> HWND {
        self.control(
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
