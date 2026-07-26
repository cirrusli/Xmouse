# Xmouse 0.8.0

## 关于页

设置侧边栏新增“关于”页，以三个简洁卡片集中展示：

- 当前版本、Windows 支持范围和 Rust + Win32 技术选型。
- Xmouse 的低资源占用、本地数据管理与无 WebView 特性。
- ↑、L、S、C、V 默认手势和基本使用步骤。
- 作者 `cirrusli`、个人 GitHub 与 Xmouse 项目地址。

“访问 GitHub”按钮会把 `https://github.com/cirrusli` 交给系统默认浏览器，Xmouse 自身不会发起网络请求。关于页会跟随浅色、深色主题切换，并隐藏无关的“保存设置”按钮。

## 开源元数据

- Cargo 作者设置为 `cirrusli`。
- 仓库与主页设置为 `https://github.com/cirrusli/Xmouse`。
- MIT 许可证版权信息更新为 `cirrusli`。

## 验证

- 22 项常规单元测试通过，真实剪贴板往返测试保留为按需测试。
- `cargo check`、`cargo test --release`、`cargo clippy --release -- -D warnings` 和 `cargo build --release` 通过。
- 浅色和深色“关于”页完成实际窗口检查，文字、卡片和按钮均无截断。
