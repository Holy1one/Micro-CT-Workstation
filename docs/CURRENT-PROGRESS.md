# 当前进度（2026-09-16）

## 已完成

- 新建 `Micro-CT-App`：Tauri 2 + React/TypeScript + Rust `ct-engine` sidecar。
- UI 已按 `前端/首页.png` 与 `前端/按钮说明.png` 重做为固定 16:9 工业工作台：无整页滚动、左侧设备与参数、中央设备场景、右侧射线/运行状态、底部日志与影像区。
- 中央四键控制舱默认收拢，悬停或聚焦展开；支持回零、开始/暂停/继续、开发预览进度恢复、紧急停止。
- 开发预览支持连接、预检、回零、无射线流程、暂停/继续、恢复与停止；STOP 会使预检和回零失效。
- X-ray 始终 fail-closed；未实现真实设备命令。
- 开发预览不生成或伪造投影，真实影像计数保持 0。
- Windows、digiCamControl、Python 插件、FBP、材料分辨和 3D Viewer 均只有边界与目录预留，尚未实现。

## 验证结果

- `npm run typecheck`：通过。
- `npm run build`：验证时通过；生成的 `dist/` 已在交接清理中删除，可按需重建。
- `npm run test:sites`：4/4 通过。
- `cargo test --workspace`：8/8 通过。
- `npm run engine:build` 与 `npx tauri build --debug --no-bundle`：验证时通过；macOS sidecar 和 `target/` 构建产物已在交接清理中删除，可按需重建。
- Codex in-app browser 1280 × 720：所有主要区域同屏可见，无整页滚动。
- 交互链路已验：连接 → 预检 → 回零 → 开始 → 暂停；恢复进度 → 紧急停止；X-ray 全程闭锁。

## 重要文件

- `src/App.tsx`：固定工业工作台与交互。
- `src/styles.css`：固定布局、工业视觉与悬浮控制舱。
- `src/engine/`：前端 adapter 契约与开发预览。
- `crates/ct-engine/`：Rust 领域状态机与 JSONL 协议。
- `src-tauri/`：桌面壳、sidecar 生命周期与 IPC。
- `public/assets/micro-ct-equipment-scene-v2.png`：中心设备场景素材。
- `docs/ARCHITECTURE.md`：长期混合架构决策。
- `docs/WINDOWS-INTEGRATION.md`：Windows 验收边界。

## 当前限制

- macOS 结果只证明 UI、协议与纯逻辑，不证明 Windows WebView2、设备驱动、CUDA 或安装包。
- UI 中的 X-ray 数值、Timer 和 Max X-ray 是第一版交互占位，未进入真实控制命令。
- “恢复进度”目前只恢复开发预览检查点，没有读取持久任务文件。
- 真实相机图片到达前，Image Preview 只显示空槽。

## Windows 验证

实测日期：2026-09-16。目标为 Windows 11 x64 桌面基线；未连接、枚举或驱动任何真实设备，未执行相机拍摄或 X-ray 输出。

### 环境证据

- OS：`cmd /c ver` 返回 `10.0.26200.9445`；注册表 `DisplayVersion=25H2`、`CurrentBuild=26200`、`UBR=9445`。注册表 `ProductName` 仍返回旧标签 `Windows 10 Pro`，本记录保留该差异，不据此改写系统版本。
- Node.js `v24.13.1`，npm `11.8.0`。PowerShell 执行策略阻止 `npm.ps1`，本次使用同一安装目录的 `npm.cmd`/`npx.cmd`。
- Rust stable `1.98.1 (48a229cea 2026-09-01)`，Cargo `1.98.1`，host/target `x86_64-pc-windows-msvc`。
- Visual Studio Build Tools 2026 `18.0.2`（installation version `18.0.11222.15`）；MSVC `cl 19.50.35719`、link `14.50.35719.0`；Windows 11 SDK `10.0.26100.0`。
- Microsoft Edge WebView2 Runtime `153.0.4234.32`；运行中的 WebView2 user agent 同为 `Edg/153.0.4234.32`。
- .NET：x64 host/runtime `8.0.11`，Windows Desktop Runtime `8.0.11`；.NET Framework `4.8.09221`（release `533509`）；没有安装 .NET SDK。

