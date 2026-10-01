# Micro-CT 软件架构

本文是当前软件架构的入口。源码和测试是最终事实源；本文负责解释模块职责、调用方向和安全边界。机器可读的路径归属在 [`module-map/modules.json`](../../module-map/modules.json)，强依赖在 [`module-map/edges.json`](../../module-map/edges.json)。

## 当前生产链路

```text
React UI
  │ EngineCommand / EngineSnapshot
  ▼
Tauri invoke
  │ engine_command / engine_snapshot
  ▼
EngineClient
  │ versioned JSONL over stdio
  ▼
ct-engine ─────────────── authoritative state and safety policy
  ├─ scan coordinator ─── one projection transaction at a time
  └─ devices
      ├─ xray/Moxtek ───── serial binary protocol and measured readback
      ├─ camera/D7100 ──── DigiCamControl and host-file confirmation
      └─ turntable/Nano ── versioned line protocol and motion confirmation
```

数据与控制只向下流动，完整快照只向上返回。React、Tauri、3D 场景和设备驱动都不得维护第二份生产扫描状态。`ct-engine` 是设备连接、预检、安全门控、扫描阶段和提交进度的唯一所有者。

## 模块边界

- `crates/ct-engine/src/devices/xray/`：只处理 X 射线源 I/O、测量回读和 fail-closed 关束；不决定何时允许曝光。
- `crates/ct-engine/src/devices/camera/`：只处理 D7100 发现、曝光参数、拍摄和主机文件确认；不得静默回退到存储卡。
- `crates/ct-engine/src/devices/turntable/`：只处理 Nano 身份、协议、运动确认、警告输出和急停；不推进扫描进度。
- `crates/ct-engine/src/scan.rs`：编排参考帧、操作员门控与原有投影事务；只在设备证据完整后提交图像。
- `crates/ct-engine/src/reconstruction.rs`：读取已封存的扫描数据，离线校正并生成重构体与预览缓存；不调用设备。
- `crates/ct-engine/src/lib.rs`：领域命令、安全门控、完整快照和 JSONL DTO。
- `src-tauri/`：操作系统能力与 sidecar 转发；不拥有设备或扫描状态。
- `src/engine/`：前端契约、生产客户端和浏览器预览；预览不得访问真实硬件。
- `src/scene/`：只读可视化；不发送领域命令。

## 运行流程

1. UI 将操作转换为 `EngineCommand`，不直接调用设备。
2. Tauri 将命令转成带协议版本、请求 ID 和单调序号的 JSONL 请求。
3. `ct-engine` 校验 envelope、模式、当前 phase 和安全条件。
4. 普通命令由引擎同步处理；真实扫描由扫描线程按参考帧阶段和投影事务执行。操作员弹窗的确认由引擎校验，取消使本轮任务安全结束并保留断点。
5. 每次响应返回完整 `EngineSnapshot`，UI 用新快照整体替换视图状态。
6. IPC、设备、超时或未知状态失败时，引擎进入 fail-closed 路径并优先关束。

新扫描在预检后记录实测重构几何；开始后拍十张前暗场，自动开束拍十张前空场并关束，确认装样后执行投影核心。最后一张投影提交后关束并封存清单，不再要求取样或拍摄后参考。重构只使用前暗场和前空场。完成后首次从底部 Reconstruction 区请求重构才开始计算；缓存经清单与文件哈希校验后复用。FDK 在 CPU 和兼容 GPU 中按首帧实测与至少 15% 加速门槛选路，GPU 故障全量回退 CPU；小规模扫描可用受预算限制的 CPU SIRT；CGLS 继续置灰。3D View 仅展示由底部入口打开的结果，不提供重构按钮。

扫描前空场只开束一次，整组连续拍摄并在最后一张文件确认后关束；整组不能在 `maxXraySec` 内安全完成时关束报故障，不逐张循环开束。厂商要求每次关束后至少 2 秒才可再次开束。正常投影跨角度连续出束，仅在出束上限、暂停、停止、故障和最终结束时关束。

### 离线重构预处理与显示

重构从线性 Bayer 蓝通道和暗场差分中稳健拟合完整圆形发光屏边缘，要求足够的角度支持；此路径要求近正视圆屏，不支持未经标定的倾斜屏幕。先按完整圆的外接正方形复制原分辨率 ROI，再统一重采样全部暗场、空场和投影，随后暗空场校正及 FDK/SIRT。方形留 1% 屏缘拟合余量和整数插值护边，上下半圆使用同一边界；原始 NEF 只读。`reconstruction/<method>/` 保存独立的 `detector-crops.f32le`、暗空场均值与方差、`projections.f32le`、`detector-quality.bin` 和 `preprocessing.json`，记录整数方形坐标、屏宽、带余量采样宽度、方向、噪声下限、哈希与逐投影质量计数。实测屏直径定义物理比例，采样 pitch 显式计入 1% 余量，不把较小区域放大为完整屏宽。计算盒覆盖整个屏直径在等中心的物理范围，不再缩至 90%。

