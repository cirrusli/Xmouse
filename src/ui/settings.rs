use super::{
    native::{ControlFactory, set_control_font},
    theme::{ACCENT_COLOR, Palette, rgb},
};
use std::{ffi::c_void, ptr};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::{
        CreatePen, CreateSolidBrush, DT_CENTER, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DeleteObject,
        DrawTextW, HFONT, PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::WindowsAndMessaging::{BS_OWNERDRAW, WS_CHILD, WS_TABSTOP, WS_VISIBLE},
};

pub const IDC_ENABLED: i32 = 1001;
pub const IDC_AUTOSTART: i32 = 1008;
pub const IDC_CAPTURE: i32 = 1009;
pub const IDC_SAVE: i32 = 1013;
pub const IDC_OPEN_HISTORY: i32 = 1014;
pub const IDC_CLEAR_HISTORY: i32 = 1015;
pub const IDC_STATUS: i32 = 1016;
pub const IDC_ENCRYPT_CONTENT: i32 = 1017;
pub const IDC_NAV_GENERAL: i32 = 1018;
pub const IDC_NAV_HISTORY: i32 = 1019;
pub const IDC_OPEN_DATA_DIR: i32 = 1020;
pub const IDC_DARK_MODE: i32 = 1021;
pub const IDC_TRIGGER_RIGHT: i32 = 1022;
pub const IDC_TRIGGER_X1: i32 = 1023;
pub const IDC_TRIGGER_X2: i32 = 1024;
pub const IDC_NAV_RESOURCES: i32 = 1025;
pub const IDC_NAV_GESTURES: i32 = 1026;
pub const IDC_GESTURE_TOPMOST: i32 = 1027;
pub const IDC_GESTURE_CLOSE: i32 = 1028;
pub const IDC_GESTURE_SEARCH: i32 = 1029;
pub const IDC_GESTURE_COPY: i32 = 1030;
pub const IDC_GESTURE_HISTORY: i32 = 1031;
pub const IDC_GESTURE_CLEAR: i32 = 1032;

const SS_RIGHT_STYLE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    General,
    History,
    Gestures,
    Resources,
}

#[derive(Clone, Copy)]
pub struct Fonts {
    pub body: HFONT,
    pub section: HFONT,
    pub title: HFONT,
}