### 构建与测试

- `npm ci`：成功，按 lockfile 安装 74 个包。沙箱内首次运行因 registry 请求 `EACCES` 并触发 npm `Exit handler never called`；允许联网并把 cache 放到 `%TEMP%\micro-ct-npm-cache` 后成功。
- `npm test`：最终 4/4 通过（含 `tsc --noEmit`）。一次与 `npm run build` 并行的检查在 `dist/client/index.html` 生成前触发，临时为 3/4；build 完成后顺序复跑通过，不是产品测试失败。
- `npm run build`：通过，Vite 转换 4575 个模块；生成 `dist/client/index.html`、`dist/server/index.js` 和 `dist/.openai/hosting.json`。
- `cargo test --workspace`：最终 8/8 通过。干净目录首次运行在 Tauri build script 检查 sidecar 时报告 `binaries\ct-engine-x86_64-pc-windows-msvc.exe doesn't exist`；按项目脚本生成 sidecar 后复跑通过。Cargo 首次取依赖还受用户级失效代理 `127.0.0.1:7892` 影响，本次仅对下载命令临时清空代理，没有修改全局配置。
- `npm run engine:build`：通过，生成 `src-tauri\binaries\ct-engine-x86_64-pc-windows-msvc.exe`，实测大小 693,248 B。
- `npx tauri build --debug --no-bundle`：通过；产物为 `target\debug\ct-workstation.exe`。
- debug 桌面程序成功启动，窗口标题 `Micro-CT Workstation`，同时启动同目录 `ct-engine.exe` sidecar，WebView2 页面为 `http://tauri.localhost/`。

### 布局与交互

- 100%：原生窗口 WebView 客户区实测 `1600 × 900`、`devicePixelRatio=1`。document/body 的 `scrollWidth × scrollHeight` 均为 `1600 × 900`，滚动偏移为 0；主区、左右栏、中央场景、188 px 底栏和 24 px 状态栏全部在 viewport 内。
- 125%：两块物理显示器都只报告 96 DPI，因此未改用户系统设置；改用同一 WebView2 Runtime 的 `--force-device-scale-factor=1.25` 验证。实测 `devicePixelRatio=1.25`、有效 CSS viewport `1280 × 720`，document/body 均无溢出，三栏、166 px 底栏和 22 px 状态栏全部同屏。此结果证明 WebView2 缩放布局，不替代在真实 125% 显示器上的最终人工验收。
- Windows debug 桌面中完整执行：连接 → 预检 → 回零 → 开始 → 暂停 → 继续 → 紧急停止；随后执行加载上次进度 → 紧急停止。每一步均到达预期 phase。
- 两次 STOP 后预检从 `PASSED` 失效为 `READY`，Home 与 Start 均禁用；继续前必须重新预检和回零。
- 全流程 X-ray Enable 均禁用，底栏保持 `X-RAY SAFE / DISABLED`；`imageCount=0`，五个 Image Preview 槽均为 `Awaiting capture`，影像区没有图片元素。
- `ct-engine.exe` 不带 `--preview` 直接启动时，开发预览连接返回 `PRODUCTION_LOCKED`；snapshot 为 `production_locked`、`xrayEnabled=false`。生产路径没有自动降级到模拟状态，STOP 仍可幂等执行。

### digiCamControl 与 adapter 边界

