# 架构选型与边界

## 结论

采用“桌面壳 + Rust 模块化单体内核 + 平台设备桥 + Python 算法进程”的混合架构。当前需求不适合微服务，也不适合把设备控制、CT 计算和 UI 全塞进 Tauri 进程。

| 层 | 技术与职责 |
|---|---|
| 表现层 | Tauri 2 + React/TypeScript；固定工业工作台，未来在中央场景加入 vtk.js 2.5D/3D |
| 领域内核 | Rust `ct-engine`；唯一任务状态、设备所有权、安全门控、采集编排、产物提交 |
| 设备适配 | Rust adapter 为主；受 Windows SDK 限制的相机功能通过独立 .NET bridge 接入 |
| 算法运行 | 独立 Python worker；manifest 声明能力、输入输出和环境，不把 Python 动态加载进 engine |
| 数据层 | 不可变采集/科学产物 + manifest；SQLite 只保存索引、任务与恢复点，不保存大体数据 |

这是“可部署的模块化单体”，不是分布式系统。只有 ABI、运行时或故障隔离确有必要的部分才拆成进程。

## V1 运行链路

```text
React / TypeScript
  -> Tauri invoke
  -> Tauri EngineClient
  -> versioned JSONL stdio
  -> ct-engine
```

`ct-engine` 独占设备状态、预检、回零、扫描状态与停止语义。Tauri 不保存第二份领域状态，只校验传输层 envelope 并转发快照。浏览器模式的 `DevPreviewAdapter` 只服务 UI 开发，始终显示开发预览身份，且不访问任何硬件。

## IPC envelope

每条请求与响应都包含：

- `protocol_version`
- `request_id`
- `command`
- `payload`
- `timestamp`
- `sequence`
- `error_code`

V1 在 macOS 使用子进程 JSONL stdio。Windows 第一版也优先保留 stdio sidecar，减少无收益的传输重写；只有需要 engine 独立重启、服务化或多客户端时再切换 Named Pipe。envelope 和领域命令与传输实现解耦。

## 安全边界

- 生产 engine 默认 `production_locked`，不会自动退回模拟模式。
- `stop` 与 `disconnect` 会使预检和回零失效。
- 继续采集前必须重新预检并回零。
- V1 不提供 X-ray enable 命令。
- 软件 fail-closed 不能替代门锁、急停、安全继电器和硬件使能链。

## 后续扩展

```text
ct-workstation
  -> ct-engine
       -> MotionActor
       -> XRayActor
       -> CameraActor -> digicam-bridge (.NET, Windows)
       -> AcquisitionCoordinator
       -> SafetyCoordinator
       -> ArtifactStore
       -> AlgorithmSupervisor -> plugin venv / python -m ct_worker
```

重构结果区分不可变 Scientific Artifact 与用于 vtk.js 的降采样 Render Artifact。用户算法通过版本化 manifest 与独立环境加入，不动态加载进 engine 进程。

## 算法规则

- 默认三维重建：内置 FBP worker；没有用户插件时自动选择。
- 用户三维重建：上传符合 manifest/schema 的独立插件包后，由 AlgorithmSupervisor 启动隔离进程。
- 材料分辨：没有默认算法；未提供指定格式插件时功能保持不可用。
- 算法进程只读输入产物，写入新的临时产物；engine 校验完成后原子提交，失败或取消不污染已有结果。
- UI 只消费状态、缩略图与 Render Artifact；原始投影和完整体数据不经前端 IPC 传输。

## 扩展约束

- `ct-engine` 保持唯一真相源，UI、bridge 和 worker 不自行推进扫描状态。
- 每个设备 adapter 实现相同生命周期：discover/connect/health/command/stop/disconnect。
- 新模块通过 capability 注册，不通过 UI 直接调用 SDK。
- 真实 X-ray 必须有独立硬件联锁和验收；软件状态永远不能替代急停链。
