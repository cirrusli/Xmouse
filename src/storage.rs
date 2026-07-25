use crate::config::AppConfig;
use anyhow::{Context, Result, bail};
use image::{
    DynamicImage, ImageBuffer, ImageEncoder, Rgba,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
use windows::{
    Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    },
    core::PCWSTR,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    Text,
    Image,
}

impl ClipKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentEncoding {
    Plain,
    Dpapi,
}

impl ContentEncoding {
    fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Dpapi => "dpapi",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "plain" => Ok(Self::Plain),
            "dpapi" => Ok(Self::Dpapi),
            _ => bail!("未知剪贴板内容编码：{value}"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ClipPayload {
    Text(String),
    ImagePng(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct ClipItem {
    pub id: i64,
    pub kind: ClipKind,
    pub text: Option<String>,
    pub thumbnail_png: Option<Vec<u8>>,
    pub source_exe: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub byte_size: u64,
    pub last_used_at: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StorageStats {
    pub item_count: u64,
    pub content_bytes: u64,
    pub disk_bytes: u64,
}

impl ClipItem {
    pub fn display_text(&self) -> String {
        match self.kind {
            ClipKind::Text => self
                .text
                .as_deref()
                .unwrap_or("")
                .replace(['\r', '\n', '\t'], " "),
            ClipKind::Image => format!(
                "图片  {} × {}",
                self.width.unwrap_or_default(),
                self.height.unwrap_or_default()
            ),
        }
    }
}

#[derive(Clone)]
pub struct Storage {
    media_dir: PathBuf,
    database_path: PathBuf,
    config: Arc<RwLock<AppConfig>>,
}

impl Storage {
    pub fn open(root: PathBuf, config: Arc<RwLock<AppConfig>>) -> Result<Self> {
        let media_dir = root.join("media");
        fs::create_dir_all(&media_dir)
            .with_context(|| format!("创建数据目录失败：{}", media_dir.display()))?;
        let storage = Self {
            database_path: root.join("history.db"),
            media_dir,
            config,
        };
        if let Err(error) = storage.initialize() {
            if !is_database_corruption(&error) || !storage.database_path.exists() {
                return Err(error);
            }
            crate::logging::error("恢复数据库", &error);
            storage.recover_corrupt_database()?;
            storage
                .initialize()
                .context("重建损坏的剪贴板历史数据库失败")?;
        }
        Ok(storage)
    }

    fn connect(&self) -> Result<Connection> {
        let connection = Connection::open(&self.database_path)
            .with_context(|| format!("打开数据库失败：{}", self.database_path.display()))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        connection.busy_timeout(std::time::Duration::from_secs(2))?;
        Ok(connection)
    }

    fn initialize(&self) -> Result<()> {
        let connection = self.connect()?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS clips (
                id                  INTEGER PRIMARY KEY AUTOINCREMENT,
                kind                TEXT NOT NULL,
                content_hash        BLOB NOT NULL UNIQUE,
                protected_text      BLOB,
                plain_text          TEXT,
                media_name          TEXT,
                protected_thumbnail BLOB,
                source_exe          TEXT NOT NULL DEFAULT '',
                width               INTEGER,
                height              INTEGER,
                byte_size           INTEGER NOT NULL,
                created_at          INTEGER NOT NULL,
                last_used_at        INTEGER NOT NULL,
                content_encoding    TEXT NOT NULL DEFAULT 'plain'
            );
            CREATE INDEX IF NOT EXISTS idx_clips_last_used
                ON clips(last_used_at DESC);
            ",
        )?;
        let has_encoding: bool = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info('clips')
                WHERE name = 'content_encoding'
            )",
            [],
            |row| row.get(0),
        )?;
        if !has_encoding {
            connection.execute(
                "ALTER TABLE clips
                 ADD COLUMN content_encoding TEXT NOT NULL DEFAULT 'dpapi'",
                [],
            )?;
        }
        let has_plain_text: bool = connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM pragma_table_info('clips')
                WHERE name = 'plain_text'
            )",
            [],
            |row| row.get(0),
        )?;
        if !has_plain_text {
            connection.execute("ALTER TABLE clips ADD COLUMN plain_text TEXT", [])?;
        }
        Ok(())
    }

