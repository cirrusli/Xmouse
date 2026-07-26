use std::{ffi::c_void, mem, ptr};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND},
    Graphics::{
        Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute},
        Gdi::{CLEARTYPE_QUALITY, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, HFONT},
    },
    UI::Controls::SetWindowTheme,
};

pub const ACCENT_COLOR: COLORREF = rgb(37, 99, 235);

#[derive(Clone, Copy)]
pub struct Palette {
    pub page: COLORREF,
    pub card: COLORREF,
    pub border: COLORREF,
    pub text: COLORREF,
    pub muted: COLORREF,
    pub sidebar: COLORREF,
    pub hover: COLORREF,
    pub selected: COLORREF,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            page: rgb(18, 20, 24),
            card: rgb(28, 31, 37),
            border: rgb(52, 57, 66),
            text: rgb(239, 242, 247),
            muted: rgb(155, 163, 175),
            sidebar: rgb(22, 25, 30),
            hover: rgb(42, 47, 56),
            selected: rgb(32, 48, 76),
        }
    } else {
        Palette {
            page: rgb(246, 248, 251),
            card: rgb(255, 255, 255),
            border: rgb(225, 229, 236),
            text: rgb(31, 41, 55),
            muted: rgb(107, 114, 128),
            sidebar: rgb(243, 245, 248),
            hover: rgb(238, 242, 247),
            selected: rgb(239, 246, 255),
        }
    }
}

pub fn create_ui_font(height: i32, weight: i32) -> HFONT {
    let face = wide("Microsoft YaHei UI");
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

pub fn apply_window_theme(hwnd: HWND, dark: bool) {
    if hwnd.is_null() {
        return;
    }
    let enabled = i32::from(dark);
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &enabled as *const i32 as *const c_void,
            mem::size_of::<i32>() as u32,
        );
    }
}

pub fn apply_child_theme(hwnd: HWND, dark: bool) {
    if hwnd.is_null() {
        return;
    }
    let theme = wide(if dark {
        "DarkMode_Explorer"
    } else {
        "Explorer"
    });
    unsafe {
        SetWindowTheme(hwnd, theme.as_ptr(), ptr::null());
    }
}

pub const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
