use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    core::PCWSTR,
};

pub const APP_DIR_NAME: &str = "Xmouse";
pub const DEFAULT_SEARCH_URL: &str = "https://www.google.com/search?q={query}";
const LEGACY_BING_SEARCH_URL: &str = "https://www.bing.com/search?q={query}";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TriggerButton {
    Right,
    X1,
    X2,
}

impl TriggerButton {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Right => "鼠标右键",
            Self::X1 => "侧键 X1",
            Self::X2 => "侧键 X2",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Right => 0,
            Self::X1 => 1,
            Self::X2 => 2,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::X1,
            2 => Self::X2,
            _ => Self::Right,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    pub capture: bool,
    pub encrypt_content: bool,
    pub max_items: usize,
    pub max_disk_mib: u64,
    pub max_text_bytes: usize,
    pub max_image_input_mib: u64,
    pub max_image_stored_mib: u64,
    pub excluded_processes: Vec<String>,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            capture: true,
            encrypt_content: false,
            max_items: 200,
            max_disk_mib: 256,
            max_text_bytes: 1_048_576,
            max_image_input_mib: 64,
            max_image_stored_mib: 20,
            excluded_processes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub enabled: bool,
    pub trigger: TriggerButton,
    pub activation_delay_ms: u32,
    pub activation_distance_dip: f32,
    pub recognition_threshold: f32,
    pub show_trail: bool,
    pub search_url_template: String,
    pub autostart: bool,
    pub history: HistoryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            trigger: TriggerButton::Right,
            activation_delay_ms: 180,
            activation_distance_dip: 12.0,
            recognition_threshold: 0.82,
            show_trail: true,
            search_url_template: DEFAULT_SEARCH_URL.to_owned(),
            autostart: false,
            history: HistoryConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!("不支持的配置版本 {}", self.schema_version);
        }
        if !(50..=1_000).contains(&self.activation_delay_ms) {
            bail!("触发延时必须在 50–1000 ms 之间");
        }
        if !(4.0..=80.0).contains(&self.activation_distance_dip) {
            bail!("触发距离必须在 4–80 DIP 之间");
        }
        if !(0.60..=0.98).contains(&self.recognition_threshold) {
            bail!("识别阈值必须在 0.60–0.98 之间");
        }
        if !self.search_url_template.contains("{query}") {
            bail!("搜索网址必须包含 {{query}}");
        }
        if !(10..=10_000).contains(&self.history.max_items) {
            bail!("历史数量必须在 10–10000 之间");
        }
        if !(16..=16_384).contains(&self.history.max_disk_mib) {
            bail!("历史磁盘上限必须在 16–16384 MiB 之间");
        }
        Ok(())
    }
}

pub fn app_data_dir() -> Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("找不到 LOCALAPPDATA")?;
    Ok(base.join(APP_DIR_NAME))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(app_data_dir()?.join("config.json"))
}

pub fn load_or_create() -> Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        let config = AppConfig::default();
        save(&path, &config)?;
        return Ok(config);
    }

    let bytes = fs::read(&path).with_context(|| format!("读取配置失败：{}", path.display()))?;
    match serde_json::from_slice::<AppConfig>(&bytes) {
        Ok(mut config) => {
            if config.search_url_template == LEGACY_BING_SEARCH_URL {
                config.search_url_template = DEFAULT_SEARCH_URL.to_owned();
                save(&path, &config)?;
            }
            config.validate()?;
            Ok(config)
        }
        Err(error) => {
            let backup = path.with_extension("json.invalid");
            let _ = fs::copy(&path, &backup);
            let config = AppConfig::default();
            save(&path, &config)?;
            Err(error).context(format!(
                "配置损坏，已备份到 {} 并恢复默认值",
                backup.display()
            ))
        }
    }
}

pub fn save(path: &Path, config: &AppConfig) -> Result<()> {
    config.validate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建配置目录失败：{}", parent.display()))?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(config)?;
    let mut file =
        fs::File::create(&temp).with_context(|| format!("写入配置失败：{}", temp.display()))?;
    file.write_all(&data)
        .with_context(|| format!("写入配置失败：{}", temp.display()))?;
    file.sync_all()
        .with_context(|| format!("刷新配置失败：{}", temp.display()))?;
    drop(file);

    let temp_wide: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        MoveFileExW(
            PCWSTR(temp_wide.as_ptr()),
            PCWSTR(path_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(error).with_context(|| format!("保存配置失败：{}", path.display()));
    }
    Ok(())
}
