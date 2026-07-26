use crate::{
    config::AppConfig,
    storage::{ClipPayload, Storage, normalize_image_to_png, png_to_dib},
};
use anyhow::{Context, Result, bail};
use std::{
    path::Path,
    ptr,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU32, Ordering},
    },
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, GlobalFree, HANDLE, HGLOBAL, HWND, SetLastError},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
            GetClipboardOwner, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
            SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        Ole::{
            CF_BITMAP, CF_DIB, CF_DIBV5, CF_DSPBITMAP, CF_DSPENHMETAFILE, CF_DSPMETAFILEPICT,
            CF_ENHMETAFILE, CF_METAFILEPICT, CF_PALETTE, CF_UNICODETEXT,
        },
        Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    },
};

const MAX_CLIPBOARD_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug)]
pub struct ClipboardSnapshot {
    formats: Vec<SnapshotFormat>,
}

#[derive(Debug)]
struct SnapshotFormat {
    format: u32,
    bytes: Vec<u8>,
}

#[derive(Clone)]
pub struct ClipboardService {
    storage: Storage,
    config: Arc<RwLock<AppConfig>>,
    ignored_updates: Arc<AtomicU32>,
    suspended_capture: Arc<AtomicU32>,
}

impl ClipboardService {
    pub fn new(storage: Storage, config: Arc<RwLock<AppConfig>>) -> Self {
        Self {
            storage,
            config,
            ignored_updates: Arc::new(AtomicU32::new(0)),
            suspended_capture: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn ignore_next_updates(&self, count: u32) {
        self.ignored_updates.fetch_add(count, Ordering::AcqRel);
    }

    pub fn clear_ignored_updates(&self) {
        self.ignored_updates.store(0, Ordering::Release);
    }

    pub fn consume_ignored_update(&self) -> bool {
        let mut current = self.ignored_updates.load(Ordering::Acquire);
        while current > 0 {
            match self.ignored_updates.compare_exchange_weak(
                current,
                current - 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(next) => current = next,
            }
        }
        false
    }

    pub fn is_capture_suspended(&self) -> bool {
        self.suspended_capture.load(Ordering::Acquire) > 0
    }

    pub fn suspend_capture(&self) -> CaptureSuspension {
        self.suspended_capture.fetch_add(1, Ordering::AcqRel);
        CaptureSuspension {
            counter: self.suspended_capture.clone(),
        }
    }

    pub fn read_current_text(&self) -> Result<Option<String>> {
        let guard = ClipboardGuard::open_with_retry(ptr::null_mut())?;
        let text = read_unicode_text();
        drop(guard);
        text
    }

    /// Materializes the clipboard instead of retaining the live IDataObject returned by OLE.
    /// A live object can recursively reopen the clipboard during OleSetClipboard and fail with
    /// CLIPBRD_E_CANT_OPEN after the target application has replaced its contents.
    pub fn snapshot_current(&self) -> Result<ClipboardSnapshot> {
        let guard = ClipboardGuard::open_with_retry(ptr::null_mut())?;
        let snapshot = capture_snapshot();
        drop(guard);
        snapshot
    }

    pub fn restore_snapshot(&self, snapshot: &ClipboardSnapshot) -> Result<()> {
        let mut prepared = Vec::with_capacity(snapshot.formats.len());
        for item in &snapshot.formats {
            let global = allocate_and_copy(item.bytes.as_ptr(), item.bytes.len())
                .with_context(|| format!("准备剪贴板格式 {} 失败", item.format))?;
            prepared.push(PreparedFormat {
                format: item.format,
                global: Some(global),
            });
        }

        let guard = ClipboardGuard::open_for_restore(ptr::null_mut())?;
        unsafe {
            if EmptyClipboard() == 0 {
                bail!("清空临时剪贴板失败 (Win32 {})", GetLastError());
            }
        }
        for item in &mut prepared {
            let global = item.global.context("待恢复的剪贴板内存已丢失")?;
            if unsafe { SetClipboardData(item.format, global as HANDLE) }.is_null() {
                bail!("恢复剪贴板格式 {} 失败 (Win32 {})", item.format, unsafe {
                    GetLastError()
                });
            }
            // SetClipboardData transfers ownership to the system on success.
            item.global = None;
        }
        drop(guard);
        Ok(())
    }

    pub fn capture_current(&self) -> Result<()> {
        let config = self.config.read().expect("config poisoned").clone();
        if !config.history.capture {
            return Ok(());
        }
        let _guard = ClipboardGuard::open_with_retry(ptr::null_mut())?;
        if clipboard_requests_exclusion()? {
            return Ok(());
        }
        let source = clipboard_source_process_name().unwrap_or_default();
        if config
            .history
            .excluded_processes
            .iter()
            .any(|item| item.eq_ignore_ascii_case(&source))
        {
            return Ok(());
        }

        if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT as u32) } != 0
            && let Some(text) = read_unicode_text()?
        {
            self.storage.store(ClipPayload::Text(text), &source)?;
            return Ok(());
        }

        let png_format = register_format("PNG");
        let max_input = config.history.max_image_input_mib * 1024 * 1024;
        if png_format != 0
            && unsafe { IsClipboardFormatAvailable(png_format) } != 0
            && let Some(bytes) = read_global_bytes(png_format, max_input)?
        {
            let png = normalize_image_to_png(&bytes, true, max_input)?;
            self.storage.store(ClipPayload::ImagePng(png), &source)?;
            return Ok(());
        }

        for format in [CF_DIBV5 as u32, CF_DIB as u32] {
            if unsafe { IsClipboardFormatAvailable(format) } != 0
                && let Some(bytes) = read_global_bytes(format, max_input)?
            {
                let png = normalize_image_to_png(&bytes, false, max_input)?;
                self.storage.store(ClipPayload::ImagePng(png), &source)?;
                return Ok(());
            }
        }
        Ok(())
    }

