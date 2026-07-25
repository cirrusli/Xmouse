# Xmouse

Xmouse 是一个面向 Windows 10/11 x64 的轻量鼠标手势与剪贴板历史工具。它使用 Rust 和原生 Win32 API，不包含 WebView 或托管 UI 运行时。

0.3 版采用参考 Quicker 的现代原生界面：左侧设置导航、分区内容页、圆角输入控件和紧凑的剪贴板搜索面板，同时维持低资源占用。默认搜索引擎为 Google。

## 默认手势

| 手势 | 动作 |
| --- | --- |
| ↑ | 切换鼠标下窗口置顶状态 |
| L | 向目标窗口发送 `Ctrl+W` |
| S | 搜索选中文本并恢复原剪贴板 |
| C | 向目标窗口发送 `Ctrl+C` |
| V | 打开剪贴板历史 |

右键是默认触发键，也可以在设置中改为 X1/X2 侧键。普通短按会被重放，仍可使用正常右键菜单。

## Edge 鼠标手势冲突

Edge 的内置鼠标手势与 Xmouse 都使用“按住右键并拖动”，不建议同时启用。请在 Edge 打开 `edge://settings/appearance`，在“自定义浏览器”中关闭“启用鼠标手势”。受管理设备也可以将 `SOFTWARE\Policies\Microsoft\Edge` 下的 `MouseGestureEnabled` DWORD 策略设为 `0`。如果必须保留 Edge 手势，可将 Xmouse 触发键改为 X1 或 X2 侧键。

## 数据与隐私

- 历史保存在 `%LOCALAPPDATA%\Xmouse`。
- 新记录默认明文保存，便于个人调试：文本以 UTF-8 保存在 `history.db` 的 `plain_text` 列，图片以 PNG 保存在 `media` 目录。
- 可在“剪贴板历史”设置页启用 Windows DPAPI；启用后新记录按当前 Windows 用户加密。
- 每条历史记录保存自己的编码标记，因此旧的 DPAPI 记录与新的明文记录可以同时正常读取。
- 不包含遥测、云同步或应用内网络请求；S 手势仅把搜索 URL 交给系统默认浏览器。
- 会尊重 Windows 的 `ExcludeClipboardContentFromMonitorProcessing` 和 `CanIncludeInClipboardHistory` 标记。

## 构建

需要 Rust stable MSVC 或 GNU x64 工具链、Windows 资源编译器（MSVC `rc.exe` 或 GNU `windres` + C 预处理器）以及 SQLite 3。执行：

```powershell
cargo test
cargo build --release
```

Release 可执行文件位于 `target\release\xmouse.exe`。

## 便携版

解压 `Xmouse-0.3.0-windows-x64.zip` 后直接运行 `xmouse.exe`。请保留
`sqlite3.dll` 与可执行文件在同一目录。首次启动会打开设置页，并在通知区域
创建托盘图标；关闭设置页只会隐藏窗口，需从托盘菜单选择“退出 Xmouse”结束进程。

此 MVP 未包含代码签名、安装器、自动更新、跨权限输入、自动粘贴和自定义手势录制。
