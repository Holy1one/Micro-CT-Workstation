# Micro-CT Workstation

Micro-CT-App 是 RTS9060 Micro-CT 的桌面控制工程：Tauri 2 外壳 + React 控制台 + Rust `ct-engine` 状态机，直接驱动真实转台（Nano/CH340）、Moxtek 12 W 射线源与 Nikon D7100 相机。

## 当前能力

- 设备链路：自动识别 Nano 与 Moxtek 串口，连接状态与遥测实时回读。
- 射线控制：射线源连接成功即可操作，无需预检与回零，可脱离 CT 系统单独控制。
- USB Auto Shut Down（设备端死开关）：勾选即把死开关武装到设备，禁止开束，主机失联后设备按设定延时自动关束；释放后由 SEND V / SEND I 确认设定值，确认完成即可手动开束，出束期间锁定设定值修改。
- 遗留束流检测：连接时读取硬件真实出束状态；上一进程被强杀留下的出束会如实上报，开关显示红色，操作员直接关闭。
- 退出安全链：窗口关闭、外壳崩溃或进程被杀都会停止扫描并强制关束。
- 扫描：N 视角等分旋转 + 曝光 + 拍照，单次串口抖动只影响当前命令，序列继续。
- 打包：`npx tauri build --no-bundle` 产出免安装 `ct-workstation.exe`（内嵌 `ct-engine` sidecar）。

## 当前边界

- `ct-engine` 是唯一的业务状态与命令裁决者。
- Tauri 只管理窗口、前端 IPC 与 engine 子进程生命周期。
- 浏览器预览使用显式开发适配器，界面持续显示预览标识。
- 急停与联锁以硬件为准，软件状态仅作显示与记录。
- 3D Viewer、FBP、材料分辨、Python 插件与 digiCamControl 仍待实现。

## Windows 入口

Windows 11 x64 是真实设备集成与最终发布平台。安装 MSVC Build Tools、WebView2、Rust `x86_64-pc-windows-msvc`、Node 与 .NET 后运行 `npm run tauri:dev`。相机、串口、射线、CUDA 与免安装包在目标 Windows 工作站验收，详见 `docs/WINDOWS-INTEGRATION.md`。

## 开发

```bash
npm install
npm run dev -- --port 4173 --strictPort   # 浏览器界面与流程开发
npm run tauri:dev                          # 原生桌面应用（需 Rust stable）
```

验证：

```bash
npm test
npm run build
npm run typecheck
npm run test:sites
cargo test --workspace
cargo check --workspace
```

## 目录

- `src/`：React/TypeScript UI 与开发预览适配器。
- `crates/ct-engine/`：独立 Rust 状态机和 JSONL IPC 服务。
- `src-tauri/`：薄桌面适配层。
- `portable-release/`：免安装桌面应用。
- `algorithm-plugins/`：未来算法插件契约。
- `bridges/digicam/`：未来 Windows 相机桥契约。
- `docs/`：架构、平台集成与验收记录。
