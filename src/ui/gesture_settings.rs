use super::native::{ControlFactory, set_control_font};
use std::ptr;
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Gdi::HFONT,
    UI::WindowsAndMessaging::{BS_OWNERDRAW, WS_CHILD, WS_TABSTOP, WS_VISIBLE},
};

pub const IDC_GESTURE_UP: i32 = 1027;
pub const IDC_GESTURE_L: i32 = 1028;
pub const IDC_GESTURE_S: i32 = 1029;
pub const IDC_GESTURE_C: i32 = 1030;
pub const IDC_GESTURE_V: i32 = 1031;
pub const IDC_GESTURE_CLEAR: i32 = 1032;
pub const IDC_GESTURE_LEFT: i32 = 1035;
pub const IDC_GESTURE_RIGHT: i32 = 1036;
pub const IDC_GESTURE_BINDING: i32 = 1039;
pub const IDC_GESTURE_SEVEN: i32 = 1040;
pub const IDC_GESTURE_CIRCLE: i32 = 1041;

pub struct Controls {
    pub up: HWND,
    pub letter_l: HWND,
    pub letter_s: HWND,
    pub letter_c: HWND,
    pub letter_v: HWND,
    pub left: HWND,
    pub right: HWND,
    pub seven: HWND,
    pub circle: HWND,
    pub binding: HWND,
    pub clear: HWND,
    pub status: HWND,
    pub page: Vec<HWND>,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            up: ptr::null_mut(),
            letter_l: ptr::null_mut(),
            letter_s: ptr::null_mut(),
            letter_c: ptr::null_mut(),
            letter_v: ptr::null_mut(),
            left: ptr::null_mut(),
            right: ptr::null_mut(),
            seven: ptr::null_mut(),
            circle: ptr::null_mut(),
            binding: ptr::null_mut(),
            clear: ptr::null_mut(),
            status: ptr::null_mut(),
            page: Vec::new(),
        }
    }
}

pub fn create_controls(hwnd: HWND, body_font: HFONT, section_font: HFONT) -> Controls {
    let builder = ControlFactory::new(hwnd, body_font);
    let mut page = Vec::new();
    let title = builder.label("选择手势轨迹", 232, 116, 260, 26);
    set_control_font(title, section_font);
    page.push(title);

    let up = choice(&builder, "↑ 上划", 232, 154, IDC_GESTURE_UP);
    let letter_l = choice(&builder, "L 字形", 354, 154, IDC_GESTURE_L);
    let letter_s = choice(&builder, "S 字形", 476, 154, IDC_GESTURE_S);
    let letter_c = choice(&builder, "C 字形", 598, 154, IDC_GESTURE_C);
    let letter_v = choice(&builder, "V 字形", 720, 154, IDC_GESTURE_V);
    let left = choice(&builder, "← 左划", 232, 202, IDC_GESTURE_LEFT);
    let right = choice(&builder, "→ 右划", 354, 202, IDC_GESTURE_RIGHT);
    let seven = choice(&builder, "7 字形", 476, 202, IDC_GESTURE_SEVEN);
    let circle = choice(&builder, "○ 圆形", 598, 202, IDC_GESTURE_CIRCLE);
    page.extend([
        up, letter_l, letter_s, letter_c, letter_v, left, right, seven, circle,
    ]);

    let status = builder.label("", 232, 244, 590, 22);
    page.push(status);
    let binding = builder.control(
        "BUTTON",
        "执行动作",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        282,
        330,
        36,
        IDC_GESTURE_BINDING,
    );
    page.push(binding);
    let clear = builder.control(
        "BUTTON",
        "清除当前轨迹样本",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        578,
        174,
        40,
        IDC_GESTURE_CLEAR,
    );
    page.push(clear);

    Controls {
        up,
        letter_l,
        letter_s,
        letter_c,
        letter_v,
        left,
        right,
        seven,
        circle,
        binding,
        clear,
        status,
        page,
    }
}

fn choice(builder: &ControlFactory, text: &str, x: i32, y: i32, id: i32) -> HWND {
    builder.control(
        "BUTTON",
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        x,
        y,
        112,
        40,
        id,
    )
}