噪声下限采用裁剪暗场样本方差并计入暗场均值不确定性，最低 1 ADU；它是删失近似，不是恢复高衰减区的真实值。作 0.7 像素归一化高斯平滑，仅在完整圆最外侧 98%–100% 半径渐隐，不再排除 94% 半径以外的一圈。有正参考增益的低增益像素仍参与重建，质量标记 5 只作警告，不按阈值整片置零；真正无参考增益和饱和像素另行标记。20% 原始无效比例门限保留，按整个圆计数，不按低增益阈值缩小分母。CPU/GPU 保存全角度覆盖诊断，但默认预览直接显示完整有符号科学体的下采样，绝不以覆盖交集隐藏下半体或其他不完整区域。低信号、低增益和部分角度覆盖区域可能有伪影，不作定量准确性保证。

`reconstruction_preprocess.rs` 在源平面用共轭扇束射线估计水平轴修正：固定共同比较射线、分离拟合/验证角度、搜索不超过记录位置的 ±16 探测器像素，要求两组误差均改善超过 10%、最优点在内部且一致。不能满足则保留记录值。`calibration.json` 分开保存采集记录、候选值、实际值、误差与采用原因，不修改 manifest，不估计无证据的垂直偏移或倾角。实测暗空场仍为校正依据，不自动启用亮度漂移拟合或旧数据的背景混合增强。科学体保留有符号数值，默认预览窗下限为零。缓存版本为 5，校验预处理和校准记录哈希，旧的裁切预览缓存不复用。

开始重构时继续展示设备；仅在重构完成且预览数据加载校验成功后自动展示结果。Equipment／Result 是独立显示按钮，不发送重构或设备命令；Start Reconstruction 只负责启动计算。用户切回设备后，普通快照轮询不会抢回结果视图。

## 投影事务

```text
turntable MOVE_ABS + arrival confirmation
  -> XRAY_WARNING asserted
  -> X-ray setpoints + verified beam ON
  -> camera capture + host file confirmation
  -> Nano CAPTURE_DONE
  -> manifest/progress commit
  -> next confirmed MOVE_ABS while beam remains monitored ON
  -> verified beam OFF at time limit, pause, stop, or final exposure
```

最后一张投影提交后，真实扫描保持转台在该张的确认角度；关束、警告输出关闭及最终 manifest 写入成功后报告完成。投影提交时已确认 Nano 的位置和文件；收尾不重复校验整批影像或读取最终 Nano STATUS，也不再发送整圈 `MOVE_ABS` 或 HOME 命令。

每次应用启动后的首次生产扫描必须执行一次 HOME；同一设备会话中的后续扫描可以沿用这次 HOME 建立的参考，但每次预检和启动前均重新读取 Nano STATUS；STOP、故障、掉电、失联或无效参考不能由软件缓存绕过。应用关闭保持 STOP 失效安全动作。中断任务经过预检、必要的 HOME 和文件完整性校验后可以从下一未提交投影继续；文件哈希与参数校验不证明跨 HOME 的物理角度配准，重构精度需另行验证。

每段连续出束受 `maxXraySec` 限制；开始下一投影前和转台到位后检查剩余出束预算，若不足以完成曝光确认，先关束确认、按单调时钟累计 300 秒关束间隔，再继续尚未开始的曝光。已开始的曝光若异常拖延至硬上限仍必须关束并报告故障，不能为了完成图像越过上限。300 秒是本项目的软件策略，不是已查到的 Moxtek 厂商固定冷却规格；厂商《70kV 12W MagPro Operation Manual》要求再次开束前至少关束 2 秒，并限制机壳温度不高于 65°C。剩余时间估算使用已完成投影的有效工作耗时（排除实际冷却等待），另计尚需等待的关束间隔及预计后续间隔。移动期间持续监测出束。暂停只在完整事务边界生效。界面 Stop 请求普通结束并等待工作线程的关束读回；它不锁存软件急停。真实故障、掉线和超时必须尝试关束与停止运动，不能跳过关束确认。再次扫描必须重新预检并在启动前重读当前 Nano 证据；旧预检结果、停止后的参考和软件缓存均不能复用。硬件急停及 Nano STOP 保护继续有效。

## 当前与未来控制拓扑

当前唯一生产实现是 PC 直接控制三类设备。未来 Arduino 中心拓扑的兼容策略见 [dual-control-topology.md](dual-control-topology.md)。未来实现可以替换设备通信拓扑，但不能搬走 `ct-engine` 的安全权威、事务提交规则或对外命令语义。

## 阅读顺序

1. [directory-map.md](directory-map.md)：每个目录及子目录的功能定位。
2. [execution-flows.md](execution-flows.md)：启动、命令、扫描、停止和恢复的执行逻辑。
3. [dual-control-topology.md](dual-control-topology.md)：PC 直控与未来 Arduino 主控的双线边界。
4. [module-graph.md](module-graph.md)：由声明文件生成的模块依赖图。
