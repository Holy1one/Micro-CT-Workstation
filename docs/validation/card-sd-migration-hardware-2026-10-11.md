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
| 150 ms 末脉冲下降沿实测间隔 | **BLOCKED** | 需授权转台运动；本次未运动 |
| 单卡/Overflow 触发、结束与续存、Stop/心跳、图像质量 | **BLOCKED** | 需授权拍摄与运动 |
| 存卡扫描端到端、1000 秒截止、温度/OFF/联锁、采集至重构全链、RAM/SD 对照提速 | **BLOCKED** | 射线源本次未连接、未上电 |

**未验证即不得声称**：真实 `InitiateCapture`/`ObjectAdded`/`CaptureComplete`/`StoreFull` 语义、
真实卡对象内容与 WPD 传输边界、150 ms 实测间隔、整机联动与提速。`hardwareCertified` 保持
`false`；本记录仅为**授权台架检查**，不构成硬件认证。

**回滚**：如需回到烧录前镜像，用 `before-flash.hex` 以同一 avrdude 参数执行
`-U flash:w:<backup>:i`；该文件是本机唯一证据，删除前须确认已被长期保留。
