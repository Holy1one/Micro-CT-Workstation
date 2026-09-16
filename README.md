# Micro-CT Workstation

Micro-CT-App 是新的桌面表现层与控制编排工程。V1 提供可操作的设备状态、预检、回零、参数设置、扫描流程预览、停止和日志界面；它不连接真实设备，不产生投影，不启用射线。

## 当前边界

- `ct-engine` 是唯一的业务状态与命令裁决者。
- Tauri 只管理窗口、前端 IPC 和 engine 子进程生命周期。
- 浏览器预览使用显式开发适配器，界面持续显示“未连接真实设备”。
- 射线通道默认闭锁。软件状态不替代硬件急停与联锁。
- 3D Viewer、FBP、材料分辨、Python 插件与 digiCamControl 不在 V1 实现范围内。

## Mac 开发

```bash
npm install
npm run dev -- --port 4173 --strictPort
```

浏览器开发模式用于界面和流程演示。运行原生桌面应用前需要 Rust stable：

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
npm run tauri:dev
```

验证：

```bash
npm test
npm run build
cargo test --workspace
cargo check --workspace
```

## Windows 入口

Windows 11 x64 是真实设备集成和最终发布平台。安装 MSVC Build Tools、WebView2、Rust `x86_64-pc-windows-msvc`、Node 与 .NET 后运行 `npm run tauri:dev`。相机、串口、射线、CUDA 和安装包必须在目标 Windows 工作站验收，详见 `docs/WINDOWS-INTEGRATION.md`。

## 目录

- `src/`：React/TypeScript UI 与开发预览适配器。
- `crates/ct-engine/`：独立 Rust 状态机和 JSONL IPC 服务。
- `src-tauri/`：薄桌面适配层。
- `algorithm-plugins/`：未来算法插件契约。
- `bridges/digicam/`：未来 Windows 相机桥契约。
- `docs/`：架构与平台集成说明。
