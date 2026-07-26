use super::theme::Palette;
use std::{ffi::c_void, mem};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{
        BITMAPINFO, BITMAPINFOHEADER, CreatePen, CreateSolidBrush, DIB_RGB_COLORS, DeleteObject,
        FillRect, PS_SOLID, RoundRect, SRCCOPY, SelectObject, StretchDIBits,
    },
};

pub const MAX_PREVIEW_EDGE: u32 = 360;
pub const PREVIEW_PADDING: i32 = 12;

pub struct PreviewImage {
    pub item_id: i64,
    width: i32,
    height: i32,
    bgra: Vec<u8>,
}

impl PreviewImage {
    pub fn from_png(item_id: i64, png: &[u8]) -> Option<Self> {
        let image = image::load_from_memory(png)
            .ok()?
            .thumbnail(MAX_PREVIEW_EDGE, MAX_PREVIEW_EDGE)
            .to_rgba8();
        let (width, height) = image.dimensions();
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for (index, pixel) in image.pixels().enumerate() {
            let x = index as u32 % width;
            let y = index as u32 / width;
            let checker = if (x / 12 + y / 12).is_multiple_of(2) {
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
        Some(Self {
            item_id,
            width: width as i32,
            height: height as i32,
            bgra,
        })
    }

    pub fn window_size(&self) -> (i32, i32) {
        (
            self.width + PREVIEW_PADDING * 2,
            self.height + PREVIEW_PADDING * 2,
        )
    }

    pub fn draw(&self, hdc: *mut c_void, client: RECT, colors: Palette) {
        let background = unsafe { CreateSolidBrush(colors.card) };
        unsafe {
            FillRect(hdc, &client, background);
            DeleteObject(background);
        }
        let brush = unsafe { CreateSolidBrush(colors.card) };
        let pen = unsafe { CreatePen(PS_SOLID, 1, colors.border) };
        let previous_brush = unsafe { SelectObject(hdc, brush) };
        let previous_pen = unsafe { SelectObject(hdc, pen) };
        unsafe {
            RoundRect(hdc, 0, 0, client.right, client.bottom, 18, 18);
            SelectObject(hdc, previous_brush);
            SelectObject(hdc, previous_pen);
            DeleteObject(brush);
            DeleteObject(pen);
        }

        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: self.width,
                biHeight: -self.height,
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
                hdc,
                PREVIEW_PADDING,
                PREVIEW_PADDING,
                self.width,
                self.height,
                0,
                0,
                self.width,
                self.height,
                self.bgra.as_ptr() as *const c_void,
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, RgbaImage, codecs::png::PngEncoder};

    #[test]
    fn preview_preserves_aspect_ratio_and_bounds() {
        let image = RgbaImage::from_pixel(800, 400, image::Rgba([40, 120, 220, 128]));
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(image.as_raw(), 800, 400, image::ExtendedColorType::Rgba8)
            .unwrap();
        let preview = PreviewImage::from_png(7, &png).unwrap();
        assert_eq!(preview.item_id, 7);
        assert_eq!(preview.width, 360);
        assert_eq!(preview.height, 180);
        assert_eq!(preview.bgra.len(), 360 * 180 * 4);
    }
}
