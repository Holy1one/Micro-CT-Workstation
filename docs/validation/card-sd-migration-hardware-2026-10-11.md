# 存卡迁移实机台架记录（2026-10-11，仅转台与相机）

本记录只覆盖用户本次明确授权的实机动作：转台连接与身份读取、D7100 发现与显式选择、
控制器固件 2.2.0 烧录与读回校验、以及烧录后经生产引擎复核卡策略门禁。**未出束、未运动、
未拍摄**；射线源本次未连接、未上电，因此 P5/P6 与存卡扫描端到端保持 BLOCKED。
`hardwareCertified` 保持 `false`。

## 1 设备与端口核对

| 项目 | 实测 |
| --- | --- |
| 转台控制器 | `USB-SERIAL CH340 (COM7)`，`USB\VID_1A86&PID_7523\7&3A944CE5&0&2` |
| 相机 | `D7100`，`USB\VID_04B0&PID_0430\8322769` |
| 引擎 | 生产模式 `ct-engine.exe`（无 `--preview`），SHA-256 记于各运行目录 `engine.sha256` |
| 驱动方式 | `.cache/tools/drive-engine.ps1`：JSONL v2 信封，逐条请求/逐条响应，证据可回放 |

JSONL v2 信封字段为 snake_case（`engine_session_id`/`protocol_version`/`request_id`/`error_code`），
且除 `snapshot` 外每个请求的 `payload` 必须自带 `"type": "<command>"`，否则引擎在任何设备动作前
返回 `COMMAND_PAYLOAD_MISMATCH`。证据：`.cache/tests/hardware/20261011-p2-readonly-{a,b}`。

## 2 烧录前基线（只读）

- 转台身份：`RTS9060_NANO` v2.1.1 / build 20261009 / 协议 2，能力仅
  `HEARTBEAT_ACK, XRAY_WARNING, RESTORE_REFERENCE` —— **未公布 `MOVE_SETTLE_150MS`**；
  状态 `LOCKED`、`homed=false`、`referenceValid=false`、`positionPulses=-95733`、
  `pulsesPerRev=96000`、`microsteps=8`。证据：`.cache/tests/hardware/20261011-p2-readonly-c`。
- 相机发现：1 台，`D7100`，后端 `windows_wpd_unverified`，设备 ID
  `\\?\usb#vid_04b0&pid_0430#8322769#{6ac27878-a6fa-4155-ba85-f98f491d4f33}`。
  证据：`.cache/tests/hardware/20261011-p2-readonly-d`。
- 显式选择后身份与能力：后端 `windows_wpd_nikon_d7100`、厂商 `Nikon Corporation`、
  序列号 `83227690000000000000000000000000`；协商能力 `hostCapture`/`scanNef`/`liveView`/
  `properties`/`cardCapture`/`cardOverflow` = true，`autofocus`/`focusStep` = false；
  实测属性读回可用（`recordingMedia` 确认值 0 = 机身存储卡，电量 100%，`exposureMinMs` 0.125 /
  `exposureMaxMs` 30000）；会话干净关闭（引擎报告 `camera cleanup=true`、`STOP confirmed=true`，
  退出码 0）。证据：`.cache/tests/hardware/20261011-p2-readonly-e`。
- 由此得到的关键结论：**已刷固件 2.1.1 不满足 `card_capture_policy` 的固定候选**（2.2.0 /
  20261010 + `MOVE_SETTLE_150MS`），在该状态下任何卡目标预检都会被 `CARD_FIRMWARE_UNVERIFIED`
  拒绝——这是 fail-closed 的正确行为，但卡路径无法在旧镜像上被演练。

## 3 实机发现并修复的问题：已发布就绪度与目的不一致

`cameraScanReadiness` 原先无条件按主机 RAM 目的计算。真实机身在确认存卡（`recordingMedia=0`）时，
界面因此报告"未就绪 · 要求 Host RAM (1)"并提示操作员把机身切回 RAM，而预检实际冻结的是
schema-7 卡 profile；控制器固件门禁也只在预检被拒后才可见。

修复（`399aa55`）：就绪度改为跟随**相机自己确认的目的**，卡目的时公布 `cardSettleFirmware`
门禁，并与预检共用同一份控制器准入函数（`card_capture_policy_for`），两者不会再分歧；
主机 RAM 目的的报告保持不变。回归测试
`device_controls::tests::published_readiness_describes_the_destination_the_camera_confirmed`。

## 4 固件烧录（本次授权动作）

