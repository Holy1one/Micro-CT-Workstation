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
      ├─ xray/Moxtek ───── single serial owner, readback and autonomous watchdog
      ├─ camera/WPD ────── native PTP/Nikon COM owner, events and host files
      └─ turntable/Nano ── serial owner, negative REL planner and confirmed capture hold
```

数据与控制只向下流动，完整快照只向上返回。React、Tauri、3D 场景和设备驱动都不得维护第二份生产扫描状态。`ct-engine` 是设备连接、预检、安全门控、扫描阶段和提交进度的唯一所有者。

## 模块边界

- `crates/ct-engine/src/devices/xray/`：只处理 X 射线源 I/O、测量回读和 fail-closed 关束；不决定何时允许曝光。
- `crates/ct-engine/src/devices/camera/`：处理原生 WPD/PTP 身份和能力、明确选择、参数读回、D7100 RAM NEF、关联曝光/文件事件、Live View 和支持的对焦；不得静默回退到存储卡。
- `crates/ct-engine/src/devices/turntable/`：只处理 Nano 身份、协议、运动确认、警告输出和急停；不推进扫描进度。
- `crates/ct-engine/src/scan.rs`：编排参考帧、操作员门控与原有投影事务；只在设备证据完整后提交图像。
- `crates/ct-engine/src/reconstruction.rs`：读取已封存的扫描数据，离线校正并生成重构体与预览缓存；不调用设备。
- `crates/ct-engine/src/lib.rs`：领域命令、安全门控、完整快照和 JSONL DTO。
- `crates/ct-engine/src/software_update.rs`：维护占用、更新前设备清理及本会话/操作关联的权威报告；不持有下载、签名或安装逻辑。
- `src-tauri/`：操作系统能力与 sidecar 转发；不拥有设备或扫描状态。
- `src/engine/`：前端契约、生产客户端和浏览器预览；预览不得访问真实硬件。
- `src/scene/`：只读可视化；不发送领域命令。

## 运行流程

1. UI 将操作转换为 `EngineCommand`，不直接调用设备。
2. Tauri 将命令转成带协议版本、请求 ID 和单调序号的 JSONL 请求。
3. `ct-engine` 校验 envelope、模式、当前 phase 和安全条件。
4. 长操作由后台 job 持代理和输入执行，主循环定时处理健康、操作结果、超时和 EOF；唯一引擎应用结果。Stop/OFF 优先，监测不依赖 UI 轮询。扫描由扫描线程按参考帧阶段和投影事务执行。操作员弹窗的确认由引擎校验，取消使本轮任务安全结束并保留断点。
5. 每次响应返回完整 `EngineSnapshot`，UI 按引擎会话和快照序号接收新状态，分别展示受理、取消请求、清理未知与完成。
6. IPC、设备、超时或未知状态失败时，引擎进入 fail-closed 路径并优先关束。

新扫描先由 `prepare_scan` 原子提交任务 ID、投影数、连续出束上限及全部实测几何，再运行预检；路径仅取相机栏的 `Parameters.savePath`，曝光仅从已确认的原生快门派生。取消准备窗口不发设备命令、不提交半套参数、不创建文件或任务锁；预检成功后不再重复填写几何。相机核心配置只由明确 Apply 写入；预检、开始和恢复鲜读比较，不自动停止 Live View、不写快门/RAM/单张数量。开始后拍十张前暗场，自动开束拍十张前空场并关束，确认装样后执行投影核心。末张投影按本次可靠 ExposureEnded 关束，完整文件/哈希与匹配 ticket 释放后提交并封存清单，不再要求取样或拍摄后参考。重构只使用前暗场和前空场。重构 worker 的真实事件携带序号和时间，以稳定 ID、`source=reconstruction` 进入聚合 `consoleLogs`，界面不再生成重复事件。完成后首次从底部 Reconstruction 区请求重构才开始计算；缓存经清单与文件哈希校验后复用。FDK 在 CPU 和兼容 GPU 中按首帧实测与至少 15% 加速门槛选路，GPU 故障全量回退 CPU；小规模扫描可用受预算限制的 CPU SIRT；CGLS 继续置灰。3D View 仅展示由底部入口打开的结果，不提供重构按钮。

扫描前空场只开束一次，整组连续拍摄并在最后一张文件确认后关束；整组不能在 `maxXraySec` 内安全完成时关束报故障，不逐张循环开束。厂商要求每次关束后至少 2 秒才可再次开束。正常投影跨角度连续出束，仅在出束上限、暂停、停止、故障和最终结束时关束。

### 离线重构预处理与显示

重构从线性 Bayer 蓝通道和暗场差分中稳健拟合完整圆形发光屏边缘，要求足够的角度支持；此路径要求近正视圆屏，不支持未经标定的倾斜屏幕。先按完整圆的外接正方形复制原分辨率 ROI，再统一重采样全部暗场、空场和投影，随后暗空场校正及 FDK/SIRT。方形留 1% 屏缘拟合余量和整数插值护边，上下半圆使用同一边界；原始 NEF 只读。`reconstruction/<method>/` 保存独立的 `detector-crops.f32le`、暗空场均值与方差、`projections.f32le`、`detector-quality.bin` 和 `preprocessing.json`，记录整数方形坐标、屏宽、带余量采样宽度、方向、噪声下限、哈希与逐投影质量计数。实测屏直径定义物理比例，采样 pitch 显式计入 1% 余量，不把较小区域放大为完整屏宽。计算盒覆盖整个屏直径在等中心的物理范围，不再缩至 90%。

噪声下限采用裁剪暗场样本方差并计入暗场均值不确定性，最低 1 ADU；它是删失近似，不是恢复高衰减区的真实值。作 0.7 像素归一化高斯平滑，仅在完整圆最外侧 98%–100% 半径渐隐，不再排除 94% 半径以外的一圈。有正参考增益的低增益像素仍参与重建，质量标记 5 只作警告，不按阈值整片置零；真正无参考增益和饱和像素另行标记。20% 原始无效比例门限保留，按整个圆计数，不按低增益阈值缩小分母。CPU/GPU 保存全角度覆盖诊断，但默认预览直接显示完整有符号科学体的下采样，绝不以覆盖交集隐藏下半体或其他不完整区域。低信号、低增益和部分角度覆盖区域可能有伪影，不作定量准确性保证。

`reconstruction_preprocess.rs` 在源平面用共轭扇束射线估计水平轴修正：固定共同比较射线、分离拟合/验证角度、搜索不超过记录位置的 ±16 探测器像素，要求两组误差均改善超过 10%、最优点在内部且一致。不能满足则保留记录值。`calibration.json` 分开保存采集记录、候选值、实际值、误差与采用原因，不修改 manifest，不估计无证据的垂直偏移或倾角。实测暗空场仍为校正依据，不自动启用亮度漂移拟合或旧数据的背景混合增强。科学体保留有符号数值，默认预览窗下限为零。缓存版本为 5，校验预处理和校准记录哈希，旧的裁切预览缓存不复用。

开始重构时继续展示设备；仅在重构完成且预览数据加载校验成功后自动展示结果。Equipment／Result 是独立显示按钮，不发送重构或设备命令；Start Reconstruction 只负责启动计算。用户切回设备后，普通快照轮询不会抢回结果视图。

## 投影事务

```text
turntable prescribed negative MOVE_REL + unwrapped arrival/phase confirmation
  -> XRAY_WARNING asserted
  -> X-ray setpoints + verified beam ON
  -> camera capture -> correlated ExposureEnded -> complete host file + SHA-256
  -> Nano CAPTURE_DONE
  -> manifest/progress commit
  -> next confirmed MOVE_REL while beam remains monitored ON
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