    pub fn set_payload(&self, payload: &ClipPayload) -> Result<()> {
        let _guard = ClipboardGuard::open_with_retry(ptr::null_mut())?;
        self.ignore_next_updates(1);
        unsafe {
            if EmptyClipboard() == 0 {
                bail!("清空剪贴板失败");
            }
        }
        match payload {
            ClipPayload::Text(text) => set_unicode_text(text),
            ClipPayload::ImagePng(png) => set_png_image(png),
        }
    }
}

pub struct CaptureSuspension {
    counter: Arc<AtomicU32>,
}

impl Drop for CaptureSuspension {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct ClipboardGuard;

impl ClipboardGuard {
    pub fn open_with_retry(owner: HWND) -> Result<Self> {
        const DELAYS: [u64; 8] = [0, 10, 20, 40, 80, 120, 180, 250];
        for delay in DELAYS {
            if delay > 0 {
                thread::sleep(Duration::from_millis(delay));
            }
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Self);
            }
        }
        bail!("剪贴板正被其他程序占用")
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}

impl ClipboardGuard {
    fn open_for_restore(owner: HWND) -> Result<Self> {
        const DELAYS: [u64; 12] = [0, 10, 20, 40, 80, 120, 180, 250, 400, 600, 900, 1_200];
        let mut last_error = 0;
        for delay in DELAYS {
            if delay > 0 {
                thread::sleep(Duration::from_millis(delay));
            }
            if unsafe { OpenClipboard(owner) } != 0 {
                return Ok(Self);
            }
            last_error = unsafe { GetLastError() };
        }
        bail!("恢复原剪贴板超时：仍被其他程序占用 (Win32 {last_error})")
    }
}

struct PreparedFormat {
    format: u32,
    global: Option<HGLOBAL>,
}

impl Drop for PreparedFormat {
    fn drop(&mut self) {
        if let Some(global) = self.global.take() {
            unsafe {
                GlobalFree(global);
            }
        }
    }
}