pub struct Controls {
    pub save: HWND,
    pub status: HWND,
    pub page_title: HWND,
    pub page_subtitle: HWND,
    pub nav_general: HWND,
    pub nav_history: HWND,
    pub nav_gestures: HWND,
    pub nav_resources: HWND,
    pub enabled: HWND,
    pub dark_mode: HWND,
    pub trigger_right: HWND,
    pub trigger_x1: HWND,
    pub trigger_x2: HWND,
    pub autostart: HWND,
    pub capture: HWND,
    pub encrypt_content: HWND,
    pub history_usage: HWND,
    pub resource_cpu: HWND,
    pub resource_private: HWND,
    pub resource_working_set: HWND,
    pub resource_gpu: HWND,
    pub resource_details: HWND,
    pub gesture_topmost: HWND,
    pub gesture_close: HWND,
    pub gesture_search: HWND,
    pub gesture_copy: HWND,
    pub gesture_history: HWND,
    pub gesture_clear: HWND,
    pub gesture_status: HWND,
    pub general_page: Vec<HWND>,
    pub history_page: Vec<HWND>,
    pub gestures_page: Vec<HWND>,
    pub resources_page: Vec<HWND>,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            save: ptr::null_mut(),
            status: ptr::null_mut(),
            page_title: ptr::null_mut(),
            page_subtitle: ptr::null_mut(),
            nav_general: ptr::null_mut(),
            nav_history: ptr::null_mut(),
            nav_gestures: ptr::null_mut(),
            nav_resources: ptr::null_mut(),
            enabled: ptr::null_mut(),
            dark_mode: ptr::null_mut(),
            trigger_right: ptr::null_mut(),
            trigger_x1: ptr::null_mut(),
            trigger_x2: ptr::null_mut(),
            autostart: ptr::null_mut(),
            capture: ptr::null_mut(),
            encrypt_content: ptr::null_mut(),
            history_usage: ptr::null_mut(),
            resource_cpu: ptr::null_mut(),
            resource_private: ptr::null_mut(),
            resource_working_set: ptr::null_mut(),
            resource_gpu: ptr::null_mut(),
            resource_details: ptr::null_mut(),
            gesture_topmost: ptr::null_mut(),
            gesture_close: ptr::null_mut(),
            gesture_search: ptr::null_mut(),
            gesture_copy: ptr::null_mut(),
            gesture_history: ptr::null_mut(),
            gesture_clear: ptr::null_mut(),
            gesture_status: ptr::null_mut(),
            general_page: Vec::new(),
            history_page: Vec::new(),
            gestures_page: Vec::new(),
            resources_page: Vec::new(),
        }
    }
}
pub fn create_controls(hwnd: HWND, fonts: Fonts) -> Controls {
    let mut controls = Controls::default();
    let builder = ControlFactory::new(hwnd, fonts.body);

    controls.nav_general = builder.control(
        "BUTTON",
        "常规",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        14,
        104,
        162,
        42,
        IDC_NAV_GENERAL,
    );
    controls.nav_history = builder.control(
        "BUTTON",
        "剪贴板历史",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        14,
        154,
        162,
        42,
        IDC_NAV_HISTORY,
    );
    controls.nav_gestures = builder.control(
        "BUTTON",
        "个性化手势",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        14,
        204,
        162,
        42,
        IDC_NAV_GESTURES,
    );
    controls.nav_resources = builder.control(
        "BUTTON",
        "资源占用",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        14,
        254,
        162,
        42,
        IDC_NAV_RESOURCES,
    );
    controls.page_title = builder.label("常规", 220, 24, 300, 34);
    set_control_font(controls.page_title, fonts.title);
    controls.page_subtitle = builder.label("手势与启动", 221, 58, 440, 22);
    controls.status = builder.control(
        "BUTTON",
        "运行中",
        WS_CHILD | WS_VISIBLE | BS_OWNERDRAW as u32,
        0,
        758,
        27,
        104,
        32,
        IDC_STATUS,
    );

    let mut general_page = Vec::new();
    let mut history_page = Vec::new();
    let mut gestures_page = Vec::new();
    let mut resources_page = Vec::new();

    let startup_title = builder.label("基础", 232, 116, 160, 26);
    set_control_font(startup_title, fonts.section);
    general_page.push(startup_title);
    controls.enabled = builder.control(
        "BUTTON",
        "启用鼠标手势",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        150,
        270,
        34,
        IDC_ENABLED,
    );
    general_page.push(controls.enabled);
    controls.autostart = builder.control(
        "BUTTON",
        "开机自动启动",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        530,
        150,
        270,
        34,
        IDC_AUTOSTART,
    );
    general_page.push(controls.autostart);
    controls.dark_mode = builder.control(
        "BUTTON",
        "深色模式",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        194,
        270,
        34,
        IDC_DARK_MODE,
    );
    general_page.push(controls.dark_mode);

    let trigger_title = builder.label("触发键", 232, 276, 220, 26);
    set_control_font(trigger_title, fonts.section);
    general_page.push(trigger_title);
    controls.trigger_right = builder.control(
        "BUTTON",
        "鼠标右键",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        314,
        112,
        40,
        IDC_TRIGGER_RIGHT,
    );
    general_page.push(controls.trigger_right);
    controls.trigger_x1 = builder.control(
        "BUTTON",
        "侧键 X1",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        354,
        314,
        112,
        40,
        IDC_TRIGGER_X1,
    );
    general_page.push(controls.trigger_x1);
    controls.trigger_x2 = builder.control(
        "BUTTON",
        "侧键 X2",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        476,
        314,
        112,
        40,
        IDC_TRIGGER_X2,
    );
    general_page.push(controls.trigger_x2);
    general_page.push(builder.label("Edge 冲突时请使用侧键", 614, 323, 210, 22));

    let mapping_title = builder.label("快捷手势", 232, 410, 160, 26);
    set_control_font(mapping_title, fonts.section);
    general_page.push(mapping_title);
    general_page.push(builder.label(
        "↑  置顶窗口        L  关闭页面        S  搜索选中内容",
        232,
        452,
        590,
        26,
    ));
    general_page.push(builder.label("C  复制内容        V  打开剪贴板历史", 232, 492, 590, 24));

    controls.history_usage = builder.control(
        "STATIC",
        "",
        WS_CHILD | WS_VISIBLE | SS_RIGHT_STYLE,
        0,
        566,
        34,
        274,
        24,
        0,
    );
    history_page.push(controls.history_usage);

    let history_title = builder.label("记录", 232, 116, 160, 26);
    set_control_font(history_title, fonts.section);
    history_page.push(history_title);
    controls.capture = builder.control(
        "BUTTON",
        "记录文本和图片",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        150,
        270,
        34,
        IDC_CAPTURE,
    );
    history_page.push(controls.capture);
    controls.encrypt_content = builder.control(
        "BUTTON",
        "本机加密",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        530,
        150,
        270,
        34,
        IDC_ENCRYPT_CONTENT,
    );
    history_page.push(controls.encrypt_content);

    let actions_title = builder.label("管理", 232, 260, 160, 26);
    set_control_font(actions_title, fonts.section);
    history_page.push(actions_title);
    let open_history = builder.control(
        "BUTTON",
        "打开历史",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        304,
        128,
        40,
        IDC_OPEN_HISTORY,
    );
    history_page.push(open_history);
    let clear_history = builder.control(
        "BUTTON",
        "清空历史",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        372,
        304,
        128,
        40,
        IDC_CLEAR_HISTORY,
    );
    history_page.push(clear_history);
    let open_data = builder.control(
        "BUTTON",
        "打开数据目录",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        512,
        304,
        138,
        40,
        IDC_OPEN_DATA_DIR,
    );
    history_page.push(open_data);
    history_page.push(builder.label("按 V 可在光标附近快速打开历史", 232, 376, 420, 22));

    let gestures_title = builder.label("选择要学习的动作", 232, 116, 260, 26);
    set_control_font(gestures_title, fonts.section);
    gestures_page.push(gestures_title);
    controls.gesture_topmost = builder.control(
        "BUTTON",
        "↑ 置顶",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        154,
        112,
        40,
        IDC_GESTURE_TOPMOST,
    );
    controls.gesture_close = builder.control(
        "BUTTON",
        "L 关闭",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        354,
        154,
        112,
        40,
        IDC_GESTURE_CLOSE,
    );
    controls.gesture_search = builder.control(
        "BUTTON",
        "S 搜索",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        476,
        154,
        112,
        40,
        IDC_GESTURE_SEARCH,
    );
    controls.gesture_copy = builder.control(
        "BUTTON",
        "C 复制",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        598,
        154,
        112,
        40,
        IDC_GESTURE_COPY,
    );
    controls.gesture_history = builder.control(
        "BUTTON",
        "V 历史",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        720,
        154,
        112,
        40,
        IDC_GESTURE_HISTORY,
    );
    gestures_page.extend([
        controls.gesture_topmost,
        controls.gesture_close,
        controls.gesture_search,
        controls.gesture_copy,
        controls.gesture_history,
    ]);
    controls.gesture_status = builder.label("", 232, 202, 590, 22);
    gestures_page.push(controls.gesture_status);
    controls.gesture_clear = builder.control(
        "BUTTON",
        "清除当前动作样本",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        232,
        568,
        174,
        40,
        IDC_GESTURE_CLEAR,
    );
    gestures_page.push(controls.gesture_clear);

    let cpu_title = builder.label("CPU", 232, 124, 120, 24);
    resources_page.push(cpu_title);
    controls.resource_cpu = builder.label("0.00%", 232, 154, 250, 44);
    set_control_font(controls.resource_cpu, fonts.title);
    resources_page.push(controls.resource_cpu);

    let gpu_title = builder.label("GPU", 554, 124, 120, 24);
    resources_page.push(gpu_title);
    controls.resource_gpu = builder.label("0%", 554, 154, 250, 44);
    set_control_font(controls.resource_gpu, fonts.title);
    resources_page.push(controls.resource_gpu);
    resources_page.push(builder.label("GDI 软件绘制", 554, 202, 220, 22));

    let private_title = builder.label("私有内存", 232, 286, 160, 24);
    resources_page.push(private_title);
    controls.resource_private = builder.label("0 MiB", 232, 316, 250, 44);
    set_control_font(controls.resource_private, fonts.title);
    resources_page.push(controls.resource_private);

    let working_title = builder.label("工作集", 554, 286, 160, 24);
    resources_page.push(working_title);
    controls.resource_working_set = builder.label("0 MiB", 554, 316, 250, 44);
    set_control_font(controls.resource_working_set, fonts.title);
    resources_page.push(controls.resource_working_set);

    let details_title = builder.label("进程", 232, 452, 160, 24);
    set_control_font(details_title, fonts.section);
    resources_page.push(details_title);
    controls.resource_details = builder.label("", 232, 490, 580, 56);
    resources_page.push(controls.resource_details);

    controls.save = builder.control(
        "BUTTON",
        "保存设置",
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        0,
        734,
        625,
        128,
        40,
        IDC_SAVE,
    );

    controls.general_page = general_page;
    controls.history_page = history_page;
    controls.gestures_page = gestures_page;
    controls.resources_page = resources_page;
    controls
}

