use crate::{
    action::ActionKind,
    gesture::{GestureId, UserGestureTemplate},
};
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
pub const CURRENT_SCHEMA_VERSION: u32 = 2;
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct GestureBinding {
    pub gesture: GestureId,
    pub action: ActionKind,
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
    pub gesture_bindings: Vec<GestureBinding>,
    pub gesture_guard: GestureGuardConfig,
    pub history: HistoryConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
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
            gesture_bindings: default_gesture_bindings(),
            gesture_guard: GestureGuardConfig::default(),
            history: HistoryConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
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
        if self.custom_gestures.len() > GestureId::ALL.len() * 3 {
            bail!("每条轨迹最多保存 3 个个性化手势样本");
        }
        for gesture in GestureId::ALL {
            if self
                .custom_gestures
                .iter()
                .filter(|sample| sample.gesture == gesture)
                .count()
                > 3
            {
                bail!("{} 的个性化样本超过 3 个", gesture.short_label());
            }
        }
        if self.custom_gestures.iter().any(|sample| !sample.is_valid()) {
            bail!("个性化手势样本损坏");
        }
        if self.gesture_bindings.len() > GestureId::ALL.len() {
            bail!("手势动作映射数量无效");
        }
        for gesture in GestureId::ALL {
            if self
                .gesture_bindings
                .iter()
                .filter(|binding| binding.gesture == gesture)
                .count()
                > 1
            {
                bail!("{} 存在重复动作映射", gesture.short_label());
            }
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

    pub fn action_for(&self, gesture: GestureId) -> ActionKind {
        self.gesture_bindings
            .iter()
            .find(|binding| binding.gesture == gesture)
            .map(|binding| binding.action)
            .unwrap_or_else(|| default_action(gesture))
    }

    pub fn set_action(&mut self, gesture: GestureId, action: ActionKind) {
        if let Some(binding) = self
            .gesture_bindings
            .iter_mut()
            .find(|binding| binding.gesture == gesture)
        {
            binding.action = action;
        } else {
            self.gesture_bindings
                .push(GestureBinding { gesture, action });
        }
    }
}

pub fn default_action(gesture: GestureId) -> ActionKind {
    match gesture {
        GestureId::Up => ActionKind::ToggleTopmost,
        GestureId::LetterL => ActionKind::CloseTab,
        GestureId::LetterS => ActionKind::SearchSelection,
        GestureId::LetterC => ActionKind::CopySelection,
        GestureId::LetterV => ActionKind::OpenHistory,
        GestureId::Left => ActionKind::SwitchDesktopRight,
        GestureId::Right => ActionKind::SwitchDesktopLeft,
        GestureId::Seven => ActionKind::TaskManager,
        GestureId::Circle => ActionKind::ScreenSnip,
    }
}

fn default_gesture_bindings() -> Vec<GestureBinding> {
    GestureId::ALL
        .into_iter()
        .map(|gesture| GestureBinding {
            gesture,
            action: default_action(gesture),
        })
        .collect()
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
            let mut changed = migrate_config(&mut config)?;
            if config.search_url_template == LEGACY_BING_SEARCH_URL {
                config.search_url_template = DEFAULT_SEARCH_URL.to_owned();
                changed = true;
            }
            config.validate()?;
            if changed {
                save(&path, &config)?;
            }
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

fn migrate_config(config: &mut AppConfig) -> Result<bool> {
    match config.schema_version {
        CURRENT_SCHEMA_VERSION => Ok(false),
        1 => {
            let old_desktop_pair = config.action_for(GestureId::Left)
                == ActionKind::SwitchDesktopLeft
                && config.action_for(GestureId::Right) == ActionKind::SwitchDesktopRight;
            if old_desktop_pair {
                config.set_action(GestureId::Left, ActionKind::SwitchDesktopRight);
                config.set_action(GestureId::Right, ActionKind::SwitchDesktopLeft);
            }
            for gesture in [GestureId::Seven, GestureId::Circle] {
                if !config
                    .gesture_bindings
                    .iter()
                    .any(|binding| binding.gesture == gesture)
                {
                    config.set_action(gesture, default_action(gesture));
                }
            }
            config.schema_version = CURRENT_SCHEMA_VERSION;
            Ok(true)
        }
        version => bail!("不支持的配置版本 {version}"),
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
        assert_eq!(
            config.action_for(GestureId::LetterS),
            ActionKind::SearchSelection
        );
        assert_eq!(
            config.action_for(GestureId::Left),
            ActionKind::SwitchDesktopRight
        );
        assert_eq!(
            config.action_for(GestureId::Right),
            ActionKind::SwitchDesktopLeft
        );
        assert_eq!(config.action_for(GestureId::Seven), ActionKind::TaskManager);
        assert_eq!(config.action_for(GestureId::Circle), ActionKind::ScreenSnip);
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
            gesture: GestureId::LetterS,
            points,
        };
        assert!(sample.is_valid());
        let config = AppConfig {
            custom_gestures: vec![sample; 4],
            ..AppConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn gesture_bindings_are_customizable_and_reject_duplicates() {
        let mut config = AppConfig::default();
        config.set_action(GestureId::LetterC, ActionKind::Paste);
        assert_eq!(config.action_for(GestureId::LetterC), ActionKind::Paste);
        config.validate().unwrap();

        config.gesture_bindings.push(GestureBinding {
            gesture: GestureId::LetterC,
            action: ActionKind::CopySelection,
        });
        assert!(config.validate().is_err());
    }

    #[test]
    fn legacy_personalized_samples_keep_their_shape_after_upgrade() {
        let config: AppConfig = serde_json::from_str(
            r#"{
                "custom_gestures": [{
                    "action": "copy_selection",
                    "points": []
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(config.custom_gestures[0].gesture, GestureId::LetterC);
        assert_eq!(
            config.action_for(GestureId::LetterC),
            ActionKind::CopySelection
        );
    }

    #[test]
    fn schema_one_desktop_defaults_are_reversed_during_migration() {
        let mut config = AppConfig::default();
        config.schema_version = 1;
        config
            .gesture_bindings
            .retain(|binding| !matches!(binding.gesture, GestureId::Seven | GestureId::Circle));
        config.set_action(GestureId::Left, ActionKind::SwitchDesktopLeft);
        config.set_action(GestureId::Right, ActionKind::SwitchDesktopRight);

        assert!(migrate_config(&mut config).unwrap());
        assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(
            config.action_for(GestureId::Left),
            ActionKind::SwitchDesktopRight
        );
        assert_eq!(
            config.action_for(GestureId::Right),
            ActionKind::SwitchDesktopLeft
        );
        assert_eq!(config.action_for(GestureId::Seven), ActionKind::TaskManager);
        assert_eq!(config.action_for(GestureId::Circle), ActionKind::ScreenSnip);
    }

    #[test]
    fn migration_preserves_custom_desktop_actions() {
        let mut config = AppConfig::default();
        config.schema_version = 1;
        config.set_action(GestureId::Left, ActionKind::BrowserBack);
        config.set_action(GestureId::Right, ActionKind::BrowserForward);

        migrate_config(&mut config).unwrap();
        assert_eq!(config.action_for(GestureId::Left), ActionKind::BrowserBack);
        assert_eq!(
            config.action_for(GestureId::Right),
            ActionKind::BrowserForward
        );
    }
}