| 步骤 | 命令与结果 |
| --- | --- |
| 构建候选 | `npm.cmd run firmware:build` → PASS `flash=14196 / 30720`、`RAM=1476 / 2048`；源身份 34818 字节 / SHA-256 `2843f57e…`（与 `build-config.json` 固定候选一致）；产物 `.hex` 39949 字节 / SHA-256 `542c9f30…`，`.bin` 14196 字节 / SHA-256 `79e54f1b…` |
| 读回旧镜像（回滚备份） | `avrdude -p atmega328p -c arduino -P COM7 -b 57600 -U flash:r:…:i` → 32768 字节，非 0xFF 数据 15839 字节，SHA-256 `d283c9de…`（存于 `.cache/tests/hardware/20261011-firmware-flash/before-flash.hex`，未纳入版本控制） |
| 写入并校验 | `-U flash:w:…rts9060_nano.hex:i -U flash:v:…:i` → `Writing 14196 bytes`、`14196 bytes of flash verified`（两次校验均通过），退出码 0 |
| 生产引擎复核 | 重新连接后 `turntable_identity`：`RTS9060_NANO` **v2.2.0 / build 20261010**，能力含 **`MOVE_SETTLE_150MS`**；相机确认存卡时 `cameraScanReadiness = {issues: [], ready: true}` |

最后一行是本迁移目前最强的实机证据：控制器固件门禁与卡目的就绪度**经生产代码路径**同时通过
（`NanoIdentity::advertises_move_settle_150ms()` + `card_capture_policy` + 目的感知就绪度）。
证据：`.cache/tests/hardware/20261011-firmware-flash/`。

## 5 状态

| 项 | 状态 | 说明 |
| --- | --- | --- |
| 转台连接/身份/状态读回 | **PASS（实机）** | COM7、2.2.0 身份与能力读回 |
| D7100 发现与显式选择、能力协商、属性读回 | **PASS（实机）** | 含 `cardCapture`/`cardOverflow` 协商 |
| 固件候选构建、烧录与读回校验 | **PASS（实机）** | 14196 字节写入并两次校验通过，回滚镜像已留 |
| 卡策略固件门禁 + 目的感知就绪度 | **PASS（实机）** | 经生产引擎复核 |
| 150 ms 末脉冲下降沿实测间隔 | **部分 PASS（实机）** | HOME 后 1°/1°/2°/5° 相对运动全部完成并到达 CAPTURE_HOLD→IDLE；实测脉冲速率 ≈2.01 ms/脉冲（≈497 脉冲/秒），零脉冲截距 ≈1319 ms（协议与稳定等待的上界）。**150 ms 本身仍无法与协议开销分离**，见第 8 节 |
| 单卡/Overflow 触发、结束与续存、Stop/心跳、图像质量 | **BLOCKED** | 需授权拍摄与运动 |
| 主机 RAM 单张真实拍摄 | **FAIL（实机）** | 机身完成曝光（`PropertyChanged 0xD1B4`）但未在截止时间内给出曝光结束/完成事件，无 NEF；引擎按 fail-closed 隔离会话并退出码 1。见第 7 节 |
| 存卡扫描端到端、1000 秒截止、温度/OFF/联锁、采集至重构全链、RAM/SD 对照提速 | **BLOCKED** | 射线源本次未连接、未上电 |

**未验证即不得声称**：真实 `InitiateCapture`/`ObjectAdded`/`CaptureComplete`/`StoreFull` 语义、
真实卡对象内容与 WPD 传输边界、150 ms 实测间隔、整机联动与提速。`hardwareCertified` 保持
`false`；本记录仅为**授权台架检查**，不构成硬件认证。

**回滚**：如需回到烧录前镜像，用 `before-flash.hex` 以同一 avrdude 参数执行
`-U flash:w:<backup>:i`；该文件是本机唯一证据，删除前须确认已被长期保留。

## 6 转台运动与 150 ms 时序实测（已授权，被引擎拒绝）

授权后以 `turntable_relative` 请求 1°/1°/2°/5° 四次小角度运动。四次请求都被**引擎自身的安全
判据拒绝**（操作 `stage=failed`，日志 `safety error: motion requires confirmed IDLE, HOME and
valid reference`），每次拒绝后引擎按既定策略补发 `stop`。`plan_relative` 在任何串口 MOVE 之前
就返回错误，因此**没有任何机械动作**发生。证据：`.cache/tests/hardware/20261011-motion-settle/`
（含请求/响应 JSONL、拒绝日志，以及随后只读复核的状态）。

原因与结论：烧录前那次只读连接看到的 `homed=false`/`referenceValid=false`/`state=LOCKED` 仍然
成立，而生产运动入口要求已确认 IDLE + HOME + 有效参考。因此：

- **150 ms 末脉冲下降沿实测间隔仍为 BLOCKED**：必须先执行 HOME（台架模式需操作员确认射线物理
  隔离；本次射线未连接）才能做任何运动，HOME 是另一次需要单独授权的机械动作。
- 该拒绝本身是**正确的 fail-closed 行为**（不是缺陷）：引擎在没有可信参考时拒绝相对运动。
- 另需注意：拒绝后的 `stop` 使控制器回到 `LOCKED` 且脉冲计数归零（`positionPulses=0`，
  原先为 -95733）。这不代表发生了运动，但说明该计数在无有效参考时不可作为位置证据。

## 7 单张真实拍摄（主机 RAM，不出束）：实机 FAIL，已定位