## 原生离线候选与自主设备

当前源码通过原生 Windows WPD/PTP 控制相机；D7100 RAM NEF 是当前完整采集事务和正式扫描首期适配对象；已实现的摄影参数与操作子集不等于覆盖相机全部厂商功能。其他型号只开放有证据的标准属性。COM 对象由相机线程持有，Moxtek 与 Nano 各有单一串口所有者。Live View JPEG走当前用户 ACL、随机端点、会话/序号/codec/长度验证的引擎 named pipe（单帧上限 4 MiB，JPEG 宽高上限 4096×3072；Tauri 侧连接、读取、校验与 ACK 共用同一 250 ms 期限，前端不得按更大尺寸或更长超时设计），再由 Tauri转发；NEF经原生完整下载、文件/hash确认后由引擎登记，照片预览提取内嵌 JPEG。JSONL v2仅承载控制、状态、登记及内部端点握手，不承载实时图像。

Live View全链使用内存：原生最新帧槽、named pipe和前端有界JPEG data URL均不保存实时图像文件。端点轮换的短暂缺失/busy只在原250ms期限内重试；暂无新帧返回null。UI保留最近已解码画面并标记STALE，收到当前会话的新帧后恢复LIVE；停止、隔离、断开或会话变更清除画面，迟到结果不得返回。data URL避免浏览器Blob存储的潜在磁盘溢写。