fn capture_snapshot() -> Result<ClipboardSnapshot> {
    let mut formats = Vec::new();
    let mut total_bytes = 0usize;
    let mut unsupported = Vec::new();
    let mut current = 0u32;

    loop {
        unsafe {
            SetLastError(0);
        }
        let next = unsafe { EnumClipboardFormats(current) };
        if next == 0 {
            let error = unsafe { GetLastError() };
            if error != 0 {
                bail!(
                    "枚举剪贴板格式失败 (Win32 {error}, previous {current}, copied {})",
                    formats.len()
                );
            }
            break;
        }
        current = next;
        // GDI and metafile clipboard formats are not HGLOBAL values. Calling GlobalSize on
        // those handles is undefined and can corrupt the process heap, so classify them
        // before attempting to materialize ordinary memory-backed formats.
        if is_non_global_handle_format(current) {
            unsupported.push(current);
            continue;
        }

        let handle = unsafe { GetClipboardData(current) };
        if handle.is_null() {
            bail!("读取剪贴板格式 {current} 失败 (Win32 {})", unsafe {
                GetLastError()
            });
        }
        let global = handle as HGLOBAL;
        let size = unsafe { GlobalSize(global) };
        if size == 0 {
            unsupported.push(current);
            continue;
        }
        total_bytes = total_bytes
            .checked_add(size)
            .context("剪贴板快照大小溢出")?;
        if total_bytes > MAX_CLIPBOARD_SNAPSHOT_BYTES {
            bail!("当前剪贴板超过 128 MiB，无法安全保存后再读取选区");
        }
        let pointer = unsafe { GlobalLock(global) } as *const u8;
        if pointer.is_null() {
            bail!("锁定剪贴板格式 {current} 失败 (Win32 {})", unsafe {
                GetLastError()
            });
        }
        let bytes = unsafe { std::slice::from_raw_parts(pointer, size) }.to_vec();
        unsafe {
            GlobalUnlock(global);
        }
        formats.push(SnapshotFormat {
            format: current,
            bytes,
        });
    }

    if !unsupported.is_empty() && !can_safely_omit_handle_formats(&unsupported, &formats) {
        bail!(
            "当前剪贴板含无法安全备份的格式 {:?}，已取消临时复制",
            unsupported
        );
    }

    Ok(ClipboardSnapshot { formats })
}

fn can_safely_omit_handle_formats(unsupported: &[u32], formats: &[SnapshotFormat]) -> bool {
    let has_bitmap_copy = formats
        .iter()
        .any(|item| item.format == CF_DIB as u32 || item.format == CF_DIBV5 as u32);
    unsupported.iter().all(|format| {
        matches!(
            *format,
            value if value == CF_BITMAP as u32
                || value == CF_DSPBITMAP as u32
                || value == CF_PALETTE as u32
        ) && has_bitmap_copy
    })
}

fn is_non_global_handle_format(format: u32) -> bool {
    matches!(
        format,
        value if value == CF_BITMAP as u32
            || value == CF_DSPBITMAP as u32
            || value == CF_PALETTE as u32
            || value == CF_METAFILEPICT as u32
            || value == CF_DSPMETAFILEPICT as u32
            || value == CF_ENHMETAFILE as u32
            || value == CF_DSPENHMETAFILE as u32
    )
}

fn clipboard_requests_exclusion() -> Result<bool> {
    let excluded = register_format("ExcludeClipboardContentFromMonitorProcessing");
    if excluded != 0 && unsafe { IsClipboardFormatAvailable(excluded) } != 0 {
        return Ok(true);
    }

    let can_include = register_format("CanIncludeInClipboardHistory");
    if can_include == 0 || unsafe { IsClipboardFormatAvailable(can_include) } == 0 {
        return Ok(false);
    }
    let handle = unsafe { GetClipboardData(can_include) };
    if handle.is_null() {
        return Ok(false);
    }
    let global = handle as HGLOBAL;
    let size = unsafe { GlobalSize(global) };
    if size < std::mem::size_of::<u32>() {
        return Ok(false);
    }
    let pointer = unsafe { GlobalLock(global) } as *const u32;
    if pointer.is_null() {
        return Ok(false);
    }
    let value = unsafe { pointer.read_unaligned() };
    unsafe {
        GlobalUnlock(global);
    }
    Ok(value == 0)
}

