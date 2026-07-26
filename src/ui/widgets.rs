use super::theme::{ACCENT_COLOR, Palette, rgb};
use std::ffi::c_void;
use windows_sys::Win32::{
    Foundation::{COLORREF, RECT},
    Graphics::Gdi::{
        CreatePen, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE,
        DT_VCENTER, DeleteObject, DrawTextW, Ellipse, FillRect, HFONT, PS_SOLID, RoundRect,
        SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_SELECTED},
        WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW},
    },
};

#[derive(Clone, Copy)]
pub enum ButtonRole {
    Status { enabled: bool },
    Navigation { active: bool },
    Primary,
    Danger,
    Secondary,
}

pub fn rounded_panel(
    hdc: *mut c_void,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    radius: i32,
    colors: Palette,
) {
    let brush = unsafe { CreateSolidBrush(colors.card) };
    let pen = unsafe { CreatePen(PS_SOLID, 1, colors.border) };
    let old_brush = unsafe { SelectObject(hdc, brush) };
    let old_pen = unsafe { SelectObject(hdc, pen) };
    unsafe {
        RoundRect(hdc, left, top, right, bottom, radius, radius);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
    }
}

pub fn draw_button(
    draw: &DRAWITEMSTRUCT,
    colors: Palette,
    font: HFONT,
    role: ButtonRole,
    corner_color: COLORREF,
) {
    let disabled = draw.itemState & ODS_DISABLED != 0;
    let pressed = !disabled && draw.itemState & ODS_SELECTED != 0;
    let corner_brush = unsafe { CreateSolidBrush(corner_color) };
    unsafe {
        FillRect(draw.hDC, &draw.rcItem, corner_brush);
        DeleteObject(corner_brush);
    }

    let (mut fill, mut border, mut text_color, radius) = match role {
        ButtonRole::Status { enabled: true } => {
            (rgb(231, 248, 239), rgb(191, 229, 207), rgb(22, 125, 74), 16)
        }
        ButtonRole::Status { enabled: false } => (
            rgb(255, 247, 225),
            rgb(244, 218, 151),
            rgb(164, 104, 16),
            16,
        ),
        ButtonRole::Navigation { active } => (
            if active {
                colors.selected
            } else if pressed {
                colors.hover
            } else {
                colors.sidebar
            },
            if active {
                colors.selected
            } else {
                colors.sidebar
            },
            if active { ACCENT_COLOR } else { colors.text },
            10,
        ),
        ButtonRole::Primary => (
            if pressed {
                rgb(29, 78, 216)
            } else {
                ACCENT_COLOR
            },
            if pressed {
                rgb(29, 78, 216)
            } else {
                ACCENT_COLOR
            },
            rgb(255, 255, 255),
            10,
        ),
        ButtonRole::Danger => (
            if pressed { colors.hover } else { colors.card },
            colors.border,
            rgb(190, 45, 55),
            10,
        ),
        ButtonRole::Secondary => (
            if pressed { colors.hover } else { colors.card },
            colors.border,
            colors.text,
            10,
        ),
    };
    if disabled {
        fill = colors.hover;
        border = colors.border;
        text_color = colors.muted;
    }

    let brush = unsafe { CreateSolidBrush(fill) };
    let pen = unsafe { CreatePen(PS_SOLID, 1, border) };
    let old_brush = unsafe { SelectObject(draw.hDC, brush) };
    let old_pen = unsafe { SelectObject(draw.hDC, pen) };
    unsafe {
        RoundRect(
            draw.hDC,
            draw.rcItem.left,
            draw.rcItem.top,
            draw.rcItem.right - 1,
            draw.rcItem.bottom - 1,
            radius,
            radius,
        );
        SelectObject(draw.hDC, old_brush);
        SelectObject(draw.hDC, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(draw.hDC, text_color);
    }

    if matches!(role, ButtonRole::Navigation { active: true }) {
        let accent = unsafe { CreateSolidBrush(ACCENT_COLOR) };
        let marker = RECT {
            left: draw.rcItem.left + 1,
            top: draw.rcItem.top + 9,
            right: draw.rcItem.left + 4,
            bottom: draw.rcItem.bottom - 9,
        };
        unsafe {
            FillRect(draw.hDC, &marker, accent);
            DeleteObject(accent);
        }
    }

    let text = wide(&window_text(draw.hwndItem));
    let mut text_rect = draw.rcItem;
    let text_flags = if matches!(role, ButtonRole::Navigation { .. }) {
        text_rect.left += 20;
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
    } else {
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS
    };
    if pressed && !matches!(role, ButtonRole::Status { .. }) {
        text_rect.top += 1;
    }
    let old_font = unsafe { SelectObject(draw.hDC, font) };
    unsafe {
        DrawTextW(draw.hDC, text.as_ptr(), -1, &mut text_rect, text_flags);
        SelectObject(draw.hDC, old_font);
    }
}

pub fn draw_toggle(draw: &DRAWITEMSTRUCT, colors: Palette, font: HFONT, checked: bool) {
    let background = unsafe { CreateSolidBrush(colors.card) };
    unsafe {
        FillRect(draw.hDC, &draw.rcItem, background);
        DeleteObject(background);
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(draw.hDC, colors.text);
    }

    let track = RECT {
        left: draw.rcItem.right - 48,
        top: draw.rcItem.top + (draw.rcItem.bottom - draw.rcItem.top - 22) / 2,
        right: draw.rcItem.right - 4,
        bottom: draw.rcItem.top + (draw.rcItem.bottom - draw.rcItem.top + 22) / 2,
    };
    let track_color = if checked { ACCENT_COLOR } else { colors.hover };
    let track_brush = unsafe { CreateSolidBrush(track_color) };
    let track_pen = unsafe {
        CreatePen(
            PS_SOLID,
            1,
            if checked { ACCENT_COLOR } else { colors.border },
        )
    };
    let old_brush = unsafe { SelectObject(draw.hDC, track_brush) };
    let old_pen = unsafe { SelectObject(draw.hDC, track_pen) };
    unsafe {
        RoundRect(
            draw.hDC,
            track.left,
            track.top,
            track.right,
            track.bottom,
            22,
            22,
        );
    }
    let thumb_left = if checked {
        track.right - 19
    } else {
        track.left + 3
    };
    let thumb_brush = unsafe {
        CreateSolidBrush(if checked {
            rgb(255, 255, 255)
        } else {
            colors.muted
        })
    };
    unsafe {
        SelectObject(draw.hDC, thumb_brush);
        Ellipse(
            draw.hDC,
            thumb_left,
            track.top + 3,
            thumb_left + 16,
            track.top + 19,
        );
        SelectObject(draw.hDC, old_brush);
        SelectObject(draw.hDC, old_pen);
        DeleteObject(thumb_brush);
        DeleteObject(track_brush);
        DeleteObject(track_pen);
    }

    let text = wide(&window_text(draw.hwndItem));
    let mut text_rect = RECT {
        left: draw.rcItem.left,
        top: draw.rcItem.top,
        right: track.left - 12,
        bottom: draw.rcItem.bottom,
    };
    let old_font = unsafe { SelectObject(draw.hDC, font) };
    unsafe {
        DrawTextW(
            draw.hDC,
            text.as_ptr(),
            -1,
            &mut text_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        SelectObject(draw.hDC, old_font);
    }
}

pub fn draw_choice(draw: &DRAWITEMSTRUCT, colors: Palette, font: HFONT, selected: bool) {
    let pressed = draw.itemState & ODS_SELECTED != 0;
    let fill = if selected {
        ACCENT_COLOR
    } else if pressed {
        colors.hover
    } else {
        colors.card
    };
    let border = if selected {
        ACCENT_COLOR
    } else {
        colors.border
    };
    let text_color = if selected {
        rgb(255, 255, 255)
    } else {
        colors.text
    };
    let corner_brush = unsafe { CreateSolidBrush(colors.card) };
    unsafe {
        FillRect(draw.hDC, &draw.rcItem, corner_brush);
        DeleteObject(corner_brush);
    }
    let brush = unsafe { CreateSolidBrush(fill) };
    let pen = unsafe { CreatePen(PS_SOLID, 1, border) };
    let old_brush = unsafe { SelectObject(draw.hDC, brush) };
    let old_pen = unsafe { SelectObject(draw.hDC, pen) };
    unsafe {
        RoundRect(
            draw.hDC,
            draw.rcItem.left,
            draw.rcItem.top,
            draw.rcItem.right - 1,
            draw.rcItem.bottom - 1,
            14,
            14,
        );
        SelectObject(draw.hDC, old_brush);
        SelectObject(draw.hDC, old_pen);
        DeleteObject(brush);
        DeleteObject(pen);
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(draw.hDC, text_color);
    }
    let text = wide(&window_text(draw.hwndItem));
    let mut rect = draw.rcItem;
    let old_font = unsafe { SelectObject(draw.hDC, font) };
    unsafe {
        DrawTextW(
            draw.hDC,
            text.as_ptr(),
            -1,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(draw.hDC, old_font);
    }
}

fn window_text(hwnd: *mut c_void) -> String {
    let length = unsafe { GetWindowTextLengthW(hwnd) };
    let mut buffer = vec![0u16; length.max(0) as usize + 1];
    let written = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..written.max(0) as usize])
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