- Windows 卸载注册表记录 digiCamControl `2.1.7.0`，缓存安装包为 `C:\ProgramData\Package Cache\{c33d2323-4e2f-4b42-92a7-9f89f14ea398}\digiCamControlsetup_2.1.7.0.exe`。
- 注册的应用目录 `E:\Application\digiCamControl` 当前为空，常见路径中没有 `CameraControl.exe` 或 `CameraControl.Core.dll`；没有运行中的 digiCamControl 进程/服务。结合“没有 .NET SDK”，本机当前不能构建或验证真实 .NET bridge，也没有证据可选择 SDK 目标框架。
- 最小 bridge 保持独立 .NET 进程、UTF-8 JSONL stdio 和版本化 envelope，只接受 engine 发出的 discover/connect/health/configure/capture/stop/disconnect；文件大小、SHA-256、远端删除确认与设置快照必须回传。详细 v1 契约已写入 `bridges/digicam/README.md`。
- Nano、Nikon、Moxtek 的后续 Rust adapter 落点为 `crates/ct-engine/src/adapters/`；`ct-engine` 继续作为唯一设备/任务所有者。未获得实际 SDK/协议与明确硬件授权前不创建可启用实现，不猜测命令。

### 未完成项

- 尚未在真实 Windows 125% 显示设置下人工复核；本轮使用 WebView2 强制 DPR 1.25。
- digiCamControl 主程序/SDK 程序集和 .NET SDK 缺失，bridge 只有契约，没有可执行文件；Nikon 发现、连接、拍摄、传输校验和远端删除均未测试。
- Nano 串口、Moxtek、硬件联锁、真实急停链、相机、CUDA、Python worker、代码签名和安装包均未验收。本次“通过”只表示 Windows debug 软件基线、开发预览状态机和默认闭锁行为通过，不表示硬件验收。

### Windows UI 重构复验（2026-09-16）

- 重新逐页检查 `前端/前端设计.pptx` 全部 4 页以及 `前端/首页.png`、`前端/按钮说明.png`，按参考的信息架构重构，而非照搬渲染图的卡片视觉。
- 移除重复的自绘应用标题栏，原生 Windows 标题栏下直接进入 `File / Edit / Tools / Help` 菜单行；可见界面、状态、错误和日志统一为英文。
- 三栏在 1600 × 900 下实测宽度约为 381 / 814 / 381 px；左右栏由原 292 px 加宽，中间场景缩小且图片使用 `contain`，避免挤压设备与控制信息。
- 字体提升到工作台可读尺寸：应用正文 13 px、设备名称 12 px、日志 11 px；125% 验证下分别为 12 / 11 / 10 px。
- 三台设备各有独立状态灯和 Refresh 控件；审计确认 `.device-refresh` 数量为 3。当前刷新只重读 engine snapshot，不伪造独立硬件连接结果。
- 底栏恢复为 5 个选择项。默认日志使用浅色工业配色和无分隔线的终端式连续行，时间格式为 `YYYY-MM-DD HH:mm:ss`；Image Preview 只在第五项激活后替换日志内容。
- 修复 React 条件分支复用导致的日志残影；日志与图片区使用独立 key。Image Preview 激活后日志容器不存在，5 个空槽强制单行：100% 下每槽约 269.8 px，125% 下每槽约 209.8 px；真实图片数仍为 0。
- 右下仅显示 Operation Status、流程节点和当前 Task / Scan setup / Safety 摘要，不再常驻 Image Preview。
- 使用 `前端/logo.png` 居中补白为正方形图标源，通过 `npx tauri icon` 生成 Windows `icon.ico` 等资源；从最终 `ct-workstation.exe` 提取的关联图标已目视确认一致。
- 1600 × 900 / DPR 1.0 严格几何审计通过：document、主区、左右栏、中央场景、四个侧栏面板、底栏、日志导航和日志内容均无 `scrollWidth/clientWidth` 或 `scrollHeight/clientHeight` 溢出。
- 1280 × 720 / DPR 1.25 严格几何审计通过：左侧设备与参数面板客户高度均为 250 px，参数区 `scrollHeight=clientHeight=250`，Apply Parameters 完整可见；其余所有主面板同样无滚动或隐藏截断。
- 最终 Windows debug 中复验：逐设备刷新 ×3 → 连接 → 预检 → 回零 → 开始 → 暂停 → 继续 → STOP → 加载上次进度 → STOP。两次 STOP 后预检和回零均失效，Home/Start 禁用，X-ray 保持 `SAFE / DISABLED`，Image Preview 没有伪造图片。
