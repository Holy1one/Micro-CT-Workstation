# 静态资源

本目录只维护关键 Logo、运行字体和配置实际引用的打包图标。

- micro-ct-logo.png：用户于 2026-10-08 提供的 4096×4096 RGBA 原图，保留原始字节与真实透明通道，作为浅色 Logo 母版。
- micro-ct-logo-dark.png：用户授权本地精确调色的深色母版；仅提亮中央 T、压暗右侧梯形，外圈像素、轮廓和透明通道保持不变。app-logo-light-256.png、app-logo-dark-256.png 是两款主题的轻量运行衍生图，使用一致的原始画布留白。
- app-32x32.png、app-128x128.png、app-128x128@2x.png、app-icon.ico：Windows 桌面图标，裁去母版多余透明边距，内容占最长边约 88%，各尺寸直接从母版以预乘透明度缩小。ICO 的第一帧必须为 256px：Tauri 2.11.5 直接解码第一帧用于原生窗口，16px 第一帧会被任务栏放大而模糊。其余帧覆盖 192、128、96、80、64、48、40、32、24、20、16px。全部为 32 位 RGBA，不添加底板。
- app-icon.icns：既有母版的 macOS 配置资源，当前仅交付 Windows MSI。
- installer-banner.bmp、installer-dialog.bmp：MSI 标题区和侧边品牌图，使用本项目 Logo；按 WiX 标准画布的 3 倍尺寸提供高 DPI 像素，24 位 BMP 格式用于 MSI Bitmap 控件，白底与安装器页面一致。维护时运行 python tools/build/prepare-installer-images.py；常规打包不依赖 Python。
- inter-var.woff2、jetbrains-mono-var.woff2：既有运行字体，保留原始字节与来源。

tools/build/prepare-assets.mjs 将浅色和深色 256px 透明图标分别暂存为 .cache/frontend/public/assets/micro-ct-logo.png、micro-ct-logo-dark.png，并复制两款字体；应用菜单栏和网页 favicon 跟随实际主题自动切换，4096px 母版不进入应用包。主题保留显式用户选择，否则跟随系统。Windows 安装快捷方式的静态图标继续使用透明原图。

原生窗口由 backend/src-tauri/src/windows_icons.rs 使用 Windows LoadImageW 从 EXE 的 32512 图标资源分别加载大/小尺寸，按窗口 DPI 刷新，由 Windows 完成透明边缘解码和尺寸选择。仅销毁本模块持有的替换句柄，不释放框架句柄。

重建 Windows 图标：从仓库根执行 python tools/build/prepare-windows-icons.py（仅维护时需要 Pillow）。常规开发和打包仍只需要既有 Node/Rust 工具链；prepare-assets.mjs 自动检查第一帧、DPI 尺寸和透明格式。不要直接用 Tauri CLI 或 Pillow 的默认 ICO 帧顺序覆盖本目录。原始浅色/深色母版字节保持不变。

实现依据：[Microsoft Windows app icon construction](https://learn.microsoft.com/en-us/windows/apps/design/style/iconography/app-icon-construction)、[Tauri 2.11.5 icon codegen](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri-codegen/src/image.rs)。

历史界面截图统一留在 .cache/screenshots/，不作为维护素材或运行资源。既有静态三维降级图、旧设备示意图和未使用 dock SVG 已退出；不继续向本目录写入截图。Logo 授权来源为本次用户提供的附件；字体来源沿用迁移前历史。
