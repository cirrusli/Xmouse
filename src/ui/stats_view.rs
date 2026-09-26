use crate::stats::Snapshot;
use std::ffi::c_void;
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{
        CreateSolidBrush, DT_LEFT, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
        FillRect, HFONT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
};

use super::theme::{ACCENT_COLOR, Palette};

pub fn draw(hdc: *mut c_void, colors: Palette, body: HFONT, title: HFONT, snapshot: &Snapshot) {
    unsafe {
        SetBkMode(hdc, TRANSPARENT as i32);
    }
    draw_text(hdc, body, colors.muted, "已识别", 238, 194, 170, 24, false);
    draw_text(
        hdc,
        title,
        colors.text,
        &recognized_count(snapshot).to_string(),
        238,
        221,
        170,
        40,
        false,
    );
    draw_text(
        hdc,
        body,
        colors.muted,
        "调用成功",
        446,
        194,
        170,
        24,
        false,
    );
    draw_text(
        hdc,
        title,
        colors.text,
        &snapshot.successes.to_string(),
        446,
        221,
        170,
        40,
        false,
    );
    draw_text(
        hdc,
        body,
        colors.muted,
        "未完成绘制",
        650,
        194,
        190,
        24,
        false,
    );
    draw_text(
        hdc,
        title,
        colors.text,
        &(snapshot.unrecognized + snapshot.cancelled + snapshot.too_short).to_string(),
        650,
        221,
        170,
        40,
        false,
    );
    draw_text(
        hdc,
        body,
        colors.text,
        "使用频率排行",
        238,
        310,
        260,
        28,
        false,
    );
    draw_text(
        hdc,
        body,
        colors.muted,
        &format!(
            "绘制 {} · 失败 {} · 禁用 {}",
            snapshot.attempts, snapshot.failures, snapshot.disabled
        ),
        554,
        311,
        270,
        28,
        true,
    );
    if snapshot.ranking.is_empty() {
        draw_text(
            hdc,
            body,
            colors.muted,
            "当前时段还没有手势记录",
            238,
            410,
            560,
            36,
            false,
        );
        return;
    }
    let max_count = snapshot.ranking[0].count.max(1);
    for (index, rank) in snapshot.ranking.iter().take(9).enumerate() {
        let y = 354 + index as i32 * 29;
        draw_text(
            hdc,
            body,
            colors.muted,
            &format!("{:02}", index + 1),
            238,
            y,
            35,
            24,
            false,
        );
        draw_text(
            hdc,
            body,
            colors.text,
            rank.gesture.short_label(),
            278,
            y,
            105,
            24,
            false,
        );
        let background = RECT {
            left: 390,
            top: y + 8,
            right: 728,
            bottom: y + 17,
        };
        fill_rect(hdc, &background, colors.hover);
        let fill = RECT {
            right: background.left
                + ((background.right - background.left) as u64 * rank.count / max_count) as i32,
            ..background
        };
        fill_rect(hdc, &fill, ACCENT_COLOR);
        draw_text(
            hdc,
            body,
            colors.text,
            &rank.count.to_string(),
            744,
            y,
            80,
            24,
            true,
        );
    }
}

fn recognized_count(snapshot: &Snapshot) -> u64 {
    snapshot.ranking.iter().map(|rank| rank.count).sum()
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    hdc: *mut c_void,
    font: HFONT,
    color: u32,
    value: &str,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    right_aligned: bool,
) {
    let mut rect = RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    };
    let text = value.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let old_font = unsafe { SelectObject(hdc, font) };
    unsafe {
        SetTextColor(hdc, color);
        DrawTextW(
            hdc,
            text.as_ptr(),
            -1,
            &mut rect,
            DT_VCENTER | DT_SINGLELINE | if right_aligned { DT_RIGHT } else { DT_LEFT },
        );
        SelectObject(hdc, old_font);
    }
}

fn fill_rect(hdc: *mut c_void, rect: &RECT, color: u32) {
    let brush = unsafe { CreateSolidBrush(color) };
    unsafe {
        FillRect(hdc, rect, brush);
        DeleteObject(brush);
    }
}
