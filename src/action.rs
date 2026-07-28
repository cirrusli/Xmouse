use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Disabled,
    ToggleTopmost,
    CloseTab,
    SearchSelection,
    CopySelection,
    OpenHistory,
    SwitchDesktopLeft,
    SwitchDesktopRight,
    Paste,
    BrowserBack,
    BrowserForward,
    MinimizeWindow,
    MaximizeRestore,
    ShowDesktop,
    TaskView,
    TaskManager,
    ScreenSnip,
    VolumeMute,
    VolumeDown,
    VolumeUp,
    MediaPlayPause,
}

impl ActionKind {
    pub const ALL: [Self; 21] = [
        Self::Disabled,
        Self::ToggleTopmost,
        Self::CloseTab,
        Self::SearchSelection,
        Self::CopySelection,
        Self::OpenHistory,
        Self::Paste,
        Self::BrowserBack,
        Self::BrowserForward,
        Self::MinimizeWindow,
        Self::MaximizeRestore,
        Self::SwitchDesktopLeft,
        Self::SwitchDesktopRight,
        Self::ShowDesktop,
        Self::TaskView,
        Self::TaskManager,
        Self::ScreenSnip,
        Self::VolumeMute,
        Self::VolumeDown,
        Self::VolumeUp,
        Self::MediaPlayPause,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Disabled => "不执行动作",
            Self::ToggleTopmost => "置顶 / 取消置顶窗口",
            Self::CloseTab => "关闭当前标签页",
            Self::SearchSelection => "搜索选中文本",
            Self::CopySelection => "复制选中内容",
            Self::OpenHistory => "打开剪贴板历史",
            Self::SwitchDesktopLeft => "切换到左侧桌面",
            Self::SwitchDesktopRight => "切换到右侧桌面",
            Self::Paste => "粘贴",
            Self::BrowserBack => "浏览器后退",
            Self::BrowserForward => "浏览器前进",
            Self::MinimizeWindow => "最小化窗口",
            Self::MaximizeRestore => "最大化 / 还原窗口",
            Self::ShowDesktop => "显示桌面",
            Self::TaskView => "任务视图",
            Self::TaskManager => "打开任务管理器",
            Self::ScreenSnip => "打开截图工具",
            Self::VolumeMute => "静音 / 取消静音",
            Self::VolumeDown => "降低音量",
            Self::VolumeUp => "提高音量",
            Self::MediaPlayPause => "播放 / 暂停媒体",
        }
    }
}
