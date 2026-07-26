# Xmouse

Xmouse 是面向 Windows 10/11 x64 的轻量鼠标手势与剪贴板历史工具。它使用 Rust 和原生 Win32 API，不包含 WebView 或托管 UI 运行时。

当前版本：**0.5.0**

## 快速启动

双击根目录的 `启动 Xmouse.cmd`。脚本会从 `latest\Xmouse.exe` 启动当前最新版。

便携版也可以直接解压 `outputs\Xmouse-0.5.0-windows-x64.zip` 后运行 `Xmouse.exe`。请保留 `sqlite3.dll` 与程序在同一目录。

## 目录结构

| 路径 | 用途 |
| --- | --- |
| `启动 Xmouse.cmd` | 启动 `latest` 中的当前版本 |
| `latest\` | 当前可直接运行的便携版 |
| `archive\v0.x.x\` | 历史版本包、校验文件和截图 |
| `outputs\` | 当前版本的对外交付物 |
| `src\`、`assets\` | Rust 源码和资源 |
| `docs\` | 当前版本说明 |
| `work\`、`target\` | 构建缓存与中间文件 |

`latest`、`archive` 和 `outputs` 中的二进制交付物不会提交到 Git；版本历史由 Git 标签和 `CHANGELOG.md` 共同记录。

## 项目文档

- 产品需求、技术选型与后续路线：`docs\PRODUCT-REQUIREMENTS-AND-ROADMAP.md`
- 工程架构与模块边界：`docs\ARCHITECTURE.md`
- 当前版本说明：`docs\RELEASE-NOTES-0.5.0.md`
- 完整版本历史：`CHANGELOG.md`

## 默认手势

| 手势 | 动作 |
| --- | --- |
| ↑ | 切换鼠标下窗口的置顶状态 |
| L | 向目标窗口发送 `Ctrl+W` |
| S | 搜索选中文本并恢复原剪贴板 |
| C | 向目标窗口发送 `Ctrl+C` |
| V | 打开剪贴板历史 |

右键是默认触发键，也可以在设置中改为 X1/X2 侧键。普通短按会被重放，仍可使用正常右键菜单；轨迹过短时也按普通单击处理，不再弹出“手势太快”提示。

## 界面

- 设置页和剪贴板历史支持浅色、深色模式同步切换。
- 常用设置只保留启用状态、开机启动、触发键、历史记录和本机加密。
- “资源占用”页显示当前进程的 CPU、GPU、私有内存、工作集、句柄和运行时间。
- 剪贴板历史页及弹窗右上角显示记录总数与实际磁盘占用。
- 常用记录可以置顶；置顶项优先显示，并且不会被普通数量或容量淘汰自动删除。
- 提示条固定显示在鼠标所在显示器的底部居中位置。

## Edge 鼠标手势冲突

Edge 内置手势与 Xmouse 都会使用“按住右键并拖动”，不建议同时启用。请在 Edge 打开 `edge://settings/appearance`，在“自定义浏览器”中关闭“启用鼠标手势”。如果必须保留 Edge 手势，可将 Xmouse 触发键改为 X1 或 X2。

受管理设备也可以将 `SOFTWARE\Policies\Microsoft\Edge` 下的 `MouseGestureEnabled` DWORD 策略设为 `0`。

## 数据与隐私

- 历史保存在 `%LOCALAPPDATA%\Xmouse`。
- 新记录默认明文保存：文本位于 `history.db` 的 `plain_text` 列，图片以 PNG 保存在 `media` 目录，便于个人调试。
- “本机加密”是可选的 Windows DPAPI 保护；启用后，仅新记录按当前 Windows 用户加密。
- 不包含遥测、云同步或应用内网络请求。S 手势只把搜索 URL 交给系统默认浏览器。
- 会尊重 Windows 的 `ExcludeClipboardContentFromMonitorProcessing` 和 `CanIncludeInClipboardHistory` 标记。

## 构建

需要 Rust stable x64 工具链、Windows 资源编译器和 SQLite 3。执行：

```powershell
cargo test
cargo build --release
```

Release 可执行文件位于 `target\release\xmouse.exe`。当前版本说明见 `docs\RELEASE-NOTES-0.5.0.md`，完整变更历史见 `CHANGELOG.md`。

首次启动会打开设置页并创建托盘图标。关闭设置页只会隐藏窗口；需要从托盘菜单选择“退出 Xmouse”结束进程。
