use super::{
    format::relative_history_time,
    theme::{ACCENT_COLOR, Palette},
};
use crate::storage::ClipItem;
use std::{ffi::c_void, mem};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{
        BITMAPINFO, BITMAPINFOHEADER, CreateSolidBrush, DIB_RGB_COLORS, DT_END_ELLIPSIS, DT_LEFT,
        DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, FillRect, HFONT, SRCCOPY,
        SelectObject, SetBkMode, SetTextColor, StretchDIBits, TRANSPARENT,
    },
    UI::Controls::{DRAWITEMSTRUCT, ODS_SELECTED},
};

pub struct Thumbnail {
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

pub struct HistoryView {
    pub item: ClipItem,
    thumbnail: Option<Thumbnail>,
}

impl HistoryView {
    pub fn new(item: ClipItem) -> Self {
        let thumbnail = item.thumbnail_png.as_deref().and_then(decode_thumbnail);
        Self { item, thumbnail }
    }
}

pub fn draw_history_item(draw: &DRAWITEMSTRUCT, view: &HistoryView, colors: Palette, font: HFONT) {
    let selected = draw.itemState & ODS_SELECTED != 0;
    let background_color = if selected {
        colors.selected
    } else {
        colors.card
    };
    let brush = unsafe { CreateSolidBrush(background_color) };
    unsafe {
        FillRect(draw.hDC, &draw.rcItem, brush);
        DeleteObject(brush);
        SetBkMode(draw.hDC, TRANSPARENT as i32);
        SetTextColor(draw.hDC, colors.text);
    }
    if selected {
        let accent = unsafe { CreateSolidBrush(ACCENT_COLOR) };
        let stripe = RECT {
            left: draw.rcItem.left,
            top: draw.rcItem.top + 6,
            right: draw.rcItem.left + 3,
            bottom: draw.rcItem.bottom - 6,
        };
        unsafe {
            FillRect(draw.hDC, &stripe, accent);
            DeleteObject(accent);
        }
    }

    let image_left = draw.rcItem.left + 10;
    let content_left = if let Some(thumbnail) = &view.thumbnail {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: thumbnail.width,
                biHeight: -thumbnail.height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [Default::default(); 1],
        };
        unsafe {
            StretchDIBits(
                draw.hDC,
                image_left,
                draw.rcItem.top + 8,
                48,
                48,
                0,
                0,
                thumbnail.width,
                thumbnail.height,
                thumbnail.bgra.as_ptr() as *const c_void,
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
        draw.rcItem.left + 68
    } else {
        draw.rcItem.left + 14
    };

    let previous_font = unsafe { SelectObject(draw.hDC, font) };
    let text = wide(&view.item.display_text());
    let mut first_line = RECT {
        left: content_left,
        top: draw.rcItem.top + 7,
        right: draw.rcItem.right - if view.item.pinned { 68 } else { 8 },
        bottom: draw.rcItem.top + 35,
    };
    unsafe {
        DrawTextW(
            draw.hDC,
            text.as_ptr(),
            -1,
            &mut first_line,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
    }
    if view.item.pinned {
        unsafe {
            SetTextColor(draw.hDC, ACCENT_COLOR);
        }
        let pin = wide("置顶");
        let mut pin_rect = RECT {
            left: draw.rcItem.right - 64,
            top: draw.rcItem.top + 7,
            right: draw.rcItem.right - 12,
            bottom: draw.rcItem.top + 35,
        };
        unsafe {
            DrawTextW(
                draw.hDC,
                pin.as_ptr(),
                -1,
                &mut pin_rect,
                DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
            );
        }
    }
    unsafe {
        SetTextColor(draw.hDC, colors.muted);
    }
    let relative_time = relative_history_time(view.item.last_used_at);
    let source = if view.item.source_exe.is_empty() {
        format!(
            "未知来源 · {} · {} KiB",
            relative_time,
            view.item.byte_size.div_ceil(1024)
        )
    } else {
        format!(
            "{} · {} · {} KiB",
            view.item.source_exe,
            relative_time,
            view.item.byte_size.div_ceil(1024)
        )
    };
    let source = wide(&source);
    let mut second_line = RECT {
        left: content_left,
        top: draw.rcItem.top + 35,
        right: draw.rcItem.right - 8,
        bottom: draw.rcItem.bottom - 4,
    };
    unsafe {
        DrawTextW(
            draw.hDC,
            source.as_ptr(),
            -1,
            &mut second_line,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        SelectObject(draw.hDC, previous_font);
    }
}

fn decode_thumbnail(png: &[u8]) -> Option<Thumbnail> {
    let image = image::load_from_memory(png)
        .ok()?
        .thumbnail(48, 48)
        .to_rgba8();
    let (width, height) = image.dimensions();
    let mut bgra = image.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(Thumbnail {
        width: width as i32,
        height: height as i32,
        bgra,
    })
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
