# Windows 集成边界

Windows 11 x64 是真实硬件、GPU 与发布验收平台。其他平台构建只证明所运行的
跨平台 UI、协议和纯逻辑检查。真实设备操作仍需本次目标的明确授权。

## 当前传输和进程

- `ct-workstation.exe`：Tauri 2 + WebView2；正式启动副本为
  `portable-release/micro-ct-workstation-portable.exe`。
- `ct-engine.exe`：Rust，拥有设备、安全门控、采集状态机与离线重构。
- 当前相机模块 `digicam_control.rs` 由 Rust adapter 启动外部
  DigiCamControl `CameraControlRemoteCmd.exe`，并在主机侧确认文件。adapter 位于
  引擎内不代表相机通信原生运行于引擎内。
- 重构方法编译进 `ct-engine`，通过 `ReconstructionMethod` 声明；FDK 使用 CPU 或
  兼容的 wgpu GPU 路径，受限规模 CPU SIRT 可用。未采用独立算法插件运行时。
- 控制与快照沿用 sidecar stdio 和版本化 JSONL envelope，不开放 localhost
  设备控制服务。前端生产路径只能通过 Tauri 进入引擎。

[ADR 0003](decisions/0003-native-wpd-camera-and-device-control.md) 已接受原生 Rust
WPD/PTP 相机与设备自主控制，状态为待实现；当前说明不得据此声称原生已上线。
迁移后不再调用 DigiCamControl，不自动回退旧工具或模拟。禁止额外相机桥进程和
算法插件的 [ADR 0002](decisions/0002-no-out-of-process-camera-bridge-or-algorithm-plugins.md)
继续有效。

## 设备模块落点与领域入口

生产设备代码按物理角色隔离，由 `ct-engine` 管理身份、生命周期和能力：

- `crates/ct-engine/src/devices/turntable/nano.rs`：串口、身份/能力、心跳、运动确认、
  警告和 STOP。
- `crates/ct-engine/src/devices/camera/digicam_control.rs`：当前 D7100 发现、精确曝光
  档位、外部捕获与主机文件确认。原生迁移仍在已有 camera 模块内拆分公共接口、
  WPD/PTP、Nikon、capture 和 Live View；不并入厂商完整外部框架。
- `crates/ct-engine/src/devices/xray/moxtek.rs`：Moxtek 命令、实测回读、两秒重开
  门控和 fail-closed 关束。

设备模块不持有任务状态，不互相调用，不自行安排下一投影。`scan.rs` 持有扫描
跨设备顺序；领域入口持有自主 HOME 等安全裁决。重构读取已封存、哈希验证的数据，
不调用设备。PC 直控与未来 Arduino 主控的稳定边界见
[dual-control-topology.md](architecture/dual-control-topology.md)。

## 已接受的原生目标契约

- 相机和转台独立连接、自检；多相机明确选中一个活动身份。完整厂商适配和正式
  扫描首期仅 D7100 NEF，其他型号仅开放有证据的标准能力，不能证明 host-only
  的捕获禁用，不向未知设备试发 Nikon 操作。
- 专用相机 COM 线程、单一 Moxtek worker 与现有 Nano 串口 worker 保持各自唯一
  通信所有权。长操作异步执行，监测不依赖 UI；Stop/OFF/EOF 保持优先，不等待
  相机卡住或运动完成，受理与硬件确认分别表达。
- 拍摄的可靠曝光结束与完整文件/哈希确认分开。正常末张投影及暂停按本次可靠
  `ExposureEnded` 关束，提交仍需完整文件和匹配 `CAPTURE_DONE`；空场末张文件
  确认后正常关束。Stop、故障和连续出束硬上限立即关束优先。
- Live View 图像经引擎拥有、当前用户 ACL 和随机会话端点的本地 named pipe
  传至 Tauri，再受控转发；JSONL 仅传状态。限制帧长、版本、session/seq 与 codec，
  丢弃旧帧，慢消费者不阻塞 Stop。Live View 开启时拒绝捕获，不自动停预览拍摄。
- 单拍与扫描目录/帧数/断点隔离，不自动开束。HOME 先确认关束；仅转台台架模式
  需物理隔离确认，射线仍 UNKNOWN 并禁开束。普通运动固定生产方向，HOME 内部
  有界反向退让保留。任务占用和未完成断点手动锁由引擎执行。
- schema 3 冻结实际采集 profile；schema 2 保持原文件可读和离线重构，新后端拒绝
  续扫。保留数值重构、缓存版本/hash 规则和只读原始 NEF。

## Windows 验收与交付门槛

离线候选必须完成适用的 Rust/TypeScript/IPC/布局门禁、固件候选源码重建与 host
harness、同生产协调器的 transport/fake 故障测试，以及 Windows 候选 EXE 构建、
启动和视觉检查。交付 `release/native-device-controls-offline/`，包含构建来源、哈希、
第三方许可、离线报告与 `BLOCKED` 实机项；保留现用正式 EXE 及来源/哈希。

以下内容须在另行明确授权后用真实设备验收，离线通过不能替代：

1. Nano 当前 flash 与候选逐字节一致性、真实方向、负载角度/回零精度、心跳、
   STOP 和 `REARM + HOME` 恢复；VERSION/BUILD 字符串不证明 flash 一致。
2. D7100 主机 RAM NEF、属性实际读回、Live View、拍摄事件关联、完整文件与取消
   清理语义，以及 WPD 故障/卡住时的隔离行为。
3. Moxtek 实测读回、联锁、急停、两秒重开、持续监测、OFF 写出/读回与故障闭锁。
4. 联动扫描、连续两轮扫描、末张保持角度、暂停/停止清理及采集 profile 恢复。
5. 目标 Windows/WebView2/GPU 环境的视觉、性能与部署；签名/安装器按发布范围验收。

任何真实 adapter 都必须显式启用并经独立验收；不得从开发预览自动切换。没有真实
硬件证据时报告“仅离线验证”，不标记 backend 为生产认证完成。