    fn recover_corrupt_database(&self) -> Result<()> {
        let timestamp = unix_millis();
        let backup = self
            .database_path
            .with_extension(format!("db.corrupt-{timestamp}"));
        fs::rename(&self.database_path, &backup).with_context(|| {
            format!(
                "备份损坏数据库失败：{} -> {}",
                self.database_path.display(),
                backup.display()
            )
        })?;
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", self.database_path.display()));
            if sidecar.exists() {
                let backup_sidecar = PathBuf::from(format!("{}{suffix}", backup.display()));
                let _ = fs::rename(sidecar, backup_sidecar);
            }
        }
        Ok(())
    }

    pub fn store(&self, payload: ClipPayload, source_exe: &str) -> Result<()> {
        let config = self.config.read().expect("config poisoned").clone();
        if !config.history.capture {
            return Ok(());
        }
        if config
            .history
            .excluded_processes
            .iter()
            .any(|item| item.eq_ignore_ascii_case(source_exe))
        {
            return Ok(());
        }

        match payload {
            ClipPayload::Text(text) => {
                if text.is_empty() {
                    return Ok(());
                }
                let bytes = text.as_bytes();
                if bytes.len() > config.history.max_text_bytes {
                    bail!("文本超过 {} 字节，已跳过", config.history.max_text_bytes);
                }
                self.store_text(&text, source_exe, config.history.encrypt_content)?;
            }
            ClipPayload::ImagePng(png) => {
                if png.len() as u64 > config.history.max_image_stored_mib * 1024 * 1024 {
                    bail!(
                        "图片超过 {} MiB，已跳过",
                        config.history.max_image_stored_mib
                    );
                }
                self.store_image(&png, source_exe, config.history.encrypt_content)?;
            }
        }
        self.evict_if_needed()
    }

    fn store_text(&self, text: &str, source_exe: &str, encrypt: bool) -> Result<()> {
        let hash = Sha256::digest(text.as_bytes());
        let (protected_text, plain_text, encoding) = if encrypt {
            (
                Some(protect(text.as_bytes())?),
                None,
                ContentEncoding::Dpapi,
            )
        } else {
            (None, Some(text), ContentEncoding::Plain)
        };
        let now = unix_millis();
        let connection = self.connect()?;
        connection.execute(
            "
            INSERT INTO clips (
                kind, content_hash, protected_text, plain_text, source_exe,
                byte_size, created_at, last_used_at, content_encoding
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)
            ON CONFLICT(content_hash) DO UPDATE SET
                protected_text = excluded.protected_text,
                plain_text = excluded.plain_text,
                source_exe = excluded.source_exe,
                last_used_at = excluded.last_used_at,
                content_encoding = excluded.content_encoding
            ",
            params![
                ClipKind::Text.as_str(),
                hash.as_slice(),
                protected_text,
                plain_text,
                source_exe,
                text.len() as i64,
                now,
                encoding.as_str()
            ],
        )?;
        Ok(())
    }

    fn store_image(&self, png: &[u8], source_exe: &str, encrypt: bool) -> Result<()> {
        let (width, height, thumbnail) = png_metadata_and_thumbnail(png)?;
        let hash = Sha256::digest(png);
        let (media, encoding) = encode_content(png, encrypt)?;
        let (thumbnail_content, _) = encode_content(&thumbnail, encrypt)?;
        let extension = if encrypt { "bin" } else { "png" };
        let name = format!("{}.{}", hex(hash.as_slice()), extension);
        let media_path = self.media_dir.join(&name);
        let now = unix_millis();

        let connection = self.connect()?;
        let existing: Option<(i64, Option<String>)> = connection
            .query_row(
                "SELECT id, media_name FROM clips WHERE content_hash = ?1",
                [hash.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        if let Some((id, previous_name)) = existing {
            fs::write(&media_path, &media)
                .with_context(|| format!("保存图片历史失败：{}", media_path.display()))?;
            connection.execute(
                "
                UPDATE clips
                SET media_name = ?1, protected_thumbnail = ?2,
                    source_exe = ?3, last_used_at = ?4, content_encoding = ?5
                WHERE id = ?6
                ",
                params![
                    name,
                    thumbnail_content,
                    source_exe,
                    now,
                    encoding.as_str(),
                    id
                ],
            )?;
            if let Some(previous_name) = previous_name
                && previous_name != name
            {
                let _ = fs::remove_file(self.media_dir.join(previous_name));
            }
            return Ok(());
        }

        fs::write(&media_path, media)
            .with_context(|| format!("保存图片历史失败：{}", media_path.display()))?;
        if let Err(error) = connection.execute(
            "
            INSERT INTO clips (
                kind, content_hash, media_name, protected_thumbnail,
                source_exe, width, height, byte_size, created_at, last_used_at,
                content_encoding
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10)
            ",
            params![
                ClipKind::Image.as_str(),
                hash.as_slice(),
                name,
                thumbnail_content,
                source_exe,
                width,
                height,
                png.len() as i64,
                now,
                encoding.as_str()
            ],
        ) {
            let _ = fs::remove_file(&media_path);
            return Err(error.into());
        }
        Ok(())
    }

    pub fn list(&self, query: &str) -> Result<Vec<ClipItem>> {
        let connection = self.connect()?;
        let mut statement = connection.prepare(
            "
            SELECT
                id, kind, protected_text, protected_thumbnail, source_exe,
                width, height, byte_size, last_used_at, content_encoding,
                plain_text
            FROM clips
            ORDER BY last_used_at DESC, id DESC
            ",
        )?;
        let mut rows = statement.query([])?;
        let query = query.trim().to_lowercase();
        let mut items = Vec::new();

        while let Some(row) = rows.next()? {
            let kind_text: String = row.get(1)?;
            let Some(kind) = ClipKind::parse(&kind_text) else {
                continue;
            };
            let protected_text: Option<Vec<u8>> = row.get(2)?;
            let protected_thumbnail: Option<Vec<u8>> = row.get(3)?;
            let encoding_text: String = row.get(9)?;
            let encoding = ContentEncoding::parse(&encoding_text)?;
            let text = match encoding {
                ContentEncoding::Plain => {
                    let plain_text: Option<String> = row.get(10)?;
                    match plain_text {
                        Some(text) => Some(text),
                        None => protected_text
                            .map(String::from_utf8)
                            .transpose()
                            .context("剪贴板文本不是 UTF-8")?,
                    }
                }
                ContentEncoding::Dpapi => protected_text
                    .as_deref()
                    .map(unprotect)
                    .transpose()?
                    .map(String::from_utf8)
                    .transpose()
                    .context("剪贴板文本不是 UTF-8")?,
            };
            let source_exe: String = row.get(4)?;
            let item = ClipItem {
                id: row.get(0)?,
                kind,
                text,
                thumbnail_png: protected_thumbnail
                    .as_deref()
                    .map(|content| decode_content(content, encoding))
                    .transpose()?,
                source_exe,
                width: row.get(5)?,
                height: row.get(6)?,
                byte_size: row.get::<_, i64>(7)?.max(0) as u64,
                last_used_at: row.get(8)?,
            };
            if query.is_empty()
                || item.display_text().to_lowercase().contains(&query)
                || item.source_exe.to_lowercase().contains(&query)
            {
                items.push(item);
            }
        }
        Ok(items)
    }

    pub fn payload(&self, id: i64) -> Result<ClipPayload> {
        let connection = self.connect()?;
        let row: (
            String,
            Option<Vec<u8>>,
            Option<String>,
            String,
            Option<String>,
        ) = connection
            .query_row(
                "SELECT kind, protected_text, media_name, content_encoding, plain_text
                 FROM clips WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .with_context(|| format!("找不到历史记录 {id}"))?;
        let encoding = ContentEncoding::parse(&row.3)?;

        match ClipKind::parse(&row.0) {
            Some(ClipKind::Text) => {
                let text = match encoding {
                    ContentEncoding::Plain => match row.4 {
                        Some(text) => text,
                        None => String::from_utf8(row.1.context("文本记录缺少内容")?)?,
                    },
                    ContentEncoding::Dpapi => {
                        String::from_utf8(unprotect(&row.1.context("文本记录缺少内容")?)?)?
                    }
                };
                Ok(ClipPayload::Text(text))
            }
            Some(ClipKind::Image) => {
                let name = row.2.context("图片记录缺少文件名")?;
                let content = fs::read(self.media_dir.join(name))?;
                Ok(ClipPayload::ImagePng(decode_content(&content, encoding)?))
            }
            None => bail!("未知剪贴板记录类型"),
        }
    }

    pub fn touch(&self, id: i64) -> Result<()> {
        self.connect()?.execute(
            "UPDATE clips SET last_used_at = ?1 WHERE id = ?2",
            params![unix_millis(), id],
        )?;
        Ok(())
    }

    pub fn remove(&self, id: i64) -> Result<()> {
        let connection = self.connect()?;
        let media: Option<String> = connection
            .query_row("SELECT media_name FROM clips WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()?
            .flatten();
        connection.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        if let Some(name) = media {
            let _ = fs::remove_file(self.media_dir.join(name));
        }
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let connection = self.connect()?;
        let mut statement =
            connection.prepare("SELECT media_name FROM clips WHERE media_name IS NOT NULL")?;
        let names: Vec<String> = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        connection.execute("DELETE FROM clips", [])?;
        drop(statement);
        for name in names {
            let _ = fs::remove_file(self.media_dir.join(name));
        }
        Ok(())
    }

    pub fn stats(&self) -> Result<StorageStats> {
        let connection = self.connect()?;
        let (count, content_bytes): (i64, i64) = connection.query_row(
            "SELECT COUNT(*), COALESCE(SUM(byte_size), 0) FROM clips",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        drop(connection);

        let mut disk_bytes = file_size(&self.database_path)
            .saturating_add(file_size(&self.database_path.with_extension("db-wal")))
            .saturating_add(file_size(&self.database_path.with_extension("db-shm")));
        if let Ok(entries) = fs::read_dir(&self.media_dir) {
            for entry in entries.flatten() {
                disk_bytes = disk_bytes.saturating_add(file_size(&entry.path()));
            }
        }
        Ok(StorageStats {
            item_count: count.max(0) as u64,
            content_bytes: content_bytes.max(0) as u64,
            disk_bytes,
        })
    }

    fn evict_if_needed(&self) -> Result<()> {
        let config = self.config.read().expect("config poisoned").clone();
        let max_bytes = config.history.max_disk_mib * 1024 * 1024;
        let connection = self.connect()?;
        loop {
            let (count, bytes): (i64, i64) = connection.query_row(
                "SELECT COUNT(*), COALESCE(SUM(byte_size), 0) FROM clips",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if count as usize <= config.history.max_items && bytes.max(0) as u64 <= max_bytes {
                break;
            }
            let victim: Option<(i64, Option<String>)> = connection
                .query_row(
                    "
                    SELECT id, media_name
                    FROM clips
                    ORDER BY last_used_at ASC, id ASC
                    LIMIT 1
                    ",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((id, media)) = victim else {
                break;
            };
            connection.execute("DELETE FROM clips WHERE id = ?1", [id])?;
            if let Some(name) = media {
                let _ = fs::remove_file(self.media_dir.join(name));
            }
        }
        Ok(())
    }
}

fn file_size(path: &std::path::Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

fn is_database_corruption(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let Some(rusqlite::Error::SqliteFailure(inner, _)) =
            cause.downcast_ref::<rusqlite::Error>()
        else {
            return false;
        };
        matches!(
            inner.code,
            rusqlite::ffi::ErrorCode::DatabaseCorrupt | rusqlite::ffi::ErrorCode::NotADatabase
        )
    })
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(TABLE[(byte >> 4) as usize] as char);
        result.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    result
}

fn encode_content(data: &[u8], encrypt: bool) -> Result<(Vec<u8>, ContentEncoding)> {
    if encrypt {
        Ok((protect(data)?, ContentEncoding::Dpapi))
    } else {
        Ok((data.to_vec(), ContentEncoding::Plain))
    }
}

fn decode_content(data: &[u8], encoding: ContentEncoding) -> Result<Vec<u8>> {
    match encoding {
        ContentEncoding::Plain => Ok(data.to_vec()),
        ContentEncoding::Dpapi => unprotect(data),
    }
}

fn protect(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len().try_into().context("DPAPI 输入过大")?,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .context("DPAPI 加密失败")?;
        let protected = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(protected)
    }
}

fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len().try_into().context("DPAPI 输入过大")?,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .context("DPAPI 解密失败")?;
        let plain = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
        Ok(plain)
    }
}

pub fn normalize_image_to_png(
    bytes: &[u8],
    source_is_png: bool,
    max_input_bytes: u64,
) -> Result<Vec<u8>> {
    if bytes.len() as u64 > max_input_bytes {
        bail!("图片输入超过限制");
    }
    if source_is_png {
        let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .context("PNG 数据损坏")?;
        if image.width() == 0 || image.height() == 0 {
            bail!("空图片");
        }
        return Ok(bytes.to_vec());
    }
    dib_to_png(bytes)
}

pub fn png_to_dib(png: &[u8]) -> Result<Vec<u8>> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .context("无法解码历史图片")?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let pixel_bytes = width as usize * height as usize * 4;
    let mut dib = vec![0u8; 40 + pixel_bytes];
    write_u32(&mut dib[0..4], 40);
    write_i32(&mut dib[4..8], width.try_into().context("图片过宽")?);
    write_i32(&mut dib[8..12], height.try_into().context("图片过高")?);
    write_u16(&mut dib[12..14], 1);
    write_u16(&mut dib[14..16], 32);
    write_u32(&mut dib[16..20], 0);
    write_u32(
        &mut dib[20..24],
        pixel_bytes.try_into().context("图片过大")?,
    );
    for y in 0..height {
        let source_y = height - 1 - y;
        for x in 0..width {
            let pixel = image.get_pixel(x, source_y).0;
            let offset = 40 + ((y * width + x) * 4) as usize;
            dib[offset..offset + 4].copy_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    Ok(dib)
}

fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>> {
    if dib.len() < 40 {
        bail!("DIB 数据过短");
    }
    let header_size = read_u32(&dib[0..4]) as usize;
    if header_size < 40 || header_size > dib.len() {
        bail!("不支持的 DIB 头");
    }
    let width = read_i32(&dib[4..8]);
    let height_signed = read_i32(&dib[8..12]);
    let planes = read_u16(&dib[12..14]);
    let bits = read_u16(&dib[14..16]);
    let compression = read_u32(&dib[16..20]);
    if width <= 0 || height_signed == 0 || planes != 1 || !matches!(bits, 24 | 32) {
        bail!("仅支持 24/32 位 DIB");
    }
    if !matches!(compression, 0 | 3) {
        bail!("不支持压缩 DIB");
    }
    let width = width as u32;
    let height = height_signed.unsigned_abs();
    let masks_size = if header_size == 40 && compression == 3 {
        12
    } else {
        0
    };
    let pixel_offset = header_size + masks_size;
    let row_stride = (width as usize * bits as usize).div_ceil(32) * 4;
    let required = pixel_offset
        .checked_add(
            row_stride
                .checked_mul(height as usize)
                .context("DIB 过大")?,
        )
        .context("DIB 过大")?;
    if required > dib.len() {
        bail!("DIB 像素数据不完整");
    }
    let mut rgba = vec![0u8; width as usize * height as usize * 4];
    let top_down = height_signed < 0;
    let bytes_per_pixel = (bits / 8) as usize;
    let mut all_alpha_zero = bits == 32;
    for output_y in 0..height as usize {
        let source_y = if top_down {
            output_y
        } else {
            height as usize - 1 - output_y
        };
        let row = &dib[pixel_offset + source_y * row_stride..];
        for x in 0..width as usize {
            let source = x * bytes_per_pixel;
            let target = (output_y * width as usize + x) * 4;
            rgba[target] = row[source + 2];
            rgba[target + 1] = row[source + 1];
            rgba[target + 2] = row[source];
            rgba[target + 3] = if bits == 32 { row[source + 3] } else { 255 };
            all_alpha_zero &= rgba[target + 3] == 0;
        }
    }
    if all_alpha_zero {
        for alpha in rgba.iter_mut().skip(3).step_by(4) {
            *alpha = 255;
        }
    }
    let image: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, rgba).context("DIB 尺寸无效")?;
    encode_png(&DynamicImage::ImageRgba8(image))
}

fn png_metadata_and_thumbnail(png: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .context("无法解析 PNG")?;
    let width = image.width();
    let height = image.height();
    let thumbnail = image.thumbnail(96, 96);
    Ok((width, height, encode_png(&thumbnail)?))
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>> {
    let rgba = image.to_rgba8();
    let mut output = Vec::new();
    PngEncoder::new_with_quality(&mut output, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )?;
    Ok(output)
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn read_i32(bytes: &[u8]) -> i32 {
    i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn write_u16(bytes: &mut [u8], value: u16) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], value: u32) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn write_i32(bytes: &mut [u8], value: i32) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_round_trip_preserves_dimensions() {
        let image =
            DynamicImage::ImageRgba8(ImageBuffer::from_pixel(8, 4, Rgba([20, 40, 80, 255])));
        let png = encode_png(&image).unwrap();
        let dib = png_to_dib(&png).unwrap();
        let restored = dib_to_png(&dib).unwrap();
        let decoded = image::load_from_memory(&restored).unwrap();
        assert_eq!(decoded.width(), 8);
        assert_eq!(decoded.height(), 4);
    }

    #[test]
    fn hex_is_stable() {
        assert_eq!(hex(&[0, 15, 16, 255]), "000f10ff");
    }

    #[test]
    fn dpapi_round_trip_uses_current_user() {
        let plain = "Xmouse DPAPI 测试".as_bytes();
        let protected = match protect(plain) {
            Ok(value) => value,
            Err(error) if format!("{error:#}").contains("0x80070002") => {
                eprintln!("当前测试令牌未加载用户 DPAPI 配置，跳过运行时往返");
                return;
            }
            Err(error) => panic!("{error:#}"),
        };
        assert_ne!(protected, plain);
        assert_eq!(unprotect(&protected).unwrap(), plain);
    }

    #[test]
    fn plaintext_history_is_readable_for_debugging() {
        let root = std::env::temp_dir().join(format!(
            "xmouse-plaintext-test-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        let config = Arc::new(RwLock::new(AppConfig::default()));
        let storage = Storage::open(root.clone(), config).unwrap();
        storage
            .store(
                ClipPayload::Text("可直接查看的剪贴板文本".to_owned()),
                "notepad.exe",
            )
            .unwrap();
        let connection = storage.connect().unwrap();
        let (plain_text, encoding, protected_text): (String, String, Option<Vec<u8>>) = connection
            .query_row(
                "SELECT plain_text, content_encoding, protected_text
                 FROM clips LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(encoding, "plain");
        assert_eq!(plain_text, "可直接查看的剪贴板文本");
        assert!(protected_text.is_none());
        drop(connection);
        match storage.payload(1).unwrap() {
            ClipPayload::Text(text) => assert_eq!(text, "可直接查看的剪贴板文本"),
            ClipPayload::ImagePng(_) => panic!("expected text"),
        }
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_database_is_backed_up_and_recreated() {
        let root = std::env::temp_dir().join(format!(
            "xmouse-storage-test-{}-{}",
            std::process::id(),
            unix_millis()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("history.db"), b"not a sqlite database").unwrap();
        let config = Arc::new(RwLock::new(AppConfig::default()));
        let storage = Storage::open(root.clone(), config).unwrap();
        assert!(storage.list("").unwrap().is_empty());
        assert!(
            fs::read_dir(&root)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-"))
        );
        drop(storage);
        fs::remove_dir_all(root).unwrap();
    }
}
