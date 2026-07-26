use super::{
    format::relative_history_time,
    theme::{ACCENT_COLOR, Palette},
};
use crate::storage::ClipItem;
use std::{ffi::c_void, mem};
use windows_sys::Win32::{
    Foundation::{RECT, SIZE},
    Graphics::Gdi::{
        BITMAPINFO, BITMAPINFOHEADER, CreateSolidBrush, DIB_RGB_COLORS, DT_END_ELLIPSIS, DT_LEFT,
        DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, FillRect,
        GetTextExtentPoint32W, HFONT, SRCCOPY, SelectObject, SetBkMode, SetTextColor,
        StretchDIBits, TRANSPARENT,
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
    search_query: String,
}

impl HistoryView {
    pub fn new(item: ClipItem, search_query: &str) -> Self {
        let thumbnail = item.thumbnail_png.as_deref().and_then(decode_thumbnail);
        Self {
            item,
            thumbnail,
            search_query: search_query.trim().to_owned(),
        }
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
    let display_text = view.item.display_text();
    let text = wide(&display_text);
    let mut first_line = RECT {
        left: content_left,
        top: draw.rcItem.top + 7,
        right: draw.rcItem.right - if view.item.pinned { 68 } else { 8 },
        bottom: draw.rcItem.top + 35,
    };
    draw_search_highlight(
        draw.hDC,
        &first_line,
        &display_text,
        &view.search_query,
        colors.highlight,
    );
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
    let source_text = source;
    let source = wide(&source_text);
    let mut second_line = RECT {
        left: content_left,
        top: draw.rcItem.top + 35,
        right: draw.rcItem.right - 8,
        bottom: draw.rcItem.bottom - 4,
    };
    draw_search_highlight(
        draw.hDC,
        &second_line,
        &source_text,
        &view.search_query,
        colors.highlight,
    );
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

fn draw_search_highlight(hdc: *mut c_void, line: &RECT, text: &str, query: &str, color: u32) {
    let Some((start, end)) = case_insensitive_match(text, query) else {
        return;
    };
    let prefix: Vec<u16> = text[..start].encode_utf16().collect();
    let matched: Vec<u16> = text[start..end].encode_utf16().collect();
    let mut prefix_size = SIZE::default();
    let mut match_size = SIZE::default();
    unsafe {
        GetTextExtentPoint32W(hdc, prefix.as_ptr(), prefix.len() as i32, &mut prefix_size);
        GetTextExtentPoint32W(hdc, matched.as_ptr(), matched.len() as i32, &mut match_size);
    }
    let left = line.left + prefix_size.cx;
    if left >= line.right || match_size.cx <= 0 {
        return;
    }
    let highlight = RECT {
        left,
        top: line.top + 4,
        right: (left + match_size.cx).min(line.right),
        bottom: line.bottom - 4,
    };
    let brush = unsafe { CreateSolidBrush(color) };
    unsafe {
        FillRect(hdc, &highlight, brush);
        DeleteObject(brush);
    }
}

fn case_insensitive_match(text: &str, query: &str) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    let folded_text = text.to_lowercase();
    let folded_query = query.to_lowercase();
    let start = folded_text.find(&folded_query)?;
    let end = start + folded_query.len();
    (text.is_char_boundary(start) && text.is_char_boundary(end)).then_some((start, end))
}

fn decode_thumbnail(png: &[u8]) -> Option<Thumbnail> {
    let image = image::load_from_memory(png)
        .ok()?
        .thumbnail(48, 48)
        .to_rgba8();
    let (width, height) = image.dimensions();
    let mut bgra = Vec::with_capacity((width * height * 4) as usize);
    for (index, pixel) in image.pixels().enumerate() {
        let x = index as u32 % width;
        let y = index as u32 / width;
        let checker = if (x / 6 + y / 6).is_multiple_of(2) {
            238u8
        } else {
            216u8
        };
        let alpha = pixel[3] as u16;
        let composite = |channel: u8| -> u8 {
            ((channel as u16 * alpha + checker as u16 * (255 - alpha)) / 255) as u8
        };
        bgra.extend_from_slice(&[
            composite(pixel[2]),
            composite(pixel[1]),
            composite(pixel[0]),
            255,
        ]);
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

#[cfg(test)]
mod tests {
    use super::case_insensitive_match;

    #[test]
    fn search_highlight_matches_ascii_case_and_chinese() {
        assert_eq!(case_insensitive_match("MSedge.exe", "edge"), Some((2, 6)));
        assert_eq!(
            case_insensitive_match("复制中文内容", "中文"),
            Some((6, 12))
        );
        assert_eq!(case_insensitive_match("图片", "文本"), None);
    }
}
