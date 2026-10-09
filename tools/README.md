# 工程工具

- build/：前端资源暂存、Sites 构建组织、引擎副本与固定固件构建。
- release/：Windows MSI 发行、身份检查、许可证、签名与不可变版本账本。
- verify/：结构化模块检查、图谱生成、影响分析。
- lib/：工具共用路径与发行版本校验。

build/run-tauri.mjs 为桌面开发、构建与发行统一设置 backend/src-tauri/ 和仓库根 npm 钩子路径；Cargo 调用显式使用 backend/Cargo.toml，编译产物继续进入根 .cache/cargo/。

build/prepare-windows-icons.py 使用 Pillow 从保留的透明母版重建 Windows DPI 图标，ICO 的 256px 帧置于首位，避免 Tauri 将 16px 帧用于原生窗口。build/windows-icon.mjs 在资源暂存时检查该约束；常规构建不依赖 Python。UI 浅色/深色 Logo 与桌面图标使用各自的画布留白。

这些源码是构建或自动约束的必要输入，继续进入 Git。纯测试辅助程序归 tests/，按用户要求本机保留而不跟踪。工具均离线编排，不连接设备、不上传、不自动发布。签名密钥保持既有受保护用户目录，公钥和版本账本继续跟踪。

release/prepare-installer-ui.mjs 使用既有 MSVC/Windows SDK 构建 windows-folder-picker.cpp，并暂存到 backend/src-tauri/gen/。MSI 即时 UI 动作仅选择目录、写入安装会话属性；Windows Shell 提供随 DPI 缩放的目录图标。build/prepare-installer-images.py 仅用于维护安装器品牌 BMP，不参与常规构建。
