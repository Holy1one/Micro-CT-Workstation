# Micro-CT Workstation

Micro-CT-App 是 Windows 工业上位机工程：React 提供操作界面，Tauri 提供桌面能力，Rust `ct-engine` 统一控制 Moxtek 射线源、Nikon D7100 相机和 RTS9060 Nano 转台。

## 架构原则

- `ct-engine` 是设备连接、安全门控、扫描状态和投影事务的唯一权威。
- Tauri 只管理窗口、原生路径/对话框、sidecar 生命周期和 IPC 转发。
- React 只发送领域命令并渲染完整快照；3D 场景只读展示。
- 浏览器开发预览完全离线，不能访问真实设备；Tauri IPC 失败时不能回退到预览。
- 每个投影在转台到位、出束/关束确认、主机侧图像文件确认和 `CAPTURE_DONE` 全部成功后才提交。

完整架构入口见 [`docs/architecture/README.md`](docs/architecture/README.md)，逐目录说明见 [`docs/architecture/directory-map.md`](docs/architecture/directory-map.md)，PC 直控与未来 Arduino 主控双线方案见 [`docs/architecture/dual-control-topology.md`](docs/architecture/dual-control-topology.md)。

## 核心模块

- `crates/ct-engine/src/devices/xray/`：Moxtek 通信、实测回读和 fail-closed 关束。
- `crates/ct-engine/src/devices/camera/`：D7100 / DigiCamControl 和主机文件确认。
- `crates/ct-engine/src/devices/turntable/`：Nano 协议、运动确认、警告和急停。
- `crates/ct-engine/src/scan.rs`：跨设备投影事务编排。
- `src-tauri/`：薄桌面壳和 versioned JSONL sidecar 客户端。
- `src/engine/`：前端契约、生产 adapter 和浏览器预览。
- `module-map/`：机器可读模块归属与隐藏依赖。

## Windows 入口

Windows 11 x64 是真实设备集成与最终发布平台。安装 MSVC Build Tools、WebView2、Rust `x86_64-pc-windows-msvc`、Node 与 .NET 后运行 `npm.cmd run tauri:dev`。真实相机、串口和射线操作必须取得本次明确授权，详见 `docs/WINDOWS-INTEGRATION.md`。

## 开发

```powershell
npm.cmd ci
npm.cmd run dev -- --port 4173 --strictPort
npm.cmd run tauri:dev
```

验证：

```powershell
npm.cmd run typecheck
npm.cmd run test:sites
npm.cmd run module-map:check
cargo test --workspace
```

运行 `npm.cmd run impact` 可根据当前 Git 工作区列出所属模块、受影响模块、强依赖和必跑门禁。
