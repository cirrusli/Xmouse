use super::theme::{ACCENT_COLOR, Palette};
use crate::stats::Snapshot;
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{
        CreateSolidBrush, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteObject,
        DrawFocusRect, DrawTextW, FillRect, HFONT, SelectObject, SetBkMode, SetTextColor,
        TRANSPARENT,
    },
    UI::Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_SELECTED},
};

pub fn draw_rank(draw: &DRAWITEMSTRUCT, colors: Palette, font: HFONT, snapshot: &Snapshot) {
    let Some(rank) = snapshot.ranking.get(draw.itemID as usize) else {
        return;
    };
    let background = if draw.itemState & ODS_SELECTED != 0 {
        colors.selected
    } else {
        colors.card
    };
    let brush = unsafe { CreateSolidBrush(background) };
    unsafe {
        FillRect(draw.hDC, &draw.rcItem, brush);
        DeleteObject(brush);
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(draw.hDC, colors.text);
    }
    let old = unsafe { SelectObject(draw.hDC, font) };
    let mut label = RECT {
        left: draw.rcItem.left + 10,
        top: draw.rcItem.top,
        right: draw.rcItem.left + 152,
        bottom: draw.rcItem.bottom,
    };
    let text = format!("{:02}  {}", draw.itemID + 1, rank.gesture.short_label());
    let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        DrawTextW(
            draw.hDC,
            text.as_ptr(),
            -1,
            &mut label,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }
    let max = snapshot
        .ranking
        .first()
        .map(|item| item.count)
        .unwrap_or(1)
        .max(1);
    let bar = RECT {
        left: label.right + 6,
        right: draw.rcItem.right - 85,
        top: draw.rcItem.top + 10,
        bottom: draw.rcItem.bottom - 10,
    };
    let track = unsafe { CreateSolidBrush(colors.hover) };
    let fill = unsafe { CreateSolidBrush(ACCENT_COLOR) };
    let filled = RECT {
        right: bar.left + ((bar.right - bar.left).max(0) as u64 * rank.count / max) as i32,
        ..bar
    };
    unsafe {
        FillRect(draw.hDC, &bar, track);
        FillRect(draw.hDC, &filled, fill);
        DeleteObject(track);
        DeleteObject(fill);
    }
    let mut count = RECT {
        left: bar.right + 8,
        right: draw.rcItem.right - 12,
        ..label
    };
    let text: Vec<u16> = rank
        .count
        .to_string()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        DrawTextW(
            draw.hDC,
            text.as_ptr(),
            -1,
            &mut count,
            DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(draw.hDC, old);
        if draw.itemState & ODS_FOCUS != 0 {
            DrawFocusRect(draw.hDC, &draw.rcItem);
        }
    }
}