当前源码为本轮 rc3 候选：已在实机 host 上完成真实 Tauri IPC、Nano 身份/状态/停止/维护门禁与相机会话验证，并记录了本轮确认的实机缺陷 F-01–F-07（逐条状态与证据见[后端重验证记录](../backend-revalidation-2026-10-06.md)）；F-01 修复已取得实机证据；当前候选的独立 NEF T7 已 PASS（§5z：1/5/15/30 s、可靠曝光结束/文件/hash、ID 预览、恢复与正常退出）。F-12 原 WPD 失败本轮未复现，根因未确证；HOME/MOVE、射线、完整扫描与软件更新维护锁仍未完成。已签rc2包及[rc2验收](../workstation-restructure-rc2-acceptance-2026-10-05.md)保持独立。每次Live View从inactive启动生成新的32位hex nonce，和稳定的camera连接/capture ID会话不同；帧pipe/metadata及`liveViewControls.sessionId`必须一致。Stop、隔离、会话退休撤销帧与控制，旧会话请求和迟到结果不得作用于下一轮LV。

摄影目录仍34项；LV另有`CameraHealth.liveViewControls={sessionId,properties}`，仅`cameraZoom`（D1A3）和`focusMode`（D061）。显式`camera_live_view_read {sessionId}`及`camera_live_view_configure {sessionId,controls}`要求当前LV会话、实际广告、严格UINT8 descriptor和鲜读/读回；唯两键，不提供任意setter。普通摄影Apply仍拒LV，LV控制不入扫描profile或摄影预设。D0E1 lensSort、D1A6照片/电影选择器、D1F2 activeSlot保留原生只读诊断（未广告UNKNOWN，nativeWritable保留而不开放写）。LV驱动还需实际操作支持、fresh lensSort=1、selector=0及D061资格：模式0允许AF，0/4允许有界step；AF-F2禁止有界motor，物理MF3禁止写与motor。lensSort=1本身不证明驱动能力。

相机属性快照分别提供本机supported、原生nativeWritable、当前effective/writable、disabledReason及每个options条目的supported/enabled/reason。属性描述和当前确认读回是依据；前端只消费这些结果，枚举用下拉菜单，无效或不支持的属性/选项变灰。模式、图像格式、白平衡、测光等父属性Apply后重新读取描述与实际值，再判定子属性。自动ISO开启时手动ISO仍控制基准值，NEF+JPEG尺寸仍控制JPEG部分，HDR在NEF下只允许OFF。功能对照及证据见[native-camera-capability-matrix.md](native-camera-capability-matrix.md)。

相机与转台独立连接、自检；两者使用常驻栏目，相机高级设置与基本栏共用会话草稿。扫描保存至根目录/任务 ID；正常独立单拍仅保存至同一根目录/`CameraCaptures`，不增加扫描帧数或 manifest。保留目录名、已有清单、目录别名与原子不覆盖检查；诊断 `camera_test_capture` 才可传显式隔离目录。扫描要求固定 M、Auto ISO OFF、MF、固定 WB，以及包围/ADL/HDR/多重曝光的严格关闭或等价证明；独立单拍允许已确认的 P/A/S、自动 ISO/WB 和支持的 AF 模式，但不隐式触发 AF，仍要求单个主机 RAM NEF输出、关联C102完成、完整文件及 hash；多重曝光UNKNOWN明确诊断，一个host文件不证明一次物理曝光。Live View 开启时拒绝拍摄。完整登记后 `deviceControls.camera.lastCapturedImage` 提供 capture ID、会话、操作、大小与哈希；Tauri 的照片预览只接收 capture ID，由引擎 descriptor 解析并复核大小/hash，不接受 renderer 任意路径。HOME 内部保留固件有界反向退让；台架物理隔离确认仍显示射线 UNKNOWN，禁止开束。

持久任务登记锁跨重启和 taskId 切换保留，允许连接修复和只读状态；参数、对焦与普通运动仍锁定。同一任务的受控恢复预检需明确确认本次 USB 自动关断释放，按 schema 3 冻结 V/I、Exposure、maxXraySec 和实际相机 profile 重建会话，随后受控 HOME 和文件哈希校验。缺失/坏登记 fail closed；可识别任务仅在完整清理后显式放弃，保留文件与原目录。schema 2 继续只读重构及版本 5 合法缓存校验，新后端拒绝续扫。

schema 3 清单的新采集 profile 使用内部版本 5，冻结 RAM 记录介质、单个主机输出数量、快门属性码、固定 WB及固定自动化证明。包围要求本次广告的 `0xD0C0`、UINT8 descriptor和鲜读 OFF=0；D7100 ADL物理适用，缺少广告或读取失败仍是 UNKNOWN，必须鲜读 OFF=4。HDR已广告时必须读回 OFF=0；仅未广告时允许已确认 NEF=4的官方互斥证明，不能覆盖已广告的 ON、错误或矛盾。多重曝光物理适用但目前无可靠 PTP OFF编码，`multipleExposureEvidence`始终明确 UNKNOWN/不可观测，绝不改写为机器 OFF；正式扫描只额外接受 `camera_confirm_multiple_exposure_off` 建立的操作员确认。该确认是独立证据：绑定已识别 `deviceId`、本次 32 hex 相机 `sessionId`、由全部 confirmed/readable/无错误读回计算的证据指纹、`policyVersion=1`、确认时间与操作者，并另以 `multipleExposureOperator` 暴露 state/attested/valid；失联、重连、重启、身份或任一确认参数变化即失效，机器明确 ON、读回失败或未确认一律拒绝，确认命令不清除 fault/cleanup/task/maintenance 锁，frozen profile 只记录其策略版本因此同参数重新确认仍可受控恢复。规范 hash覆盖固定策略版本2及这些证明。曝光延迟 OFF为3。标准 `0x500D`使用0.1 ms单位，Nikon `0xD100`使用原生分数。旧 profile 1–4保留各自编码、原 hash、文件与只读重构，拒绝续扫，不自动改参数或重写旧清单。当前版本、设备身份和实际冻结证明均匹配才可受控恢复。

