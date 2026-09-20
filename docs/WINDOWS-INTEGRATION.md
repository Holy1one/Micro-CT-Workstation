# Windows 集成边界

Windows 11 x64 是真实硬件、GPU 与发布验收平台。Mac 构建只证明跨平台 UI、协议和纯逻辑。

## 当前传输和进程

- `ct-workstation.exe`：Tauri 2 + WebView2。
- `ct-engine.exe`：Rust，设备所有权与采集状态机。
- DigiCamControl：由 Rust 相机模块以受控外部命令调用，并在主机侧确认输出文件。
- 算法进程：仍为未来扩展，只能从独立插件 manifest 启动，不进入安全控制环。
- 本机进程通信：第一版沿用 sidecar stdio + 当前版本化 envelope；需要独立服务生命周期时再替换为 Named Pipe，不开放 localhost 服务端口。

## 设备模块落点

生产设备代码按物理角色隔离，由 `ct-engine` 调用统一生命周期
`discover/connect/health/command/stop/disconnect`：

- `crates/ct-engine/src/devices/turntable/nano.rs`：串口、身份/能力、心跳、运动确认、警告和 STOP。
- `crates/ct-engine/src/devices/camera/digicam_control.rs`：D7100 发现、曝光、拍摄与主机文件确认。
- `crates/ct-engine/src/devices/xray/moxtek.rs`：Moxtek 命令、状态回读和 fail-closed 关束。

设备模块不持有任务状态，也不自行安排下一投影。`ct-engine/src/scan.rs` 是跨设备事务编排入口，
`ct-engine/src/lib.rs` 是命令与安全裁决入口。UI 只向 engine 发领域命令。FBP、用户重建和材料分辨继续落在独立 worker/manifest，不加入设备模块，也不动态加载到 engine。

独立 digiCamControl bridge 的备选 v1 JSONL 契约见 `bridges/digicam/README.md`；当前 Rust 直接调用方式与未来 bridge 方式不得同时拥有相机状态。

PC 直控与未来 Arduino 主控的稳定边界见 `docs/architecture/dual-control-topology.md`。

## 必须在 Windows 完成的验收

1. WebView2 下的 UI 与后续 vtk.js GPU 性能。
2. Nano 串口枚举、断线、心跳、STOP、`REARM + HOME` 恢复。
3. Nikon 主机存储、传输校验与远端删除。
4. Moxtek 状态读回、硬件联锁、急停与故障闭锁。
5. digiCamControl/.NET bridge 进程退出、超时与相机所有权。
6. Python/CUDA 插件隔离、显存压力与取消语义。
7. WebView2 离线部署、代码签名和 MSI/安装器。

任何真实设备 adapter 都必须显式启用并通过独立验收；不得从开发预览自动切换。