fn read_unicode_text() -> Result<Option<String>> {
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT as u32) };
    if handle.is_null() {
        return Ok(None);
    }
    let global = handle as HGLOBAL;
    let size = unsafe { GlobalSize(global) };
    if size < 2 {
        return Ok(None);
    }
    let pointer = unsafe { GlobalLock(global) } as *const u16;
    if pointer.is_null() {
        return Ok(None);
    }
    let max_units = size / 2;
    let units = unsafe { std::slice::from_raw_parts(pointer, max_units) };
    let length = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(max_units);
    let text = String::from_utf16_lossy(&units[..length]);
    unsafe {
        GlobalUnlock(global);
    }
    Ok(Some(text))
}

fn read_global_bytes(format: u32, maximum: u64) -> Result<Option<Vec<u8>>> {
    let handle = unsafe { GetClipboardData(format) };
    if handle.is_null() {
        return Ok(None);
    }
    let global = handle as HGLOBAL;
    let size = unsafe { GlobalSize(global) };
    if size == 0 {
        return Ok(None);
    }
    if size as u64 > maximum {
        bail!("剪贴板图片超过输入限制");
    }
    let pointer = unsafe { GlobalLock(global) } as *const u8;
    if pointer.is_null() {
        return Ok(None);
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer, size) }.to_vec();
    unsafe {
        GlobalUnlock(global);
    }
    Ok(Some(bytes))
}

fn set_unicode_text(text: &str) -> Result<()> {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let bytes = wide.len() * std::mem::size_of::<u16>();
    let global = allocate_and_copy(wide.as_ptr() as *const u8, bytes)?;
    let result = unsafe { SetClipboardData(CF_UNICODETEXT as u32, global as HANDLE) };
    if result.is_null() {
        unsafe {
            GlobalFree(global);
        }
        bail!("写入文本剪贴板失败");
    }
    Ok(())
}

fn set_png_image(png: &[u8]) -> Result<()> {
    let png_format = register_format("PNG");
    if png_format == 0 {
        bail!("注册 PNG 剪贴板格式失败");
    }
    let png_global = allocate_and_copy(png.as_ptr(), png.len())?;
    if unsafe { SetClipboardData(png_format, png_global as HANDLE) }.is_null() {
        unsafe {
            GlobalFree(png_global);
        }
        bail!("写入 PNG 剪贴板失败");
    }

    let dib = png_to_dib(png)?;
    let dib_global = allocate_and_copy(dib.as_ptr(), dib.len())?;
    if unsafe { SetClipboardData(CF_DIB as u32, dib_global as HANDLE) }.is_null() {
        unsafe {
            GlobalFree(dib_global);
        }
        bail!("写入 DIB 剪贴板失败");
    }
    Ok(())
}

fn allocate_and_copy(source: *const u8, length: usize) -> Result<HGLOBAL> {
    let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, length) };
    if global.is_null() {
        bail!("分配剪贴板内存失败");
    }
    let target = unsafe { GlobalLock(global) } as *mut u8;
    if target.is_null() {
        unsafe {
            GlobalFree(global);
        }
        bail!("锁定剪贴板内存失败");
    }
    unsafe {
        ptr::copy_nonoverlapping(source, target, length);
        GlobalUnlock(global);
    }
    Ok(global)
}

fn clipboard_source_process_name() -> Option<String> {
    let owner = unsafe { GetClipboardOwner() };
    if owner.is_null() {
        return None;
    }
    let mut process_id = 0u32;
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            owner,
            &mut process_id,
        );
    }
    process_name(process_id)
}