pub fn draw_sidebar_identity(hdc: *mut c_void, colors: Palette, fonts: Fonts) {
    let logo_brush = unsafe { CreateSolidBrush(ACCENT_COLOR) };
    let logo_pen = unsafe { CreatePen(PS_SOLID, 1, ACCENT_COLOR) };
    let old_brush = unsafe { SelectObject(hdc, logo_brush) };
    let old_pen = unsafe { SelectObject(hdc, logo_pen) };
    unsafe {
        RoundRect(hdc, 22, 24, 58, 60, 12, 12);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(logo_brush);
        DeleteObject(logo_pen);
        SetBkMode(hdc, TRANSPARENT as i32);
        SetTextColor(hdc, rgb(255, 255, 255));
    }
    let logo_font = unsafe { SelectObject(hdc, fonts.section) };
    let logo = wide("X");
    let mut logo_rect = RECT {
        left: 22,
        top: 24,
        right: 58,
        bottom: 60,
    };
    unsafe {
        DrawTextW(
            hdc,
            logo.as_ptr(),
            -1,
            &mut logo_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, logo_font);
        SetTextColor(hdc, colors.text);
    }

    let brand_font = unsafe { SelectObject(hdc, fonts.section) };
    let brand = wide("Xmouse");
    let mut brand_rect = RECT {
        left: 72,
        top: 22,
        right: 184,
        bottom: 50,
    };
    unsafe {
        DrawTextW(
            hdc,
            brand.as_ptr(),
            -1,
            &mut brand_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, brand_font);
    }

    let subtitle = wide("轻量效率工具");
    let version = wide(&format!("v{}", env!("CARGO_PKG_VERSION")));
    let mut subtitle_rect = RECT {
        left: 72,
        top: 50,
        right: 184,
        bottom: 76,
    };
    let mut version_rect = RECT {
        left: 22,
        top: 632,
        right: 150,
        bottom: 662,
    };
    let body_font = unsafe { SelectObject(hdc, fonts.body) };
    unsafe {
        DrawTextW(
            hdc,
            subtitle.as_ptr(),
            -1,
            &mut subtitle_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        DrawTextW(
            hdc,
            version.as_ptr(),
            -1,
            &mut version_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, body_font);
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