D7100 的 `0xD161` 对焦模式 3 为硬件 MF、4 为软件 MF，两者均符合确定性采集；0/1/2 为自动对焦，扫描继续拒绝。冻结 profile 保留实际模式值，不把硬件 MF 改写为软件 MF。

## 更新维护与验证边界

`prepare_software_update` 先拒绝扫描、暂停、收尾、拍摄、运动、Live View、重构、busy及持久任务锁，再建立独立维护占用。引擎执行 STOP/OFF 与相机清理，返回 `softwareUpdatePreparation` 的本次 `sessionId`、`operationId`、`status`、`ready`、`maintenanceLocked`、各设备 confirmed/never-owned/unknown、`noActiveTransactions` 和错误。UNKNOWN或清理未确认不会授权更新；全未建立设备会话可走明确 never-owned 分支，设备显示仍保持 UNKNOWN。普通 Stop 不清除维护占用，并使旧 ready 报告失效；匹配操作的取消只在清理已确认时释放占用，不自动放弃任务、重连设备或恢复预览/出束。

Tauri 更新器只消费匹配的领域报告，再发送 EOF并等待 owned engine 正常 exit0；helper 还须等待原应用正常退出。进程 exit0、签名正确或文件下载完成均不能代替设备安全报告。安装结果也独立于旧应用的安全退出。发行链、安装身份及双签名说明见 [Windows更新与分发](../desktop-update-distribution-plan.md)。

**P0 分项实机 PASS，完整射线联动仍未认证。** 本机过程证据 [0070 result](../../tmp/tests/desktop/20261005-004700-0070-nef/result.json)、[NEF与关联事件](../../tmp/tests/desktop/20261005-004700-0070-nef/captured-nef.json) 记录 D7100 新独立 RAM NEF：C101/C102、本次曝光结束、23,476,423 字节完整文件与 hash、原参数恢复、30.1096 秒 UI Live View（首帧后 missing=0、实时帧文件=0），以及 App/engine严格正常 exit0。该证据限定于 EXE `0070808c9fb206d4745ba039bcd1eb4c13d19e0a12fef3d164be93797637862c`、内嵌 engine `f8f074d75518e4cf0304e630c683ba643e8f5f23be882b00687ee03e3b191ffa`；明确没有射线或 Nano 运动，未保留关闭窗口前独立 LV-stop 终态快照，关闭证据来自 strict EOF清理。该 P0 分项是历史记录，不认证本轮 rc3 host；当前候选独立 NEF T7 已 PASS（§5z：机身已单张、可靠关联事件、完整文件/hash、ID 内存预览、恢复与正常退出）；F-12 原失败未复现，未认定根因或已修复，状态见[后端重验证记录](../backend-revalidation-2026-10-06.md)。上述 tmp 过程资料本机保存且不进入 Git。

原rc1/profile4的固定自动化缺口已由[审计FAIL记录](../../tmp/tests/camera/20261005-fixed-automation-audit/result.json)确认；旧P0的单个RAM文件和C102不能证明无包围/无多重曝光，也不豁免扫描门禁。当前profile5及严格证明经[领域离线回归](../../tmp/tests/ct-engine/20261005-profile5-final/result.json)（247库+2进程测试）和[浏览器合同回归](../../tmp/tests/camera/20261005-fixed-automation-audit/preview-final-result.json)（38项）验证，均为**仅离线验证**，真实扫描因多重曝光不可观测仍BLOCKED。用户关于人工验收策略的问题仍待答，当前无人工覆盖；原rc1验收记录保留为历史，不据此认证rc2。已有独立Nano、历史flash应用区及桌面分项证据见[2026-10-04验证记录](../other-device-verification-2026-10-04.md)。负载精度、bootloader/fuse、当前新策略实机和完整射线联动仍需各自证据。候选构建与发布保留既有正式入口及其可核验来源，原始NEF和未完成任务数据保留。`MICRO_CT_ENGINE_DATA_DIR`仅隔离离线测试/fixture；默认持久目录为`LOCALAPPDATA/MicroCTWorkstation`。
