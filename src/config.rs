use crate::gesture::{GestureAction, UserGestureTemplate};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    pub capture: bool,
    pub auto_paste: bool,
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
            auto_paste: true,
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
pub struct GestureGuardConfig {
    pub disable_in_fullscreen_apps: bool,
    pub excluded_processes: Vec<String>,
}

impl Default for GestureGuardConfig {
    fn default() -> Self {
        Self {
            disable_in_fullscreen_apps: true,
            excluded_processes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub enabled: bool,
    pub dark_mode: bool,
    pub trigger: TriggerButton,
    pub activation_delay_ms: u32,
    pub activation_distance_dip: f32,
    pub minimum_stroke_length_dip: f32,
    pub recognition_threshold: f32,
    pub show_trail: bool,
    pub search_url_template: String,
    pub autostart: bool,
    pub custom_gestures: Vec<UserGestureTemplate>,
    pub gesture_guard: GestureGuardConfig,
    pub history: HistoryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            enabled: true,
            dark_mode: false,
            trigger: TriggerButton::Right,
            activation_delay_ms: 180,
            activation_distance_dip: 12.0,
            minimum_stroke_length_dip: 28.0,
            recognition_threshold: 0.82,
            show_trail: true,
            search_url_template: DEFAULT_SEARCH_URL.to_owned(),
            autostart: false,
            custom_gestures: Vec::new(),
            gesture_guard: GestureGuardConfig::default(),
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
        if !(12.0..=160.0).contains(&self.minimum_stroke_length_dip) {
            bail!("最短手势轨迹必须在 12–160 DIP 之间");
        }
        if !(0.60..=0.98).contains(&self.recognition_threshold) {
            bail!("识别阈值必须在 0.60–0.98 之间");
        }
        if !self.search_url_template.contains("{query}") {
            bail!("搜索网址必须包含 {{query}}");
        }
        if self.custom_gestures.len() > GestureAction::ALL.len() * 3 {
            bail!("每个动作最多保存 3 个个性化手势样本");
        }
        for action in GestureAction::ALL {
            if self
                .custom_gestures
                .iter()
                .filter(|sample| sample.action == action)
                .count()
                > 3
            {
                bail!("{} 的个性化样本超过 3 个", action.short_label());
            }
        }
        if self.custom_gestures.iter().any(|sample| !sample.is_valid()) {
            bail!("个性化手势样本损坏");
        }
        if self.gesture_guard.excluded_processes.len() > 256
            || self
                .gesture_guard
                .excluded_processes
                .iter()
                .any(|process| process.trim().is_empty() || process.len() > 260)
        {
            bail!("手势排除程序列表无效");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gesture::Point;

    #[test]
    fn older_config_defaults_to_no_personalized_gestures() {
        let config: AppConfig = serde_json::from_str("{}").expect("default config");
        assert!(config.custom_gestures.is_empty());
        assert!(config.gesture_guard.disable_in_fullscreen_apps);
        assert!(config.gesture_guard.excluded_processes.is_empty());
        assert!(config.history.auto_paste);
        config.validate().expect("default remains valid");
    }

    #[test]
    fn gesture_and_clipboard_exclusions_are_independent() {
        let config: AppConfig = serde_json::from_str(
            r#"{
                "gesture_guard": { "excluded_processes": ["game.exe"] },
                "history": { "excluded_processes": ["password-manager.exe"] }
            }"#,
        )
        .expect("independent exclusions");

        assert_eq!(config.gesture_guard.excluded_processes, ["game.exe"]);
        assert_eq!(config.history.excluded_processes, ["password-manager.exe"]);
    }

    #[test]
    fn limits_personalized_samples_per_action() {
        let points = (0..64)
            .map(|index| Point::new(index as f32, index as f32))
            .collect();
        let sample = UserGestureTemplate {
            action: GestureAction::SearchSelection,
            points,
        };
        assert!(sample.is_valid());
        let config = AppConfig {
            custom_gestures: vec![sample; 4],
            ..AppConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
