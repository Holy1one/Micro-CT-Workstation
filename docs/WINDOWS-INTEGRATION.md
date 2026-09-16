# Windows 集成预留

Windows 11 x64 是真实硬件、GPU 与发布验收平台。Mac 构建只证明跨平台 UI、协议和纯逻辑。

## 计划中的传输和进程

- `ct-workstation.exe`：Tauri 2 + WebView2。
- `ct-engine.exe`：Rust，设备所有权与采集状态机。
- `digicam-bridge.exe`：.NET，封装 digiCamControl 或厂商 SDK。
- 算法进程：从插件 manifest 指向的 venv 启动 `python.exe -m ct_worker run request.json`。
- 本机进程通信：第一版沿用 sidecar stdio + 当前版本化 envelope；需要独立服务生命周期时再替换为 Named Pipe，不开放 localhost 服务端口。

## Adapter 落点

后续设备代码进入 `crates/ct-engine/src/adapters/`，由 engine actor 调用统一生命周期
`discover/connect/health/command/stop/disconnect`：

- `nano.rs`：串口、心跳、运动与本地 STOP 的 Rust adapter；协议以固件已发布契约为准。
- `nikon.rs`：相机领域 adapter 和 .NET bridge 客户端；SDK 兼容代码只进入 `bridges/digicam/`。
- `moxtek.rs`：状态读回与命令 adapter；没有厂商协议和硬件联锁验收前只保留不可启用契约。

adapter 不持有任务状态，也不自行安排下一投影。`ct-engine` 的 `MotionActor`、`CameraActor`、
`XRayActor`、`AcquisitionCoordinator` 和 `SafetyCoordinator` 是唯一编排入口。UI 只向 engine 发领域命令。
FBP、用户重建和材料分辨继续落在独立 Python worker/manifest，不加入这些 adapter，也不动态加载到 engine。

digiCamControl bridge 的 v1 JSONL 契约见 `bridges/digicam/README.md`。

## 必须在 Windows 完成的验收

1. WebView2 下的 UI 与后续 vtk.js GPU 性能。
2. Nano 串口枚举、断线、心跳、STOP、`REARM + HOME` 恢复。
3. Nikon 主机存储、传输校验与远端删除。
4. Moxtek 状态读回、硬件联锁、急停与故障闭锁。
5. digiCamControl/.NET bridge 进程退出、超时与相机所有权。
6. Python/CUDA 插件隔离、显存压力与取消语义。
7. WebView2 离线部署、代码签名和 MSI/安装器。

任何真实设备 adapter 都必须显式启用并通过独立验收；不得从开发预览自动切换。