授权后做了两次尝试。第一次因驱动在操作仍在运行时关闭 stdin 而被中止（相机实际已完成一次曝光，
但引擎未能接收其事件）；第二次在**等操作排空后**再关闭 stdin，得到了完整证据链。

| 观测 | 值 |
| --- | --- |
| `camera_apply_properties {recordingMedia:1}` | 接受，属性读回确认 |
| `camera_capture` 操作终态 | `cleanup_unconfirmed`，`error = camera timeout: camera owner deadline; COM session isolated; cleanup unconfirmed` |
| 事件诊断 `lastCaptureEventDiagnostic` | `operationCode=37063 (0x90C7 = **GetEvent 事件轮询**，不是拍摄操作码；拍摄操作码需另行取证)`、`responseCode=8193 (0x2001 OK)`、`eventCount=1`、`captureCompleteCount=0`、`exposureEndedObserved=false` |
| 收到的事件 | `code=16390 (0x4006 PropertyChanged)`、`parameter=53684 (0xD1B4 连拍计数)` —— **机身确实完成了一次曝光** |
| 输出文件 | `CameraCaptures/` 为空，**没有 NEF** |
| 会话结果 | `isolated=true`、`cleanupConfirmed=false`，引擎按 fail-closed 隔离会话并以退出码 1 结束（`EOF safety cleanup unconfirmed`） |

第一次尝试留下的孤立事件（后续连接时被判定为故障）为
`camera safety check failed: unassociated capture or device event while idle: code=0xC101,
parameter=0x0B000001; session retired`。

**结论（必须写入交付说明）**：真实 D7100 在主机 RAM 拍摄路径上，本引擎等待的曝光结束/完成事件
在截止时间内**没有到达**（`exposureEndedObserved=false`、`captureCompleteCount=0`），而机身以
`0x4006 PropertyChanged(0xD1B4)` 表示已曝光、并（在第一次尝试中）以 **`0xC101`** 一类 RAM 对象
事件表示结果——与脚本化模型假设的 `0x400D`/曝光结束语义不一致。引擎的处置是正确的
（超时即隔离、清理未确认即失败退出），但**主机 RAM 单拍在本机身上尚不可用**；这属于
**实机 FAIL**，不是离线可发现的问题。下一步需要事件级跟踪与（或）完成判据修正，属于新的实现批次。

**当前设备状态（需人工处置）**：相机会话处于隔离状态，机身 SDRAM 中可能残留一张未取回的图像。
在再次尝试前建议**断电重启相机**（或另行授权清理该 RAM 缓冲对象）；转台停在 `LOCKED`、
`positionPulses=0`、`homed=false`、`referenceValid=false`，未发生任何运动。

## 8 HOME 与转台运动实测（授权后完成）

授权 HOME 后执行：`turntable_connect` → `turntable_bench_mode {enabled:true,
physicallyIsolated:true}` → **`turntable_home`** → 1°/1°/2°/5° 相对运动。注意 `home`（恢复变体）
会以 `PREFLIGHT_REQUIRED` 拒绝，台架独立 HOME 必须用 `turntable_home`。

| 操作 | 实测时长 | 结果 |
| --- | --- | --- |
| `turntable_home` | **4796 ms** | `stage=confirmed`；完成后 `homed=true`、`referenceValid=true`、`hallActive=true`、`rearmed=true`、`state=IDLE`、`positionPulses=targetPulses=-2400` |
| `turntable_relative` 1°（267 脉冲） | 1844 ms / 1868 ms | 完成；`verify_plan_capture_hold` 与 `verify_plan_released` 均通过 |
| `turntable_relative` 2°（533 脉冲） | 2407 ms | 同上 |
| `turntable_relative` 5°（1333 脉冲） | 4001 ms | 同上 |

由 267/533/1333 脉冲三点拟合：**≈2.01 ms/脉冲（≈497 脉冲/秒）**，零脉冲截距 **≈1319 ms**。
该截距包含 MOVE 受理、固件 150 ms 稳定等待、`READY_TO_CAPTURE`、`CAPTURE_DONE`、引擎侧多次
`STATUS` 往返与调度——**150 ms 稳定时间落在其中但无法用本协议分离**：固件只在
`READY_TO_CAPTURE <id> POS=<pos>` 里回报结果，不回报末脉冲下降沿或稳定耗时，而引擎在运动期间
被 worker 占用，外部也无法在 SETTLING 窗口内采样。

因此本项的**实机结论**是：HOME、参考建立、生产方向相对运动、CAPTURE_HOLD 握手与释放全部在真实
控制器上成立（这已经排除了"运动完成判据只靠 ACK"的风险）；而"末脉冲下降沿 → 150 ms → READY"
的**严格隔离测量仍为 BLOCKED**，需要固件侧诊断字段或串口级跟踪（属新的实现批次）。

证据：`.cache/tests/hardware/20261011-home-settle/`（`plan-home.json`、`responses-home.jsonl`，
含被拒的 `home` 变体与成功路径的完整请求/响应）。
