# Xmouse 0.15.2

L 手势现在支持关闭 Steam 窗口。实机对照确认：Steam 主窗口由 `steamwebhelper.exe` 承载，`Ctrl+W` 无效，标题栏关闭按钮可将窗口收回托盘；用户实体键盘测试也得到相同结果。这属于动作兼容范围不足，不是手势识别或提权问题。

对 `steam.exe` 和 `steamwebhelper.exe` 的当前顶层窗口发送标准 `WM_CLOSE`，由 Steam 决定关闭窗口或收回托盘。不结束 Steam 进程，不向游戏或其他进程应用该兼容规则；Steam 的聊天/设置子窗口不转而关闭主所有者窗口。其他程序保持 `Ctrl+W`，Wallpaper UI 现有兼容路径保留。

64 项常规测试、格式检查、Clippy 与 Release 构建通过；新增测试排除 `steamservice.exe`、`GameOverlayUI.exe`、游戏进程及浏览器误命中。2026-10-31 部署后，用户实体 L 手势确认已能关闭 Steam 窗口；窗口枚举确认主窗口收起，原 `steam.exe` 进程仍正常运行。

本轮另审查了外部 SQLite DLL、产品定位和键盘/无障碍体验，详见 [评审与迭代计划](TECHNICAL-REVIEW-2026-10-31.md)。其中 SQLite 单文件发布、键盘与 UIA 改造为后续迭代建议，本版未修改这些行为。
