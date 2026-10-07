# 相机与转台最终核验（2026-10-08）

日期：2026-10-08（Asia/Shanghai）。范围：在当前源码上对相机（原生 WPD/PTP，D7100）与转台（CH340/Nano）做一次收尾一致性核验，并与 `Related_files/digiCamControl-master` 源码逐条对照。核验为**只读源码审计**，未连接设备；本轮实机证据见 `tmp/tests/desktop/20261008-030445-xray-manual-verify-r2/` 与 `20261008-031040-camera-session-fixup/`。

完整逐条报告（含两侧 `file:line`，不入库）：
- 相机：`tmp/tests/camera/20261008-camera-digicamcontrol-compare/report.md`（MATCH 40+ / DIFFERENCE-OK 22 / RISK 5 / UNVERIFIED 4 / DOC-ONLY 4）
- 转台：`tmp/tests/turntable/20261008-turntable-final-review/report.md`（MATCH 20 / DIFFERENCE-OK 9 / RISK 3 / UNVERIFIED 3 / DOC-ONLY 3）

## 本轮随 0.9.5 修复的缺陷

**联锁故障被归属成“操作员停止”（扫描结果不确定）。** 射线联锁在投影期间打开时，引擎监控先判定故障并置 `fault_latched`，随后经与操作员 Stop 同一条取消通道结束扫描；扫描 worker 只看到取消标志，于是同一物理故障在 `FAULT` 与 `STOPPED` 之间随机，并打印 “Scan ended by operator; output OFF confirmed”。安全结果（关束确认、投影未提交）一直正确，错的是**原因归属**。
修复：`crates/ct-engine/src/scan.rs` 的 `device_fault_evidence()` 在 worker 侧用自身持有的设备证据判定归属——X-ray 已锁存错误 / 输出仍 ON / OFF 未确认，或相机会话已隔离且清理未确认，一律返回故障结果；确认关束且相机健康时，操作员 Stop 仍是 Stop。
验证：该用例单独跑 10/10 通过（修前 6 次红 3 次、与 camera-loss 同跑 3/3 红）；`scan::tests::r0_` 家族 3×6 全绿；`cargo test --workspace` 连续两次 345 passed / 0 failed / 7 ignored。
同时修了一处**测试竞态**（`devices/xray/moxtek.rs` 的 `owner_monitors_interlock_without_snapshot_or_scan_polling` 只等串口 OFF 不等 worker 发布 `last_error`，断言强度不变）。

## 待办：发布后优先修复的 RISK

| # | 侧 | 问题 | 证据锚点 |
| --- | --- | --- | --- |
| R1 | 相机 | `cameraZoom 0xD1A3` 我们只接受 UINT8 且要求恰好 1 字节 SET；DigiCamControl 声明 `typeof(int)` 并写 8 字节。若 D7100 descriptor 不是 UINT8，LV 缩放永远 UNKNOWN 且不可写 | `nikon.rs:802,862-863`；`NikonBase.cs:1129,1146-1147` |
| R2 | 相机 | `0x90CA` GetVendorPropCodes 解析与 DCC 不兼容（我们 UINT32 count + N×UINT16；DCC 单字节 count、条目自 index 4 起），且 `ptp.rs:437-438` 把该布局误归给 MIT 来源 | `ptp.rs:439-451`、`nikon.rs:254`；`BaseMTPCamera.cs:689-700` |
| R3 | 相机 | 拍摄前事件 drain 可能拒绝所有拍摄（≤8 次轮询后硬失败），DCC 从不为待处理 `0x4006` 拒绝拍摄 | `capture.rs:168`、`nikon.rs:1280-1303`；`NikonBase.cs:1869,1900` |
| R4 | 相机 | 没有任何 `0x100B` 删除 SDRAM 对象，且显式允许 handle 复用，长扫描会持续占用机身对象 | `capture.rs:336-339` |
| R5 | 相机 | 传输后 8 次空队列边界可能拒绝一个已完整传输的 NEF | `capture.rs:344-406` |
| R6 | 转台 | FAULT 后 HOME 很可能被 `ERR HEARTBEAT_REQUIRED` 拒绝：CLEAR_FAULT 清心跳位，而恢复序列 CLEAR_FAULT→STATUS→REARM→STATUS→STATUS→HOME 不发 HEARTBEAT | `rts9060_nano.ino:735,585-591,742`；`device_controls.rs:649-652`、`nano.rs:891-902,785-792` |
| R7 | 转台 | 长运动期间一次迟到 HBACK 超过 1500 ms 即判失败，而空闲路径刻意容忍、固件预算是 2000 ms | `nano.rs:23-24,781-784,675-691`；`ino:85` |
| R8 | 转台 | `preflight` 与恢复 HOME 不检查 `maintenance_lock`/`manual_lock`（只有 `connection_control_allowed` 检查），直连 IPC 可在软件更新维护锁期间做设备动作；目前只有 UI 拦 | `device_controls.rs:168-173,638-641,682-697`；`software_update.rs:26` |

## 仍需实机才能定论（无本轮证据）

- 相机：`0xD1A3` descriptor 真实类型；**“一次物理曝光 → 恰好一个 C101 + 一个 C102 且 handle 匹配”这条核心假设**（DigiCamControl 无任何对应实现可供佐证）；多重曝光物理适用但不可观测；镜身拨盘与硬件 MF 只读项的物理语义。
- 转台：已烧录固件的身份/哈希核对（当前只校验自称的 `RTS9060_NANO/PROTOCOL=2/BUZZER/CAPS`）；规定方向的物理极性、HOME 精度与霍尔几何、20 µs 步进脉冲宽度与 `stepPulseStart` 竞态。
- 与射线联动：更高 kV 成像扫描（4 kV 下管电流实测≈0 µA）、重构精度；撤线后应补测**台架模式 HOME**（引擎规则规定：曾连接射线后不得用台架确认覆盖）。

## 对照结论（无缺口项）

- 转台协议：**不存在“适配器解析了固件从不发送的字段”或“固件发送但适配器不解析”的缺口**；符号约定、`(+180000)/360000` 取整恒等、固定 8/96000、到位证据（ticket + CAPTURE_HOLD + 匹配 CAPTURE_DONE + IDLE）、no-op 计划、角度边界、HOME 有界内部反向、STOP 抢占均 MATCH。
- 相机：`0xD100` 压缩有理快门表（含 Bulb/Time/FlashSync 哨兵）、`0x500D` 0.1 ms、MfDrive 参数顺序、`0xD1A4` 只读 UINT32、ObjectInfo 文件名偏移 53、LV 头 384 字节与大端宽度均 MATCH。

## 文档更正（记录项）

- `docs/backend-revalidation-2026-10-06.md:370` 称 “digiCamControl 的 Nikon 适配器没有 driveMode 写入”与 DCC 源码不符（0x5013 在其祖先类中有 setter）。结论（拨盘不经主机写入）可能仍成立，但需改为“单一实测 descriptor 事实”的表述。
- `crates/ct-engine/src/devices/camera/actor.rs:20-25` 声称完成了某次 EndLiveView 超时的硬件复现，而同目录 `README.md:21-24` 声明全部真实 D7100 行为 BLOCKED；两者需对齐。