pub fn process_name(process_id: u32) -> Option<String> {
    if process_id == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return None;
    }
    let mut buffer = vec![0u16; 32_768];
    let mut size = buffer.len() as u32;
    let success = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size)
    };
    unsafe {
        CloseHandle(process);
    }
    if success == 0 || size == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    Path::new(&path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

fn register_format(name: &str) -> u32 {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::AppConfig, storage::Storage};
    use std::{collections::BTreeMap, fs, sync::RwLock, time::SystemTime};

    #[test]
    fn only_omits_bitmap_handles_when_a_dib_copy_exists() {
        let dib = vec![SnapshotFormat {
            format: CF_DIB as u32,
            bytes: vec![1],
        }];
        assert!(can_safely_omit_handle_formats(
            &[CF_BITMAP as u32, CF_PALETTE as u32],
            &dib
        ));
        assert!(!can_safely_omit_handle_formats(&[CF_BITMAP as u32], &[]));
        assert!(!can_safely_omit_handle_formats(&[49_999], &dib));
    }

    #[test]
    fn classifies_non_global_clipboard_handles_before_reading_their_size() {
        for format in [
            CF_BITMAP,
            CF_DSPBITMAP,
            CF_PALETTE,
            CF_METAFILEPICT,
            CF_DSPMETAFILEPICT,
            CF_ENHMETAFILE,
            CF_DSPENHMETAFILE,
        ] {
            assert!(is_non_global_handle_format(format as u32));
        }
        assert!(!is_non_global_handle_format(CF_UNICODETEXT as u32));
        assert!(!is_non_global_handle_format(CF_DIB as u32));
    }

    #[test]
    #[ignore = "temporarily rewrites the interactive user's clipboard"]
    fn materialized_snapshot_round_trips_text_and_custom_formats() {
        let suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("xmouse-clipboard-test-{suffix}"));
        let config = Arc::new(RwLock::new(AppConfig::default()));
        let storage = Storage::open(root.clone(), config.clone()).unwrap();
        let service = ClipboardService::new(storage, config);

        let user_clipboard = service.snapshot_current().unwrap();
        let as_map = |snapshot: ClipboardSnapshot| {
            snapshot
                .formats
                .into_iter()
                .map(|item| (item.format, item.bytes))
                .collect::<BTreeMap<_, _>>()
        };

        let exercise = (|| -> Result<()> {
            let text = "Xmouse original clipboard fixture";
            let mut unicode = text
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            unicode.extend_from_slice(&[0, 0]);
            let html_format = register_format("HTML Format");
            let source_format = register_format("Chromium internal source URL");
            let fixture = ClipboardSnapshot {
                formats: vec![
                    SnapshotFormat {
                        format: CF_UNICODETEXT as u32,
                        bytes: unicode,
                    },
                    SnapshotFormat {
                        format: html_format,
                        bytes: b"Version:0.9\r\nStartHTML:0000000000\r\n<html>Xmouse</html>\0"
                            .to_vec(),
                    },
                    SnapshotFormat {
                        format: source_format,
                        bytes: b"https://example.invalid/\0".to_vec(),
                    },
                ],
            };
            service.restore_snapshot(&fixture)?;
            thread::sleep(Duration::from_millis(100));
            let original = service.snapshot_current()?;

            let probe = "Xmouse temporary clipboard probe 7f3a";
            service.set_payload(&ClipPayload::Text(probe.to_owned()))?;
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            let mut observed_probe = None;
            while std::time::Instant::now() < deadline {
                observed_probe = service.read_current_text()?;
                if observed_probe.as_deref() == Some(probe) {
                    break;
                }
                thread::sleep(Duration::from_millis(15));
            }
            if observed_probe.as_deref() != Some(probe) {
                bail!("temporary clipboard text was not observable");
            }

            service.restore_snapshot(&original)?;
            thread::sleep(Duration::from_millis(100));
            let restored = service.snapshot_current()?;
            if as_map(restored) != as_map(original) {
                bail!("materialized clipboard formats did not round-trip exactly");
            }
            Ok(())
        })();

        let user_restore = service.restore_snapshot(&user_clipboard);
        thread::sleep(Duration::from_millis(100));
        fs::remove_dir_all(root).unwrap();
        user_restore.unwrap();
        exercise.unwrap();
    }
}
