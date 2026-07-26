use super::theme::{ACCENT_COLOR, Palette};
use crate::gesture::{GestureAction, Point};
use std::{ffi::c_void, ptr};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{
        CreatePen, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
        HFONT, LineTo, MoveToEx, PS_DASH, PS_SOLID, RoundRect, SelectObject, SetBkMode,
        SetTextColor, TRANSPARENT,
    },
};

pub const CANVAS_RECT: RECT = RECT {
    left: 232,
    top: 256,
    right: 832,
    bottom: 546,
};

pub fn contains(x: i32, y: i32) -> bool {
    (CANVAS_RECT.left..CANVAS_RECT.right).contains(&x)
        && (CANVAS_RECT.top..CANVAS_RECT.bottom).contains(&y)
}

pub fn draw(
    hdc: *mut c_void,
    colors: Palette,
    font: HFONT,
    action: GestureAction,
    sample_count: usize,
    points: &[Point],
    drawing: bool,
) {
    let fill = unsafe { CreateSolidBrush(colors.page) };
    let border = unsafe { CreatePen(PS_SOLID, 1, colors.border) };
    let old_fill = unsafe { SelectObject(hdc, fill) };
    let old_pen = unsafe { SelectObject(hdc, border) };
    unsafe {
        RoundRect(
            hdc,
            CANVAS_RECT.left,
            CANVAS_RECT.top,
            CANVAS_RECT.right,
            CANVAS_RECT.bottom,
            16,
            16,
        );
        SelectObject(hdc, old_fill);
        SelectObject(hdc, old_pen);
        DeleteObject(fill);
        DeleteObject(border);
        SetBkMode(hdc, TRANSPARENT as i32);
    }

    let guide = unsafe { CreatePen(PS_DASH, 1, colors.border) };
    let old_guide = unsafe { SelectObject(hdc, guide) };
    unsafe {
        MoveToEx(
            hdc,
            CANVAS_RECT.left + 24,
            (CANVAS_RECT.top + CANVAS_RECT.bottom) / 2,
            ptr::null_mut(),
        );
        LineTo(
            hdc,
            CANVAS_RECT.right - 24,
            (CANVAS_RECT.top + CANVAS_RECT.bottom) / 2,
        );
        MoveToEx(
            hdc,
            (CANVAS_RECT.left + CANVAS_RECT.right) / 2,
            CANVAS_RECT.top + 24,
            ptr::null_mut(),
        );
        LineTo(
            hdc,
            (CANVAS_RECT.left + CANVAS_RECT.right) / 2,
            CANVAS_RECT.bottom - 24,
        );
        SelectObject(hdc, old_guide);
        DeleteObject(guide);
    }

    if points.len() >= 2 {
        let trail = unsafe { CreatePen(PS_SOLID, 4, ACCENT_COLOR) };
        let old_trail = unsafe { SelectObject(hdc, trail) };
        unsafe {
            MoveToEx(hdc, points[0].x as i32, points[0].y as i32, ptr::null_mut());
            for point in &points[1..] {
                LineTo(hdc, point.x as i32, point.y as i32);
            }
            SelectObject(hdc, old_trail);
            DeleteObject(trail);
        }
    }

    let message = if drawing {
        format!("正在记录 {}", action.short_label())
    } else if points.is_empty() {
        format!(
            "按住左键，按你的习惯绘制 {}（已学习 {sample_count}/3）",
            action.short_label()
        )
    } else {
        "松开后已保存；继续绘制可再添加一份样本".to_owned()
    };
    let mut text_rect = RECT {
        left: CANVAS_RECT.left + 24,
        top: CANVAS_RECT.bottom - 48,
        right: CANVAS_RECT.right - 24,
        bottom: CANVAS_RECT.bottom - 14,
    };
    let text = wide(&message);
    let old_font = unsafe { SelectObject(hdc, font) };
    unsafe {
        SetTextColor(hdc, colors.muted);
        DrawTextW(
            hdc,
            text.as_ptr(),
            -1,
            &mut text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, old_font);
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
