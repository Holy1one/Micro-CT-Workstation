# Micro-CT Workstation

Windows 桌面 Micro-CT 上位机。React 展示状态并发送命令，Tauri 提供桌面能力，Rust ct-engine 统一控制 Moxtek、Nikon D7100 和 Nano 转台。

## 开发与构建

在仓库根目录执行：

~~~powershell
npm.cmd ci
npm.cmd run dev -- --port 4173 --strictPort
npm.cmd run tauri:dev
npm.cmd run installer:build
~~~

Windows x64 安装包兼容目标为 Windows 10 和 Windows 11，使用 MSI 提供目录选择、开始菜单/桌面快捷方式，以及修改、修复、卸载。installer:build 输出 release/micro-ct-workstation_<version>_windows_x86_64.msi，该目录只保留最新安装包；构建来源、签名和许可证在 .cache/tests/release/<version>/，许可证同时安装。随包提供 WebView2 引导安装器（缺少运行时时自动联网安装）；开发构建需要 MSVC Build Tools、Rust MSVC toolchain 和 Node.js。网页 build 只更新前端构建输出。安装须由用户自行启动。

## 目录与资料

- [AGENTS.md](AGENTS.md)：规则和按任务读取顺序。
- [docs/architecture/README.md](docs/architecture/README.md)：控制链、接口与结构化约束。
- [docs/architecture/directory-map.md](docs/architecture/directory-map.md)：逐目录职责。
- [docs/guides/windows-integration.md](docs/guides/windows-integration.md)：Windows 集成。
- [docs/guides/desktop-update-distribution.md](docs/guides/desktop-update-distribution.md)：发行、签名和升级。
- [docs/validation/README.md](docs/validation/README.md)：验收摘要与证据边界。

关键 Logo 母版、字体和打包图标位于 assets/。运行时只暂存 256px 透明 Logo 与字体到 .cache/frontend/public/assets/，原图和历史截图不进入应用包；历史界面截图留在 .cache/screenshots/。

前端入口和构建配置集中在 frontend/{index.html,tsconfig.json,vite.config.mjs}。Rust workspace 位于 backend/{Cargo.toml,Cargo.lock}，领域引擎与桌面壳分别位于 backend/ct-engine/ 和 backend/src-tauri/。从仓库根运行 npm 脚本即可；直接执行 Cargo 时指定 --manifest-path backend/Cargo.toml，或进入 backend/。根目录保留 npm、Git 入口及规则和导航文档；.cargo/config.toml 统一全仓编译缓存位置。

## 离线验证

~~~powershell
npm.cmd run module-map:check
npm.cmd run typecheck
npm.cmd run build
cargo test --manifest-path backend/Cargo.toml --workspace
~~~

tests/ 本机保留，但按用户要求不进入 Git。完整的本机验证入口为 npm.cmd run verify；前端、桌面和固件测试入口分别为 test:frontend、test:desktop、firmware:test。全新检出包含必要构建、发布和架构检查工具；本地外部测试套件需要单独提供。crate 内单元测试随源码保留。

缓存、日志、截图和测试运行输出均位于 .cache/。原始 NEF、扫描清单与任务持久数据使用应用既有保存位置，不属于工程缓存清理范围。真实设备验收须另有明确授权与本次设备证据。
