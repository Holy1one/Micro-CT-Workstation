# 交给 Windows Codex 的提示词

> **历史交接提示 / 禁止直接执行。** 本文包含旧机器路径和旧阶段目标，仅保留用于追溯。当前任务入口为仓库 `AGENTS.md`、`docs/architecture/README.md` 和实际源码。

你现在接手 Windows 端的 Micro-CT-App 基线构建。项目路径是：

`E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App`

先完整读取项目根目录及上层适用的 `AGENTS.md`，然后读取：

- `docs/CURRENT-PROGRESS.md`
- `docs/ARCHITECTURE.md`
- `docs/WINDOWS-INTEGRATION.md`
- `README.md`

目标：在 Windows 11 x64 上复现并验收当前桌面基线，建立后续真实设备适配的正确入口。当前任务仅做到 Windows 可构建、可运行、开发预览交互一致，以及设备 adapter/bridge 的边界检查；不要启用真实 X-ray，不要把模拟状态自动降级进生产路径，不要改写已经定稿的三栏固定工作台布局。

按以下顺序执行：

1. 检查 Node、Rust MSVC、Visual Studio Build Tools、WebView2 与 Tauri prerequisites，记录实际版本。
2. 运行 `npm ci`、`npm test`、`npm run build`、`cargo test --workspace`。
3. 运行 `npm run engine:build`，确认生成 `src-tauri\binaries\ct-engine-x86_64-pc-windows-msvc.exe`，再执行 `npx tauri build --debug --no-bundle`。
4. 启动 debug 桌面程序，检查 1600×900 和 Windows 缩放 100%/125%：不能出现整页滚动，所有主区、底栏和状态栏必须同屏。
5. 完整执行开发预览：连接 → 预检 → 回零 → 开始 → 暂停 → 继续 → 紧急停止；再执行“加载上次进度 → 紧急停止”。确认 STOP 使预检/回零失效，X-ray 始终闭锁，Image Preview 不出现伪造图片。
6. 检查 `bridges/digicam/` 与现有 Windows digiCamControl/.NET 环境，给出最小 bridge 方案和版本化消息契约；本轮不要让 bridge 自行推进扫描，也不要在 UI 里直接调用 .NET SDK。
7. 检查未来 Nano、Nikon、Moxtek adapter 的落点。保留 `ct-engine` 为唯一设备和任务所有者。真实硬件动作、相机拍摄和任何 X-ray 输出必须另行获得明确授权后再执行。
8. 把 Windows 实测结果、命令、版本、失败信息和未完成项追加到 `docs/CURRENT-PROGRESS.md` 的“Windows 验证”章节；只写实际证据，不把编译通过描述成硬件验收。

架构约束：第一版继续使用 Tauri sidecar stdio + 版本化 JSONL envelope，不要为了形式提前改 Named Pipe；未来只有在 engine 需要独立服务生命周期时再切换传输。Rust 内核负责领域状态和安全门控；受 Windows SDK 限制的相机功能用独立 .NET bridge；FBP/用户重建/材料分辨用独立 Python worker 与 manifest，不动态加载进 engine。

完成标准：Windows debug build 成功、交互链路通过、生产默认闭锁、无伪造图像、无整页滚动、文档写明证据与限制。遇到真实设备或安全边界时先停在可编译 adapter 契约，不猜协议、不试探未知命令。
