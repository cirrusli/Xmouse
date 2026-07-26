use super::{
    native::{ControlFactory, round_control, set_control_font},
    settings::Fonts,
    theme::apply_child_theme,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::WindowsAndMessaging::{
        BS_OWNERDRAW, ES_AUTOHSCROLL, LB_SETITEMHEIGHT, LBS_HASSTRINGS, LBS_NOINTEGRALHEIGHT,
        LBS_NOTIFY, LBS_OWNERDRAWFIXED, SendMessageW, WS_CHILD, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
    },
};

pub const IDC_HISTORY_SEARCH: i32 = 1101;
pub const IDC_HISTORY_LIST: i32 = 1102;
pub const IDC_HISTORY_COPY: i32 = 1103;
pub const IDC_HISTORY_DELETE: i32 = 1104;
pub const IDC_HISTORY_CLEAR: i32 = 1105;
pub const IDC_HISTORY_PIN: i32 = 1106;

const SS_RIGHT_STYLE: u32 = 2;
const EM_SETCUEBANNER: u32 = 0x1501;

pub struct Controls {
    pub search: HWND,
    pub list: HWND,
    pub usage: HWND,
    pub pin: HWND,
    pub copy: HWND,
    pub delete: HWND,
    pub clear: HWND,
}

pub fn create_controls(hwnd: HWND, fonts: Fonts, dark: bool) -> Controls {
    let builder = ControlFactory::new(hwnd, fonts.body);
    let title = builder.label("剪贴板历史", 24, 16, 300, 42);
    set_control_font(title, fonts.title);
    builder.label("搜索并复制", 25, 60, 240, 24);
    let usage = builder.control(
        "STATIC",
        "",
        WS_CHILD | WS_VISIBLE | SS_RIGHT_STYLE,
        0,
        338,
        28,
        258,
        24,
        0,
    );
    let search = builder.control(
        "EDIT",
        "",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        0,
        24,
        98,
        572,
        38,
        IDC_HISTORY_SEARCH,
    );
    let list = builder.control(
        "LISTBOX",
        "",
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | LBS_NOTIFY as u32
            | LBS_NOINTEGRALHEIGHT as u32
            | LBS_OWNERDRAWFIXED as u32
            | LBS_HASSTRINGS as u32,
        0,
        24,
        150,
        572,
        326,
        IDC_HISTORY_LIST,
    );
    let pin = button(&builder, "置顶", 244, IDC_HISTORY_PIN);
    let copy = button(&builder, "复制", 334, IDC_HISTORY_COPY);
    let delete = button(&builder, "删除", 424, IDC_HISTORY_DELETE);
    let clear = button(&builder, "清空", 510, IDC_HISTORY_CLEAR);

    round_control(search, 572, 38, 16);
    round_control(list, 572, 326, 14);
    unsafe {
        SendMessageW(list, LB_SETITEMHEIGHT, 0, 64);
        let cue = wide("搜索剪贴板内容或来源程序");
        SendMessageW(search, EM_SETCUEBANNER, 1, cue.as_ptr() as LPARAM);
    }
    apply_child_theme(search, dark);
    apply_child_theme(list, dark);

    Controls {
        search,
        list,
        usage,
        pin,
        copy,
        delete,
        clear,
    }
}

fn button(builder: &ControlFactory, text: &str, x: i32, id: i32) -> HWND {
    builder.control(
        "BUTTON",
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        x,
        490,
        if id == IDC_HISTORY_DELETE { 78 } else { 82 },
        38,
        id,
    )
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
