# 后端重验证记录 2026-10-06

本文件记录 2026-10-06 本轮「重新验证后端、基本流程与前端接口」的实际证据。所有结论来自本轮命令输出、设备读回与二进制身份；离线门禁与实机结论分别陈述。本文件不覆盖历史验收（见 [接口验收记录](backend-interface-acceptance-2026-10-05.md)），也不等同于硬件认证完成。

当前结论以 §5z–§5ab、§7 及任务清单第八节为准；§1–§5 和各早期轮次保留其当时的版本与失败事实，不代表最新状态。第 19 轮 T7 已 PASS；第 20 轮补充原失败的驱动/取消时序证据，F-12 精确根因仍未确证。

## 1. 源码与二进制身份

| 项目 | 值 |
| --- | --- |
| 仓库 | `E:/LZU_Master/MicroCT/Micro-CT-App` |
| 分支 / HEAD | `native-device-controls-offline` / `11eed6cfea8f3f5118fcd07e70319f98fc398b51` |
| 工作副本 | 有未提交修改；源码身份以各 run 目录 `result.json` 的逐文件哈希为准，不以 HEAD 代替 |
| 应用版本 | `0.9.0-rc.3` |

### 构建与运行身份

| 阶段 | host SHA-256 | 内嵌/运行引擎 SHA-256 | 命令 / 退出码 |
| --- | --- | --- | --- |
| T1 基线（未修改源码） | `47B3C006DBDE98F453CF5417F529E06727FCAD5AC399494034861C78CC1ED783` | `E7BF81010C0FD8A393D14C5A05559CB7762E957ADABD09DD4F9BB31E18A04A61` | `tauri build --no-bundle` / 0（103.3 s） |
| 修复构建 1（F-01 首次尝试） | `6C1F8792BEFBEAC5BEC66E88BEE0A4C1768F8087FB9254AE994FED05DF9F40E6` | `D70B1F4124B862CA8E88453454B52584C10B23B2E05E8428B9283ADC18903124` | 同上 / 0（121.6 s） |
| 修复构建 2（有界重试） | `87BC1AAF6DE18D2B2BBE1FA4A10F43A2A3FA867086719D5B15E9B188176B3794` | `31E08296B52851EC571DC8381B572DEAE48D5EA714F063AF644047B4D9C1AA6E` | 同上 / 0（约 80 s） |
| **修复构建 3** | `DF7ACC909E50922DFEF14F29EEDD01674F42107E73DF89279C4C55940B51F855` | `E79109B6DB63C805A0BF1ED60B650F64E61EEFF22C95DA0AE4FC4F271E171D56` | 同上 / 0（85 s）；F-01 有界重试 + 显式恢复入口 + F-03/F-04 入队前门禁 |
| **修复构建 4** | `9893489D43E5C971FD1EA595EE71C2358C8CAC37A5E3E117B909AF97AD43FE77` | `042C3123598DCCD326B87F53989357B2B2CCFAB5BDF2C05635B4F40484C457E2` | 同上 / 0（76 s）；第 2 轮：F-02 分码、F-06 `camera_discover`、`capture_readiness` 门禁、恢复入口范围收敛（V-01）、诊断合并（V-02）、载荷文本还原（V-04）、xray 间歇测试同步 |
| **修复构建 5** | `445A6E8D6B799621F320E3E6E5F927589FB72535AC05BE9CCD140496DFA6D6F2` | `D68F99D999285F4870B60487C3BFF5538CFF9D7169D54184AF23CEB649A80611` | 同上 / 0（76 s）；第 3 轮：V-06 发现设备数不再升级为全局故障 |
| **修复构建 6** | `AE1D519210931FD70C6399A526AD3AC3FCAD30FFB1CA67266F7BADD272973BCB` | `5F87D2927ECA5ED4AC844B90528DE08048DC8497D8A593D0E61467A885DED9F9` | 同上 / 0（76 s）；第 3 轮收尾：D-1 锁判据放宽为包含匹配、D-4 单一 `capture_readiness`、D-7 lint、D-3/D-5 回归测试、请求校验优先级恢复 |
| **修复构建 7（第 4 轮实机所用）** | `85995F09D293D62C44E78178B0D5F1A7B1506DB0DBA41C432DE30CF8BDC345E3` | `CC22C875334A668F00010B9BB1E08DE3C9A3FA51C8649380F96AB183C90A3888` | 同上 / 0；源码相对构建 6 只多一个 `#[cfg(test)]` 词汇表契约测试。**注意**：内嵌引擎哈希每次重链接都会变（PE 头时间戳），实测运行引擎 `ct-engine-cc22c875334a668f.exe` = `CC22C875…`，与 `target/embedded-engine/release/ct-engine.exe` 一致；不能用"与上一构建相同"推断 |
| **修复构建 8（LV 确认超时预算尝试）** | `419A15C8FB84C7F78DCB36C820179C685A9ED2E274978AEB214F2982CBDC9448` | `EBEA733C9D7A9CCACA34971C6590B1A3AB8ADF8EC406A9036DD84974DADF522D` | 同上 / 0；`cleanup()` 的 LV OFF 确认预算由 `wait_limit`(2 s) 改为 `max(wait_limit, COMMAND_TIMEOUT)`(15 s)。**实机证明该改动不足**（见 §5f/§5g）：两次尝试共 17 s 仍双双超时 |

实际运行引擎路径形如 `%LOCALAPPDATA%\MicroCTWorkstation\runtime\ct-engine-<sha8>.exe`，由 `process.json` 记录；T1 实测 `ct-engine-e7bf81010c0fd8a3.exe` = `E7BF8101...`，与 `target/embedded-engine/release/ct-engine.exe` 一致（外侧 `target/release/ct-engine.exe` 是另一条产物，SHA 不同，不用于内嵌身份）。

正式入口 `portable-release/micro-ct-workstation-portable.exe` 本轮**未改动**，仍为 `C19B828D8F18D13D3EF2811B95B5094863AEE0E494611140001BC69716C91334`。未发布、未安装、未覆盖正式入口。

中断构建 `tmp/tests/desktop/20261006-142717-connected-backend-host/` 原样保留为失败记录；本轮在 `20261006-150714-connected-backend-host/` 重新构建成功。

## 2. 离线门禁（修复后）

| 门禁 | 命令 | 结果 |
| --- | --- | --- |
| Rust 全量回归 | `cargo test --workspace` | **PASS** exit 0：ct-engine lib **271**、main 2、ct-workstation 52 → **325 passed / 0 failed / 7 ignored**；连续多轮全绿（第 2 轮开始前该命令曾 1 次失败于 xray 间歇竞态，已定位并修正测试同步，见下） |
| 前端类型 | `npm run typecheck` | **PASS** exit 0 |
| 模块图谱 | `npm run module-map:check` | **PASS** exit 0（1852 files / 16 modules / 25 boundaries） |
| 前端合同测试 | `node --experimental-strip-types --test tests/*.test.mjs` | **PASS** 165 / 0 fail |

**门禁稳定性修正（E）**：第 2 轮独立验证首次运行 `cargo test --workspace` 得到 101（ct-engine lib 265 passed / 1 failed / 7 ignored），失败用例为 `devices::xray::moxtek::tests::owner_enforces_continuous_output_deadline_without_domain_polling`。根因是**测试同步**而非本轮改动：用例以原始 `FakeState.enabled` 为退出条件，而 worker 在 `force_off()` 内先清 `enabled`、随后才发布 fault/hard limit 错误（`moxtek.rs` 该用例与 worker 的发布顺序），并行负载下断言落在该窗口。修正为等待断言真正需要的证据（仍有 2 秒上界），断言强度不变；单测复现日志与结论见证据目录。修正后连续 3 次 `cargo test --workspace`（fail-fast 配置，即原先失败的配置）全部 exit 0。
| 离线审计（子智能体） | Rust 静态 36 条、合同 9 点、文档 27 差异 | 见第 5 节证据目录；三项审计均未修改受版本控制文件 |

7 个 ignored 全部是有意的人工/硬件门禁。

## 3. 实机结果（D7100 + CH340/Nano）

执行方式：用当前源码构建的 host 候选，经真实 Tauri WebView 的 `window.__TAURI_INTERNALS__.invoke` 走生产链（React→Tauri→EngineClient→JSONL→ct-engine）。每次运行使用独立 `MICRO_CT_ENGINE_DATA_DIR` 与独立 CDP 端口；照片/日志全部落在本次 `tmp/` 隔离根。

| ID | 结论 | 关键证据 |
| --- | --- | --- |
| T2 链路复核 | **PASS** | 50 次快照 `snapshotSequence` 单调递增 0 违例、`engineSessionId` 唯一；未知命令 `ENGINE:UNKNOWN_COMMAND`、缺 type `Command type is required`、无资源 `CAMERA_NOT_CONNECTED` / `NANO_NOT_CONNECTED` / `LV_NOT_ACTIVE` / `CAPTURE_NOT_REGISTERED`；被拒命令不产生 operation receipt；`mode=production_locked`，无预览回退 |
| T3 身份/参数读取 | **PASS** | 显式 `deviceId` 才连接（无 deviceId 仅发现且不隐式选择，operation kind `camera_discover`；发现到 0 或 ≥2 台设备只是操作员提示，完成而不升级为故障——第 3 轮 V-06，已实机 A/B）；型号 `D7100`、backend `windows_wpd_nikon_d7100`、serial `8322769`、`sessionId` 32hex；摄影目录 **34 项**；`exposureMinMs=0.125` / `Max=30000`；`runtimeProperties` 10 项、`semanticStatus` 全为 `unknown`；`multipleExposureEvidence.status=unknown` |
| T4 Apply / 依赖 / 恢复 | **PASS（含 1 个 FAIL 子项）** | `iso 100→200` 读回成功；父参数 `autoIso 0→1` Apply 后子参数 `autoIsoHighLimit` 由 `writable=false` 变为 `true`（依赖解锁在明确 Apply 后生效）；`shutter=65537` 原生编码写入后 `parameters.exposureMs=1000`；恢复原始值后与本次鲜读原值 **34 项全等（diffCount=0）**。FAIL 子项见 F-02/F-05 |
| T5 Live View | **PASS** | Start 生成新 32hex nonce（`3eccae6d…`、重启后 `8389bfad…`），`liveViewControls.sessionId == liveView.sessionId`；`cameraZoom` code `0xD1A3`、`focusMode` code `0xD061`，物理 MF(3) 只读且 options 全部 `enabled=false`；32 s 仅内存传输统计 127 帧 / 640×424 / 0 null / 0 重复 / 序号单调 / 同一 nonce；停止后旧 nonce 读取得 `ENGINE:STALE_LIVE_VIEW_SESSION`，新 LV 期间旧 nonce 亦为 `STALE_LIVE_VIEW_SESSION`；实时帧只以 `{sessionId,sequence,width,height}` 形态落盘，未写任何 JPEG/截图/base64/像素数组 |
| T6 LV 中错误与并发 | **部分 PASS** | LV 中普通 Apply（含 no-op 与空对象）被拒、`camera_capture` 与 `camera_test_capture` 均为 `LIVE_VIEW_CAPTURE_FORBIDDEN`、非法 LV 控制 `INVALID_LIVE_VIEW_CONTROLS`（普通摄影 key / 非 UINT8 / 空 / 3 项）、外来 nonce `STALE_LIVE_VIEW_SESSION`；每次拒绝后 LV 仍 active（无自动停预览）；相机故障类请求（见 F-03/F-04）**不是**干净拒绝而是 fault+断连 → **FAIL** |
| T7 独立 NEF | **BLOCKED** | 显式 Apply 与「先显式停 LV」均通过；`camera_capture` 被 `camera safety check failed: scan requires driveMode=1` 拒绝。当前机身 `driveMode=0x8011`（自拍，只读），软件无法改写；`capabilities.captureReady` 事前已为 false。**需操作员把释放模式拨到单张（S）后重跑**。未产生任何 NEF |
| T8 Nano / 退出 | **PASS** | `RTS9060_NANO` v2.1.0 build 20260918 protocol 2 COM7；status `LOCKED/homed=false/referenceValid=false/microsteps=8/positionPulses=-1141/pulsesPerRev=96000`，`confirmedAngleDeg=null`（不把目标当实测）、`hardwareCertified=false`；STOP 与断开各自确认；legacy `connect{adapter:real_hardware}` 只连 Nano（相机保持 offline）；非法运动角度与无隔离 HOME 均在入队前拒绝（`INVALID_MOTION_ANGLE` / `HOME_REQUIRES_XRAY_OFF_OR_ISOLATED_BENCH`，无任何运动）；射线全程 `beamState=unknown`、`xrayAvailable=false`、`manualControlsEnabled=false`；正常退出 **app 0 / engine 0 / 无强制结束** |
| T9 扫描准备与门禁 | **PASS** | `prepare_scan`（measured + counterclockwise + maxXraySec=600）内存提交：`taskId/angleStepDeg=45/exposureMs=1000`，**未创建任务目录**；非空既有任务目录 → `SCAN_OUTPUT_EXISTS_USE_NEW_TASK` 且原文件未被改动；`CameraCaptures` 作为 taskId → `RESERVED_OR_INVALID_TASK_ID`；`start_scan` → `PREFLIGHT_REQUIRED`（入队前，不可能产生运动）；`preflight` → `XRAY_NOT_CONNECTED`；`pause` → `NOT_RUNNING`；`confirm/cancel_scan_stage` → `SCAN_PROMPT_MISMATCH`；`abandon_scan` → `NO_ACTIVE_TASK`；`start_reconstruction cgls` → `RECONSTRUCTION_METHOD_UNAVAILABLE`，`fdk` 受理后 `RECONSTRUCTION_LOCKED`（无已提交投影）；`cameraScanReadiness.issues` 报出 multipleExposure / driveMode / longExposureNoiseReduction / recordingMedia 四项 |
| 真实扫描 | **BLOCKED** | 射线未连接且无出束授权；`multipleExposureEvidence=UNKNOWN` 使 `proves_inactive()` 恒 false，真实扫描按设计阻断。未尝试任何开束、写射线参数、完整扫描或重构 |
| HOME / MOVE | **BLOCKED** | 无当前现场运动空间与射线物理隔离条件；引擎在入队前即拒绝，未发生任何运动 |

### 接口精度实测确认

- `Operation.deviceSessionId`：相机发现/连接阶段回退 `engineSessionId`；相机业务操作使用当时的相机连接 session（如 `a027ef5ad3cf337e6924482fa59c9e18-op-3`）；转台/安全/任务操作为 `engineSessionId`。**不得统一强制相等**（实测两种取值同时存在）。
- 每次 `camera_live_view_start` 生成独立 32hex nonce；`CameraHealth.sessionId` 与 capture session 是稳定连接标识；重连后 old session 不被新 session 覆盖（`8126355a…` → `60b51b63…` → `f8ba0b68…` → `a027ef5a…`）。
- `camera_live_view_read/configure` 的 `sessionId` 必须是当前 LV nonce；malformed 与 stale 分别有独立错误路径。
- 停产入口的实际限制与源码一致：`set_parameters` → `STAGE_1_NANO_ONLY`；`timer_toggle` → `XRAY_STAGE_LOCKED`；`connect{adapter:developer_preview}` → `PRODUCTION_LOCKED`；`retry_device camera` 实际只做发现（见 F-06）。
- `prepare_scan` 要求 `measurement=measured` + `rotationDirection=counterclockwise`，`maxXraySec` 1–600。

## 4. 本轮确认的缺陷

| ID | 严重度 | 状态 | 摘要 |
| --- | --- | --- | --- |
| F-01 | 高 | **已修复（第 7 轮实机验证）** | LV 期间全局 Stop 原先使相机 `cleanup_unconfirmed` 永久隔离、退出挂起。根因：Stop 先 `request_cancel()` 使 `EndLiveView (0x9202)` 事务不再返回（15 s 预算也耗尽），而同一命令在未取消的会话上约 0.6 s 即确认。修复：Stop 在取消之前先用有效会话做一次有界（3 s）Live View 停止。实机（构建 `B976E2B3` 与 `5CD13EB7` 两次）：Stop **1.04–1.08 s**、`cleanupConfirmed=true`、`phase=stopped`、无 fault/锁，随后 `camera_connect` 与 `camera_read_properties` **直接成功**；正常退出 **app 0 / engine 0**（此前 90 s 内无法关闭、engine exit 1）。详见 §5k |
| F-02 | 高 | 已修复（离线），实机待验 | 五种语义不同的 `camera_apply_properties` 拒绝原先折叠为同一 `ENGINE:UNSUPPORTED_CAMERA_PROPERTIES`。修复：`properties.rs` 新增 `PropertyRejection` 分类（`code()`/`message()`/`into_camera_error()`），域入口改用 `camera.property_rejection(...).map_err(|r| r.code())`，错误码分为 `LIVE_VIEW_APPLY_FORBIDDEN` / `INVALID_CAMERA_PARAMETER_COUNT` / `CAMERA_PROPERTY_UNDESCRIBED` / `CAMERA_FOCUS_MODE_LOCKED` / `CAMERA_PROPERTY_WRITE_UNSUPPORTED` / `CAMERA_PROPERTY_UNSUPPORTED_VALUE` / `CAMERA_PROPERTY_DEPENDENCY_UNMET` / `CAMERA_PARAMETER_SESSION_UNAVAILABLE`；`preset_load` 仍保留旧码。**文本精度（含独立验证修正）**：9 条原始诊断字符串逐字不变（由独立验证者用改动前编译产物的 UTF-8 字面量取证）；但 apply 路径的 per-key 失败载荷一度丢失了 `CameraError::Display` 的 `camera capability unavailable: ` 前缀，已改回经 `into_camera_error().to_string()` 渲染，载荷与改动前一致 |
| F-03 | 高 | 已修复（离线），实机待验 | `capabilities.captureReady=false`（机身 driveMode≠1）时 `camera_capture` 仍被受理，worker 内失败 → fault_latched + Stop + 相机会话被摧毁。修复：入队前调用与 worker 完全一致的 `capture_readiness()`（策略读回 + LV OFF + 已确认只读 `infoDisplayErrorStatus` 运行诊断）拒绝为 `CAMERA_CAPTURE_UNQUALIFIED`，并同时覆盖 `camera_test_capture`（独立验证项 V-03 已闭合） |
| F-04 | 高 | 已修复（离线），实机待验 | `camera_focus{step:-10}` 在 LV 未开启时通过入队前校验，worker 内 `focus step requires current Live View session` 失败 → fault + 断连。修复：入队前拒绝为 `CAMERA_FOCUS_UNQUALIFIED`（`camera_autofocus` 同理），并补回归测试 |
| F-05 | 高 | 未修 → **升级为待人工决策**（见 §5d） | `autoIsoHighLimit` 广告 `writable=true`、`range=[0,25,1]`，但写入区间内值 10 被机身以 PTP `0x201c`（InvalidDevicePropValue）拒绝 → 部分失败 → fault + 断连。descriptor 的区间/枚举是格式域，不等于机身接受的离散值集合 |
| F-06 | 中 | 已修复（离线），实机待验 | `retry_device{device:"camera"}` 与无 deviceId 的 `camera_connect` 只做发现，却产生 `kind="connect"`、`status="completed"` 的 receipt（且会清空此前 `camera.lastError`），相机仍 `connected=false`。修复：两条路径的 operation kind 改为 `camera_discover`，只有带明确 deviceId 才是 `connect`；发现回执附带说明日志 |
| F-07 | 中 | 部分修复（实机待验） | 一次未确认的 LV 清理之后**机身物理上仍停留在 Live View**；此后 `camera_connect` 被 `camera safety check failed: camera already has Live View or unknown Live view status; close it before connection` 拒绝。已修复**进程内**部分：`acknowledgeUnconfirmedCleanup` 退役那台传输已关闭、永远无法确认清理的隔离代理，使新选定的相机会话得以打开；只释放原因确为 `Camera cleanup unconfirmed…` 的锁，保留跨设备 `fault_latched` 与 Abandon 锁（独立验证项 V-01 闭合；回归测试 `acknowledged_camera_cleanup_retirement_is_scoped_to_the_camera_lock` 证明 ack 后新适配器能完成 connect、且故障锁未被清除）。**机身侧 Live View 仍需人工退出**，软件无路径清除 |

F-01/F-02/F-03/F-04/F-05 的根因与最小修复方案另见：
`tmp/tests/desktop/20261006-151200-connected-device-session/DEFECT-F01-live-view-stop-cleanup-unconfirmed.md`
`tmp/tests/desktop/20261006-151500-rust-offline-audit/summary.md`（PF-02/05/06/07）
`tmp/tests/desktop/20261006-151500-contract-audit/summary.md`

## 5. 本轮修改的文件

| 文件 | 修改 | 依据 |
| --- | --- | --- |
| `crates/ct-engine/src/devices/camera/actor.rs` | `cleanup()` 改为：先用有界重试（`confirm_live_view_off`）确认 LV OFF，再保留 `transport.cancel()` 中止在途传输，取消后再重试一次；失败时把真实传输错误写入 `CameraCleanupDiagnostic.error` | F-01 |
| `crates/ct-engine/src/device_controls.rs` | `camera_connect` 支持显式操作员确认字段 `acknowledgeUnconfirmedCleanup:true`（必须同时给出明确 `deviceId`，且仅当相机确实处于 isolated 且 cleanup 未确认时生效）：释放该永久锁并记录 WARN，旧会话仍保持隔离与未知；新增 `camera_unconfirmed_cleanup()` 判定 | F-01/F-07 恢复出口（用户已授权） |
| `crates/ct-engine/src/device_controls.rs` | 在入队前按已发布的 readiness/capability 拒绝：`camera_capture` → `CAMERA_CAPTURE_UNQUALIFIED`、`camera_focus` → `CAMERA_FOCUS_UNQUALIFIED`、`camera_autofocus` → `CAMERA_AUTOFOCUS_UNQUALIFIED`；门禁放在静态输入校验与 LV 拍摄禁令之后，保持既有错误码优先级 | F-03/F-04 |
| `crates/ct-engine/src/device_controls.rs` | 新增回归测试 `unqualified_focus_and_autofocus_are_rejected_before_operation_without_fault` | F-04 |
| `crates/ct-engine/src/devices/camera/properties.rs` | 新增 `PropertyRejection`（`code()`/`message()`/`into_camera_error()`）与 `property_rejection()`；`validate_control_values` 返回 `PropertyRejection`，操作员消息逐字保持不变 | F-02 |
| `crates/ct-engine/src/devices/camera/nikon.rs` | `validate_properties` 改为包装 `property_rejection`；apply 路径 per-key 失败改用 `error.message()` | F-02 |
| `crates/ct-engine/src/devices/camera/mod.rs` | 导出 `PropertyRejection` 与 `property_rejection` | F-02 |
| `crates/ct-engine/src/devices/camera/actor.rs` | 新增 `CameraAdapter::property_rejection()` | F-02 |
| `crates/ct-engine/src/device_controls.rs` | `camera_apply_properties` 改用分类码；无 deviceId 的 `camera_connect`（含 `retry_device camera`）operation kind 改为 `camera_discover`；新增回归测试 `discovery_only_camera_connect_reports_a_discovery_receipt` | F-02/F-06 |
| `crates/ct-engine/src/devices/camera/properties.rs` | 新增 `capture_readiness()`：worker 在拍摄前实际要求的全部条件（策略读回 + LV OFF + 已确认只读 `infoDisplayErrorStatus` 运行诊断），供领域入口入队前使用 | F-03 / 验证项 V-03 |
| `crates/ct-engine/src/device_controls.rs` | 拍摄门禁改用 `capture_readiness()` 并覆盖 `camera_capture` 与 `camera_test_capture` 两条 worker 路径 | V-03 |
| `crates/ct-engine/src/device_controls.rs` | 恢复入口范围收敛：改为**退役**不可用的隔离代理（其传输已关闭、永远无法确认清理），只释放原因确为 `Camera cleanup unconfirmed…` 的锁；不再清除跨设备共享的 `fault_latched`，也不释放 Abandon 锁；新增回归测试 `acknowledged_camera_cleanup_retirement_is_scoped_to_the_camera_lock` | 验证项 V-01 |
| `crates/ct-engine/src/devices/camera/actor.rs` | `CameraCleanupDiagnostic.error` 改为在已有错误后**追加**真实 LV 传输错误，并同样并入 `health.last_error`；`confirm_live_view_off` 去掉无用的 Option 包装 | 验证项 V-02 |
| `crates/ct-engine/src/devices/xray/moxtek.rs` | 仅测试同步：`owner_enforces_continuous_output_deadline_without_domain_polling` 等待断言所需的错误证据而非首个 `enabled` 标志（2 秒上界不变，断言不变） | 验证项 E |
| `crates/ct-engine/src/device_controls.rs` | 无 deviceId 的 `camera_connect`（含 `retry_device camera`）在发现设备数为 0 或 ≥2 时**不再返回 Err**：operation 以 `camera_discover` 完成，设备数只作为操作员提示写入引擎日志，`discovered` 列表仍是机器可读信号；不再因此 `fault_latched` 与全局 Stop | 验证项 V-06 |
| `crates/ct-engine/src/device_controls.rs` | 第 3 轮收尾：恢复入口锁判据由 `starts_with` 改为 `contains("Camera cleanup unconfirmed")`（`Stop` 会把多个原因拼进同一把锁，前缀匹配会让入口不可达） | 验证项 D-1 |
| `crates/ct-engine/src/device_controls.rs` | 第 3 轮收尾：入队前 readiness 门禁移到**全部请求校验之后**（LV 拍摄禁令与 `CAMERA_CAPTURE_ROOT_REQUIRED` 等文档化错误码优先级由此恢复） | 自审发现（独立验证未覆盖） |
| `crates/ct-engine/src/devices/camera/properties.rs` | 第 3 轮收尾：`refresh_property_controls` 的 `captureReady` 判定改为直接调用 `capture_readiness`，发布值与入队前门禁共用同一实现，不再各自维护两份逻辑 | 验证项 D-4 |
| `crates/ct-engine/src/devices/camera/actor.rs` | 第 3 轮收尾：`confirm_live_view_off` 去掉死初值，改为在最后一次尝试后直接返回真实错误（语义不变，消除 `unused_assignments` 告警） | 验证项 D-7 |
| `crates/ct-engine/src/devices/camera/tests.rs`、`scan.rs` | 测试夹具新增 `ScriptedCameraConfig.discovered`（手写 `Default` 保持默认 1 台），使"发现 0/2 台"可被测试 | 验证项 D-5 |
| `crates/ct-engine/src/device_controls.rs` | 新增回归测试 `discovery_device_count_is_an_operator_prompt_not_a_global_fault`（0 与 2 台两例）、`capture_readiness_requires_the_runtime_diagnostic_evidence`、`unready_capture_is_rejected_before_operation_without_fault` | D-5 / D-3 |
| `crates/ct-engine/src/device_controls.rs` | 第 4 轮：新增词汇表契约测试 `camera_parameter_rejection_vocabulary_is_stable`，锁定 8 个分类码的唯一性、禁止回退到旧 `UNSUPPORTED_CAMERA_PROPERTIES`、并固定两条操作员文案 | F-02 防漂移 |
| `docs/frontend-functional-spec.md` | 第 4 轮：按独立审计 SA-01…SA-12 修正 12 处（拒绝码真实顺序、Apply 部分失败后果、实机覆盖归属、`captureReady` 表示法、故障锁清除路径、门禁边界、计数、竞态码说明、LV 帧字段、`confirm_scan_stage` 必填、兼容入口硬拒、导入取消先清反馈） | 规范审计 |
| `src/engine/types.ts` | `ScanFlowView.stage` 补 `"persisting_completed"`；`camera_connect` 增加可选 `acknowledgeUnconfirmedCleanup` | 合同审计 #3；恢复出口 |
| `src/platform/updater.ts` | `importSoftwareUpdate` 返回类型收窄为 `Promise<UpdateState>`（native 取消时返回当前状态而非 null） | 合同审计 #4 |

未修改设备协议、扫描事务、安全门禁、原始数据或正式入口。未运行 `portable:build`、未安装、未发布。

## 5b. 独立验证与整改（第 2 轮）

独立验证者（`fix-verifier`，只读、对抗性）逐条复核了第 1/2 轮改动并按证据证伪，报告与全部取证脚本见 `tmp/tests/desktop/20261006-163000-round2-fix-verification/`。结论与本轮处置：

| 项 | 验证结论 | 处置 |
| --- | --- | --- |
| A.1 F-01 有界重试与清理顺序 | PASS（含诊断偏差） | — |
| A.2/A.4/A.5 `PropertyRejection`、包装、per-key 文本 | PASS（含 V-04） | V-04 已改回经 `into_camera_error()` 渲染，载荷与改动前一致 |
| A.3a `acknowledgeUnconfirmedCleanup` | **FAIL（V-01）** | 已整改：退役隔离代理而不是复用；锁释放限定到 camera-cleanup 原因；不再清除共享 `fault_latched`；新增端到端回归测试 |
| A.3b/`captureReadiness` 门禁范围 | PASS（含 V-03） | 已整改：改用与 worker 一致的 `capture_readiness()`（含运行诊断），并覆盖 `camera_test_capture` |
| A.3d `camera_discover` kind | PASS（含 V-06） | V-06 保留为已知残留（见下） |
| B 契约漂移 | PASS | apply 路径不再返回旧码；无消费者硬编码旧码或旧 kind |
| C 安全语义 | **FAIL（V-01 顺带解锁）** | 已整改并加测试；LV 期间 Apply/拍摄拒绝未削弱 |
| D 文本一致性 | PASS（V-04 偏差） | 已整改 |
| E 门禁 | 首次 101、`--no-fail-fast` 320/0/7 | 已定位为 xray 测试同步竞态并修正；连续 3 次 exit 0（321/0/7） |

**已知残留（未在本轮修改，逐条记录）**
- **V-05**：8 个新错误码与 `acknowledgeUnconfirmedCleanup` 目前**没有前端消费方**——`src/`、`src-tauri/`、`tests/` 中无任何地方解释这些码，恢复入口在现有 UI 上不可达。前端需要新增错误映射与确认对话框（见 `frontend-functional-spec.md`）。
- **V-06（第 3 轮已修并实机验证）**：发现型 `camera_connect` 在设备数 ≠1 时原先返回错误并经 `record_operation_error` 升级为 `fault_latched` + 全局 Stop。现在发现回执以 `camera_discover` 正常完成，设备数只写日志，`discovered` 列表如实发布。**实机 A/B（同一台机器、相机缺席即 0 设备）**：修复前 `phase=fault`（`tmp/tests/desktop/20261006-171000-round2-device-acceptance/`）；修复后 operation `camera_discover` = `completed`、`phase=idle`、`lastError=null`，随后 Nano 连接/STOP 与正常退出 0 不受影响（`tmp/tests/desktop/20261006-174000-round3-device-v06/`）。
- **F-05**：descriptor 的 `range/options` 由原生格式区间推导，不等于机身实际接受的离散值集合（`autoIsoHighLimit=10` 被 PTP `0x201c` 拒绝）。本轮未改：离线无法得知机身接受的集合，且"拒绝值即缩小本会话目录"需要先定义会话级目录收窄语义。
- **F-01 的实机验收**：仍 BLOCKED（相机未枚举）。

## 5c. 第 3 轮独立验证与整改

第 3 轮再次由独立验证者（`final-verifier`，只读、对抗性）复核**整改后的最终工作副本**，报告见 `tmp/tests/desktop/20261006-180000-round3-final-verification/`：8 组必答项 **全部 PASS**（V-01 退役代理与锁范围、V-02 追加诊断、V-03 门禁与 worker 对齐、V-04 前缀恢复、E 测试同步、V-06 计数不故障、F-02/F-06 未回退、`preset_load` 旧码保留），`cargo test --workspace` 连续 2 次 exit 0。它同时给出 7 条保留（D-1…D-7），本轮处置如下：

| 项 | 级别 | 处置 |
| --- | --- | --- |
| D-1 多原因拼接锁使前缀判据不可达 | medium | **已修**：改为包含匹配，并保留"Abandon 锁不含该标记"的判据 |
| D-3 `capture_readiness` 未覆盖 worker 的 `recordingMedia` 新鲜回读 | medium | **部分闭合**：缓存态 `recordingMedia=1` 其实已由 `independent_readback` 覆盖，因此不重复加入；`infoDisplayErrorStatus` 已确认只读这一项由 `capture_readiness` 提供，并新增直接回归测试。**新鲜回读按定义无法在入队前完成**，仍是 worker-only 的 fail-closed 检查，已写入函数文档 |
| D-4 `captureReady` 与门禁两份重复实现 | low | **已修**：`refresh_property_controls` 直接调用 `capture_readiness` |
| D-5 V-06 无有判别力的测试 | medium | **已修**：夹具支持可变发现数量，新增 0 台/2 台两例回归；若回退旧实现，断言 `status=="completed"`/`!fault_latched`/无 `stop` 必然失败 |
| D-7 遗留 `unused_assignments` | low | **已修**：去掉死初值 |
| D-2 ack 后旧清理证据从快照消失 | low | **按事实修正注释**：旧会话仍记录在 operation 历史、WARN 日志与共享故障锁中；快照在新会话连接后如实显示新会话，不再声称旧证据继续由快照发布 |
| D-6 `acknowledgeUnconfirmedCleanup` 与 8 个新码无前端消费方 | medium | **未修（有意）**：属新前端的实现范围，已在 `frontend-functional-spec.md` 明确列出需要补的错误映射与确认流程 |

另有**自审发现并已修**的一处（独立验证未覆盖）：第 2 轮把 readiness 门禁放在请求校验之前，使 `camera_capture` 在缺少保存根时不再返回文档化的 `CAMERA_CAPTURE_ROOT_REQUIRED`。现已把门禁移到全部请求校验之后，并加回归断言固定该优先级。

## 5d. 待人工决策：F-05 参数拒绝的后果语义

**现状（实机证据）**：`camera_apply_properties{autoIsoHighLimit:10}` 在 descriptor 广告 `writable=true`、`range=[0,25,1]`、`options=0..25` 的情况下被机身以 PTP `0x201c` 拒绝；引擎把它记为"部分失败"，但随后 `record_operation_error` 按"相机操作失败"处理：`fault_latched=true` + 全局 Stop → **整个相机会话被断开并隔离**，此后连"恢复本次鲜读原值并核对差异"（T4 的明确要求）都无法在同一会话内完成。独立验证（`tmp/tests/desktop/20261006-163000-round2-fix-verification/`）与自审都确认这是"可预判的失败被升级为设备故障"的同一类问题。

**为什么没有在本轮直接修**：任何"不再 fault"的改动都会削弱 fail-closed 响应，按仓库规则必须有实机证据才能定案；而当前 D7100 未枚举，无法取得该证据。属**有意保留**，不是遗漏。

**可选方案**

| 方案 | 内容 | 优点 | 风险 / 所需证据 |
| --- | --- | --- | --- |
| **A（推荐）** | 仅对 `camera_apply_properties` 的**部分失败**放宽：当每个请求属性的**回读仍然完整且已确认**（即状态已知、设备有响应）时，操作记为 `failed`、预检失效，但**不**置 `fault_latched`、**不**发全局 Stop、**不**隔离会话。真正的设备级失败（超时、掉线、I/O 错误、回读缺失/不一致）仍按现在 fail-closed 处置 | 与 AGENTS 的"fail closed 针对错误/超时/掉线/未知状态"一致：此处的状态是**已知**的；恢复流程得以在同一会话完成；操作员能改值重试 | 需要实机验证：(1) 被拒值后会话仍可用、可继续读回；(2) 预检确实失效、不能借此通过扫描门禁；(3) 超时/掉线/回读失败仍 fault 且断连；(4) 不产生"部分成功"误报 |
| **B** | 保持现行为，但把被机身拒绝的**具体值**记入会话级目录（从 `supported_values`/`options` 移除并给出理由），使前端不再提供该值 | 不改故障策略 | 需要会话级收窄状态并定义与 `refresh_property_controls`/后续读回的关系；首次错写仍会断连 |
| **C** | 不改代码，仅在文档中声明"descriptor 区间是格式域，实际可接受值集合由机身决定"，前端只用已确认过的值 | 零风险 | 现场一次错值就要重连；与 F-03/F-04 的"可预判失败不应断连"原则不一致 |

**建议**：采用 A（范围严格限定为"部分失败且回读完整"），并把 B 作为后续增强。决策与实机验证应在相机恢复枚举后进行；在此之前前端应按 C 的保守用法处理，并保留 `DEVICE_CLEANUP_UNCONFIRMED` 与 `acknowledgeUnconfirmedCleanup` 的恢复路径。

> **后续处置（第 10 轮，见 §5p/§5q）**：方案 A 已落地。同一 `PTP 0x201c` 值拒绝现在记为操作状态 **`rejected`**（不 `fault`、不 Stop、不隔离会话，`preflight` 仍失效），并在实机 A/B 验证（`phase=fault` + 会话断开 → `phase=idle` + 会话保持）；第 11 轮又收紧了分类器，使**只有**带实机证据的 `0x201c` 计入 `rejected`，其它 PTP 码与传输失败仍按 `failed` + 故障处理。本节下方内容保留为当时的决策记录。

## 5e. 第 4 轮：接口规范对最终修订的独立审计与修正

第 4 轮由独立审计者（`spec-auditor`，只读）把 `docs/frontend-functional-spec.md` 逐条断言与**最终修订源码**对照，报告见 `tmp/tests/desktop/20261006-183000-spec-accuracy-audit/`：62 条判定 = ACCURATE 47 / STALE 5 / OVER-CLAIMED 4 / MISSING 4 / BLOCKED 2，其中必须修正 12 条。全部 12 条已在本轮修正：

| ID | 级别 | 修正内容 |
| --- | --- | --- |
| SA-01 | high | **拒绝码顺序原文写成"请求校验→设备状态→readiness"，与源码相反**。已按源码改为 6 级真实顺序：连接/隔离状态（`CAMERA_NOT_READY` 先于字段校验）→ 命令输入 → `LIVE_VIEW_CAPTURE_FORBIDDEN`（**先于**拍摄目录码）→ 拍摄目录 → `PRESET_NOT_FOUND` → readiness 三码，并给出两个具体反例 |
| SA-02 | high | 补写 **Apply 部分失败的后果链**：`failed` + `Camera parameter application partial` → `fault_latched` + 全局 Stop + 相机会话 `cleanup_unconfirmed` 隔离；并在 Apply 命令行加提示 |
| SA-03 | high | 修正实机覆盖归属：**最终候选上实测的只有 0 台发现/Nano/退出 0**；相机身份、Read/Apply、LV、扫描准备等结论来自更早的构建 3，需在最终候选上复核 |
| SA-04 | medium | 快照没有 `captureReady` 字段；改为"`capabilities.disabledReasons` 中 `captureReady` 键存在=未就绪" |
| SA-05 | medium | 补写 `fault_latched` 的清除路径：唯一清除点是**预检成功**；ack 不清除，相机再掉线或清理未确认会重新置位 |
| SA-06 | medium | 补写门禁的边界：`capture_readiness` 只覆盖无设备往返可判断的条件，worker 仍做参数比对/LV_STATUS/`recordingMedia` 新鲜回读，门禁通过不保证拍摄成功 |
| SA-07 | medium | Rust 计数由 318（修复前基线）更正为 **324**（当前修订） |
| SA-08 | medium | 标注 `CAMERA_PARAMETER_SESSION_UNAVAILABLE` 在稳态不可达（仅同请求内竞态），稳态连接问题先返回 `CAMERA_NOT_READY` |
| SA-09 | low | LV 帧形状补 `capturedAtMs` |
| SA-10 | low | `confirm_scan_stage{stage:"geometry"}` 的 `geometry` 实为必填 |
| SA-11 | low | `set_parameters` 在生产是**硬拒绝** `STAGE_1_NANO_ONLY`；`update_scan_setup` 带 `exposureMs` 拒绝 `EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY` |
| SA-12 | low | `software_update_import` 取消时**先清反馈**再返回当前状态 |

**BLOCKED（审计者明确无法判定）**：规范中否定性能力清单（JPEG/视频/Bulb/9205 区域 AF 等"未开放"）无法静态穷尽证明；相机相关实机结论无法在最终修订上复核（D7100 未枚举）。审计者另列 12 条前端功能缺口（G1–G12），含"错误串存在裸串与 `ENGINE:<CODE>` 两种形态"，属新前端实现范围。

## 5f. 第 4 轮实机复核（相机重新连接后，候选构建 7）

> **本节是第 4 轮当时的事实快照，不要当作当前状态**：其中的 "F-01 实机验收仍 BLOCKED" 等结论已被后续轮次取代——F-01 已于第 7 轮修复并在两代候选上实测（见 §5k/§5l），F-02…F-10 亦已修复（§5m–§5x）。当前状态以 §5z 与本文件 §7 为准。

宿主：`85995F09D293D62C44E78178B0D5F1A7B1506DB0DBA41C432DE30CF8BDC345E3`；内嵌引擎 `5F87D2927ECA5ED4AC844B90528DE08048DC8497D8A593D0E61467A885DED9F9`。
证据：`tmp/tests/desktop/20261006-191500-camera-connected-acceptance/`。相机为 D7100（`04B0/0430/8322769`），转台 CH340 COM7。

| 项 | 结论 | 关键实测 |
| --- | --- | --- |
| T2 链路 | **PASS（最终候选）** | 真实 Tauri → EngineClient → JSONL；命令回执、`camera_discover`、`stop` 均正常 |
| T3 身份/读取 | **PASS（最终候选）** | 显式 `deviceId` 连接成功、`camera_read_properties` 成功；`driveMode=32785`（自拍）、`recordingMedia=0`、`iso=1000`、`shutter=1966081`、`aperture=560` |
| **F-02 Apply 分码** | **PASS（实机首次验证）** | `iso=150`→`CAMERA_PROPERTY_UNSUPPORTED_VALUE`；未知键→`CAMERA_PROPERTY_UNDESCRIBED`；只读 `driveMode`→`CAMERA_PROPERTY_UNSUPPORTED_VALUE`；空对象→`INVALID_CAMERA_PARAMETER_COUNT`；缺 `properties`→`INVALID_PARAMETERS`；**6 次拒绝后相机仍 `connected=true`、`phase=idle`、无 fault、无断连**（对比第 1 轮同一路径会 fault + 断连） |
| **F-04 focus 门禁** | **PASS（实机首次验证）** | LV 未开启时 `camera_focus{step:10}`→`CAMERA_FOCUS_UNQUALIFIED`（第 1 轮同请求会 fault + 断连）；`step:0` 仍优先返回 `INVALID_CAMERA_FOCUS_STEP` |
| **F-03 拍摄门禁** | **PASS（实机首次验证）** | `driveMode≠1` 时 `camera_capture`→`CAMERA_CAPTURE_UNQUALIFIED`，**会话保持连接、`phase=idle`、无 fault**（第 1 轮同条件会 fault + 断连） |
| T5 LV | **PASS（最终候选）** | `camera_live_view_start` 生成新 nonce（`8a5ae815…`、`916ead2c…`、`3f9d46c7…`）；LV read 与 `cameraZoom` configure 成功；LV 停止后旧 nonce→`STALE_LIVE_VIEW_SESSION`；LV 未激活取帧→`LV_NOT_ACTIVE` |
| T6 LV 中拒绝 | **PASS（最终候选）** | LV 中 `camera_apply_properties`→`LIVE_VIEW_APPLY_FORBIDDEN`（分码实机验证）；`camera_capture`→`LIVE_VIEW_CAPTURE_FORBIDDEN`；非法 LV 控制→`INVALID_LIVE_VIEW_CONTROLS`；**每次拒绝后 LV 仍 active** |
| **F-01 LV 期间全局 Stop** | **FAIL（仍然可复现）** | 相机进入 `cleanup_unconfirmed`/`isolated`；`camera_connect` 与 `camera_read_properties` 被 `DEVICE_CLEANUP_UNCONFIRMED` 拒绝。**新根因**：诊断显示两次有界尝试各耗尽 2 s 预算，错误均为 `camera timeout: shared operation deadline exceeded`（`elapsedMs=4069`），因此第 3 轮的"取消顺序/重试"方向不足——问题是该 LV 确认事务在 Stop 路径下本身超时 |
| **V-01 操作员恢复入口** | **PASS（实机首次验证）** | 加 `acknowledgeUnconfirmedCleanup:true` 后不再返回 `DEVICE_CLEANUP_UNCONFIRMED`，锁被释放、隔离代理退役、产生新 session `5e4e426c…` |
| **F-07 机身残留 LV** | **FAIL（阻断后续）** | ack 恢复后连接被 `camera already has Live View or unknown Live View status` 拒绝 → 机身仍停在 Live View，软件无路径清除 |
| T7 独立 NEF | **BLOCKED** | 释放模式仍为自拍（`driveMode=32785`），且机身残留 LV 使相机无法重建会话。已完成的准备工作：`set_capture_root` 到隔离根、LV 外显式 Apply 当前 `iso/shutter/aperture` 成功 |

**本轮实机获得的净收益**：F-02/F-03/F-04 三项修复首次获得实机证据（且都表现为"不再 fault、会话保留"）；V-01 恢复入口首次获得实机证据；F-01 的根因从"瞬时失败"修正为"Stop 路径下 LV 确认事务超时"，并明确第 3 轮修法不足。

**待人工动作后才能继续**：在机身退出 Live View（或断电重连）以清除 F-07；如要完成 T7 还需把释放模式拨到单张。

## 5g. F-01 根因定论（第 4 轮实机深挖）

用户提问"为什么每次都卡在这里"，本轮用两次实机复现（构建 7 的 2 s 预算与构建 8 的 15 s 预算）给出定论。

### 观测

| 候选 | LV OFF 确认预算 | 实际耗时 | 结果 |
| --- | --- | --- | --- |
| 构建 7 | 2 s（`wait_limit`）+ 取消后重试 2 s | `elapsedMs=4069` | 两次均 `camera timeout: shared operation deadline exceeded` |
| 构建 8 | 15 s（`max(wait_limit, COMMAND_TIMEOUT)`）+ 重试 2 s | `elapsedMs=17080` | **两次仍双双超时** |

同一台设备上，**手动** `camera_live_view_stop` 在同一会话内约 0.6 s 成功。因此不是"预算不足"，而是**该 LV 确认事务在全局 Stop 路径下不返回**。诊断同时显示 `ioCancelConfirmed=true`、`closeConfirmed=true`：取消与接口关闭都成立，唯一不成立的就是 LV OFF。

### 完整卡死链条（每次可复现）

1. LV 开启时执行全局 `stop` → `cleanup()` 内的 `STOP_LV` + `LV_STATUS` 读回悬挂，2 s 与 15 s 预算都被耗尽 → `lv_stopped=false`。
2. `cleanup_confirmed = cancelled && closed && lv_stopped && !capture_pending` 恒为 false → 相机 `cleanup_unconfirmed`、`isolated`；域层 `record_operation_error` 随即 `fault_latched=true` + `manual_lock="Camera cleanup unconfirmed; session isolated"`。
3. 此后**所有**相机/连接类命令都返回 `DEVICE_CLEANUP_UNCONFIRMED`（本轮实测 `camera_connect`、`camera_read_properties` 均被拒），快照内容不再变化；而 `acknowledgeUnconfirmedCleanup` 当时在 UI 上不可达（V-05）→ **界面表现为"卡死"**。
4. 域层等待相机清理确认的窗口（约 6 s）短于工作线程的确认尝试（17 s），因此 `stop` 早已以 `cleanup_unconfirmed` 收尾；**即使工作线程稍后才完成，也没有任何路径清除已锁存的 `manual_lock`/`fault_latched`**。
5. 悬挂期间关闭应用会造成退出阻塞；本轮在系统里发现两个**已无法用 `Stop-Process` 结束**的旧候选进程（父进程早已退出，创建于 21:10，`Access is denied`），它们持续占用相机设备，使后续会话同样连不上——这解释了"每次"都能复现，而不只是偶发。

### 结论与修法方向（下一轮）

- **必须查清 Stop 路径下 `STOP_LV` 为何不返回**：同一命令在手动路径正常，差异集中在 `request_cancel()`（`invalidate_generation` 递增 generation、置 `live_stop_pending`、`frames.revoke()`）之后的执行环境，以及 Stop 路径与手动路径使用的 `CallContext` 差异。需要在该事务上加计时/阶段诊断（例如记录 STOP_LV 与 LV_STATUS 各自的耗时与返回值）后定位。
- **域层等待与工作线程确认必须一致**：要么让域层等待覆盖工作线程的确认预算，要么让"迟到的成功确认"能够清除 `manual_lock`/`fault_latched`（当前完全没有这条路径）。二者都需实机验证。
- **失败后不得把机身留在 Live View**（F-07）：LV 未确认关闭时应在诊断中明确要求操作员处理，并避免在 UI 上把状态显示为可继续。
- **进程退出必须可靠**：悬挂期间关闭应用不得留下无法结束、继续占用设备的进程。

## 5h. 第 5 轮：按参考实现（digiCamControl）改软件恢复，不再依赖人工干预

用户明确要求：**不能用"请人工按 LV 键/断电"来解决**，必须在软件层面找到控制方法。本轮据此对照 `Related_files/digiCamControl-master` 的已验证 Nikon/WPD 实现，做了四处软件改动（离线 325 测试全绿；实机验证见下）。

### 参考实现给出的三条关键做法

1. `NikonBase.StopLiveView()` = `DeviceReady(); ExecuteWithNoData(0x9202); DeviceReady();` —— **先排空设备再发结束 LV**，且不要求本进程曾开启 LV。
2. `DeviceReady(retrynum)` 对 `MTP_Device_Busy` **和 `0x800700AA`** 都 `sleep(5ms)` 重试（最多 50 次）—— 设备忙是可恢复状态，不是失败。
3. LV 取帧用 `Monitor.TryEnter(Locker, 10)`，与所有控制命令共用同一把锁，取帧永远不阻塞控制命令。

### 本轮据此实现的软件改动

| 改动 | 文件 | 目的 |
| --- | --- | --- |
| `confirm_live_view_off` 按参考顺序改为 `DeviceReady → 0x9202 → LV_STATUS 读回 → DeviceReady`，每步命名进错误文本 | `camera/actor.rs` | 结束 LV 前先排空设备；超时能定位到具体事务 |
| 新增 `force_live_view_off()`，并在 **connect 探测到机身仍在 LV 时自动执行** | `camera/nikon.rs` | 机身残留 LV（上次中断的会话或别的上位机造成）由引擎自己清掉，**不再要求操作员按键/断电**；清不掉才拒绝连接（保持 fail-closed） |
| 新增 `CameraError::DeviceBusy` 并只对**瞬时**设备忙重试（WPD `ERROR_BUSY`/PTP busy），`Busy` 仍表示"已连接/队列满"等业务状态**不重试**；`Discover` 与整个 `Connect` 序列都做有界忙重试 | `camera/wpd.rs`、`actor.rs`、`nikon.rs`、`types.rs` | 设备被上一次操作占住时，上位机仍能自动重新取得相机 |
| 清理**迟到确认**会释放相机清理锁 | `device_controls.rs` | 不再出现"工作线程稍后确认成功、但锁已锁存"的永久卡死 |

### 本轮实机证据（构建 9EADF869 / 538F5A69）

- **全局 Stop 期间 LV 确认仍然失败**：`stop` 6.3 s 返回，相机停在 `cleanup_pending`；+6 s 后仍为 `cleanup_pending` 或已转 `cleanup_unconfirmed`。第 5 轮加的 `DeviceReady` 排空与 15 s 预算都未能让它确认成功。
- **但失败原因现在可读且指向明确**：随后同一引擎内的重连失败为 **`camera busy: Windows WPD: device busy (0x800700AA)`** —— 设备被上一笔未完成的 LV 事务占住（`ERROR_BUSY`）。这与参考实现里专门重试 `0x800700AA` 的处理完全对应。
- 首次实现忙重试时只包了 `transport.open`，实测重连 **283 ms 就快速失败**（说明失败发生在 `Discover` 或连接序列的其他调用点上）→ 已改为 `Discover` + 整个 `Connect` 序列都有界重试。
- 遗留的旧引擎实例也会占用相机：本轮发现一个未关闭的上一轮会话引擎，导致新会话的 launch 被守护逻辑拦下；已按正常退出流程关闭（app 0 / engine 0）。

### 过程事故与恢复（如实记录）

本轮我用 PowerShell `Get-Content -Raw` + `Set-Content -Encoding UTF8` 批量替换 `camera/actor.rs` 时，因 ANSI(936) 读码页往返把文件写坏（cp936 重复映射 + 被吞掉的字节）。恢复过程：以 `tmp/debug/camera/20261006-corruption/actor.corrupted.bak` 为原始损坏证据；用 .NET `UTF8 解码 → cp936 重编码` 反向还原，定位并补回 2 个被吞掉的字符（`理`、`止`）与 **2 个被吞掉的结束引号**；再用 **编译器**（结构）与**改动前构建的引擎二进制字符串表**（文本，`target/embedded-engine/release/ct-engine.exe` mtime 23:31，早于损坏）双向校验：actor.rs 的全部 12 条 CJK 字面量逐字命中，`cargo test --workspace` 恢复为 **325 passed / 0 failed / 7 ignored**。教训：**不要用 PowerShell 文本 cmdlet 改写含中文的源码**；应使用编辑器工具或 Node/.NET 指定 UTF-8 显式读写。

## 5i. F-01 最深根因（第 5 轮末，构建 AB3E0CFE）

在本轮加入「忙重试 + 强制 EndLiveView + 迟到确认解锁」后，实机复现仍然失败，但失败点已经收敛到唯一一处：

| 步骤 | 实测 |
| --- | --- |
| `connect` → `camera_live_view_start` → 全局 `stop` | stop 6.05 s 返回；相机 `cleanup_pending`（清理仍在进行） |
| 等待 8 s 后 | 仍 `cleanup_pending` —— LV 事务**不返回** |
| `ack + connect`（自动恢复尝试） | 428 ms 快速失败：**`camera busy: WPD device already open`** |

**根因**：那次挂住的清理（`Request::Disconnect` → `cleanup()` → LV 确认事务）**始终持有已打开的 WPD 设备**。引擎侧的 `self.camera=None` 只是放掉了引用，工作线程仍在事务里，WPD 设备因此保持 open；新连接于是得到"设备已被打开/占用"这一**业务态 Busy**（我的 `DeviceBusy` 拆分正确地不对它重试——重试也无用，因为占用者没有退出）。等到进程结束，设备才释放。

**结论**：`CallContext` 的 deadline 只是**协作式**的——它在事务开始处与中间检查点检查，**不能中断一个已经卡在 COM 调用里的 WPD 事务**。因此"给清理更多预算"或"再重试一次"都无法解决；必须让低层事务本身可超时并可强制释放：

1. `wpd.rs` 的 `send`/等待路径必须使用**真正的有限等待**（用剩余预算做 `WaitForSingleObject` 一类的有界等待），而不是依赖协作式检查；
2. 超时必须**关闭端口并释放设备**（`Close` + 清 `open` 状态），使同一进程内的后续连接能够重新取得相机；
3. 清理路径在放弃 LV 确认后仍要保证**设备被释放**，即便 LV OFF 未确认（安全语义不变：仍报告 `cleanup_unconfirmed`、仍隔离该会话，但不再占用设备）；
4. 需实机验证：Stop 后设备在有限时间内可被重新打开；`force_live_view_off` 能在新会话中清掉机身残留 LV（本轮该路径尚未真正被执行到，因为连接一直拿不到设备）。

这条修法属于**设备传输层**，必须在实机上验证后才能声称修复；本轮只完成定位与软件侧的其余四项改动。

## 5j. 第 6 轮：软件自恢复已验证；F-01 的停止路径仍不确认（含新的定位线索）

### 已达成：失败停止后无需人工干预即可恢复相机（两次独立验证）

| 观测 | 结果 |
| --- | --- |
| LV 期间全局 `stop` | stop 6.1–6.3 s 返回，相机 `cleanup_unconfirmed`（fail-closed 判定保留） |
| 设备释放 | 清理结束后 `transport.close()` 释放设备；不再长期占用 |
| `ack + connect`（`acknowledgeUnconfirmedCleanup`） | **成功**：新 session `a0b15b69…` / `3f2a7e5d…`，`camera_phase=idle`，`cleanupConfirmed=true` |
| 其后 `camera_read_properties` | **成功** —— 相机在**没有任何人工按键/断电**的情况下被重新取得并可用 |

配套软件机制：`force_live_view_off()`（connect 时探测到机身仍开 LV 就自己发 `DeviceReady → 0x9202 → DeviceReady`，清不掉才拒连）、`CameraError::DeviceBusy` 只对**瞬时**设备忙重试（`Discover` 与整个 `Connect` 序列，避免"设备被上次操作占住就永远连不上"）、清理的迟到确认会释放清理锁。

### 仍未解决：停止路径的 LV 关闭确认

第 6 轮把顺序也试遍了，结论明确：

| 变体 | 首个失败步骤（步骤命名诊断） |
| --- | --- |
| 先 `DeviceReady` 再 `0x9202`（参考实现的顺序），2 s / 15 s 预算 | `DeviceReady before end-Live-View: camera timeout` |
| 先 `0x9202`（与手动路径相同顺序），1.5 s 预算 | `EndLiveView 0x9202: camera timeout` |
| 先 `0x9202`，**15 s 预算** | 仍 `EndLiveView 0x9202: camera timeout`（两次尝试都耗尽） |

而同一会话里**手动** `camera_live_view_stop`（`Request::LvStop`）约 0.6 s 成功。两条路径的差异因此收敛到一个点：**cleanup 永远发生在 `camera.request_cancel()` 之后**（`invalidate_generation` 递增 generation、置 `live_stop_pending`、隔离缓存 health），而手动路径不调用 `request_cancel()`（只置 `live_stop_pending` 并 revoke 帧）。

**下一轮的首选实验**：在 Stop 路径**先停 LV、后取消**——即在 `request_cancel()`/epoch 失效之前，以仍然有效的会话发 `STOP_LV + LV_STATUS` 读回；确认 OFF 后再执行取消与断开。若成立，F-01 即被真正修好，同时保留 fail-closed：未确认 OFF 时仍隔离并按 `cleanup_unconfirmed` 收尾。

### 另一个已复现的 F-01 后果：`cleanup_unconfirmed` 下的关闭挂起

本轮又一次观测到：带 `cleanup_unconfirmed` 锁存退出时，应用在 90 s 内未能确认关闭（`normalCloseConfirmed=false`，engine exit 1），且残留的应用进程需要显式强杀才能释放。这与"必须让清理尽快到达 `transport.close()`"是同一类要求。

## 5k. F-01 已修复（第 7 轮实机验证）

**根因（最终确认）**：Stop 路径在结束 Live View **之前**就调用了 `camera.request_cancel()`（`invalidate_generation` 递增 generation、置 `live_stop_pending`、撤销 LV 控制并隔离缓存 health）。在该状态下，`EndLiveView (0x9202)` 事务**不会返回**——15 s 预算也会被耗尽；而同一条命令在**未被取消**的会话上（手动 `camera_live_view_stop`）约 0.6 s 即确认。第 5/6 轮试过的"先 DeviceReady 排空"、"缩短/加长预算"都不是变量，真正的变量是**取消与停 LV 的先后**。

**修复**：`enqueue_stop` 在 `request_cancel()` 之前，若相机仍在 Live View 且 worker 不忙，先用**有效会话**执行一次有界（3 s）的 `stop_live_view_with_timeout()`；随后照旧取消、清理。失败仍按原 fail-closed 路径处理（未确认 OFF 就隔离并报 `cleanup_unconfirmed`），不新增任何放宽。

**实机对照（同一台 D7100）**

| | 修复前 | 修复后（构建 `B976E2B3`） |
| --- | --- | --- |
| LV 期间全局 `stop` | 6.1–6.3 s，相机 `cleanup_pending` → `cleanup_unconfirmed` | **1.08 s**，相机 `disconnected` |
| `cleanupConfirmed` | false | **true** |
| `enginePhase` | `fault` | **`stopped`** |
| `manual_lock` / `fault_latched` | 锁存 | 无 |
| 之后使用相机 | 需人工退出机身 LV 或 ack 恢复 | **直接 `camera_connect` 成功（新 session `c8a1d970…`）+ `camera_read_properties` 成功** |

即：**不再需要操作员按键、断电、重启或任何恢复步骤**；同时第 6 轮加入的软件恢复手段（`force_live_view_off`、`DeviceBusy` 忙重试、迟到确认解锁）作为二线保护继续保留。

**回归保护**：新增 `stop_during_live_view_ends_live_view_before_cancelling`，断言停止恰好发一次 EndLiveView、LV 已结束、`cleanup_confirmed=true`、无 fault/manual_lock，且随后 `camera_connect` 直接可用；离线总计 **326 passed / 0 failed / 7 ignored**。

**第二项后果同时消失**：F-01 修复后，同一会话的**正常退出**恢复为 **app 0 / engine 0、无强制结束**（`tmp/tests/desktop/20261007-001800-f01-final-verify/exit.json`），此前带 `cleanup_unconfirmed` 的退出会在 90 s 内无法确认（`normalCloseConfirmed=false`、engine exit 1）并残留悬挂进程。

## 5l. 第 8 轮：F-01 修复后在最终候选上的完整实机验收

宿主 `5CD13EB7F5D529A00E5D1B7EEC1406B30C539EE46C5A8D5BC47278E59B66646E`，内嵌引擎 `5B0C5A813A4F762180582AF34259D880564286AE2B300541F2AE25179A9B1142`。证据：`tmp/tests/desktop/20261007-003500-final-acceptance/`（含 `steps.jsonl`、`commands.jsonl`、`snapshots.jsonl`、`exit.json`、`process.json`）。

| 项 | 结果 | 关键实测（本轮真实证据） |
| --- | --- | --- |
| T2 链路 | **PASS** | 真实 Tauri → EngineClient → JSONL；命令回执与 `camera_discover` 正常 |
| T3 身份/读取 | **PASS** | 显式 `deviceId` 连接 594 ms；`camera_read_properties` 306 ms；**34 项**摄影参数；session `3024a881…`；`driveMode=32785`（自拍）、`recordingMedia=0`、`iso=1000`、`shutter=1966081`（30 s） |
| F-02 分码 | **PASS** | 未知键 → `CAMERA_PROPERTY_UNDESCRIBED`；空对象 → `INVALID_CAMERA_PARAMETER_COUNT`；**拒绝后相机仍连接、无 fault** |
| F-04 门禁 | **PASS** | LV 未开启 `camera_focus{step:10}` → `CAMERA_FOCUS_UNQUALIFIED`，会话保留 |
| 拒绝码优先级 | **PASS（实机）** | 未设置捕获根时 `camera_capture` → `CAMERA_CAPTURE_ROOT_REQUIRED`（**请求校验先于 readiness**，与规范一致） |
| T5 LV | **PASS** | `camera_live_view_start` 300 ms → nonce `6dea0725…`；`camera_live_view_stop` 854 ms；全程会话保持连接 |
| T7 独立 NEF | **BLOCKED** | 释放模式仍为自拍（`driveMode=32785`，descriptor 报 `nativeWritable=false`）。**参考实现 `digiCamControl` 的 Nikon 适配也没有任何 driveMode 写入**——该档位是机身拨盘，主机不可写；`captureReady` 原因文本已明确指向它 |
| T8 Nano/退出 | **PASS** | `RTS9060_NANO` 连接与 `turntable_status` 成功；`stop` → `phase=stopped`；**正常退出 app 0 / engine 0、无强制结束**（F-01 修复后退出不再挂起，本轮第二次确认） |
| T9 扫描准备 | **PASS** | `set_capture_root` → `prepare_scan`（`taskId=r8-accept`、`exposureMs=30000`、几何 measured/counterclockwise）成功提交；**未创建任何任务目录**（run 目录下无子目录）；`preflight` → `XRAY_NOT_CONNECTED`；`start_scan` → `PREFLIGHT_REQUIRED`；`abandon_scan` → `NO_ACTIVE_TASK`；`taskLock` 保持 unlocked |
| 射线 / HOME / MOVE / 真实扫描 | **BLOCKED** | 无连接与出束授权；`multipleExposureEvidence=UNKNOWN` 按设计阻扫描 |

**结论**：F-01 修复未回归任何已验证行为；最终候选的主要相机/Nano/扫描准备路径均在本轮重新取得实机证据（不再依赖更早构建的结论）。仍未完成的只有 T7（机身拨盘）与受授权/设计门禁阻断的射线类项目。

## 5m. 第 8 轮：F-01 修复的独立验证与整改

独立验证者（`final-verifier`，只读、对抗式）复核了 F-01 修复，结论 **A1–A5 全部 PASS**（含用 5 个 Lead 未列出的运行交叉印证修复前后对照：修复前 4 次 `cleanup_unconfirmed`、其中 `231000-stopfirst-verify` 的 `exit.json` 为 FAIL/`normalCloseConfirmed=false`/engine exit 1；修复后 1041–1077 ms、`cleanupConfirmed=true`、重连无需 ack、正常退出 0）。它同时指出 4 处问题，本轮**逐条整改并复验**：

| 验证发现 | 处置 |
| --- | --- |
| **B（FAIL）回归测试不具判别力**：脚本传输 `STOP_LV` 恒成功、`cancel/close` 恒 Ok，删掉"先停 LV"后测试仍通过 | **已修并已证明判别力**：改为断言"stop 命令**同步**结束 Live View"（`before the safety job runs` 时 `live_view_stop_count()==1` 且 `live_view.active==false`）。把预停临时关闭后该断言**实测失败**（`device_controls.rs:914`），恢复后通过；同时保留 `lv_stop_at_cancels==Some(0)` 的顺序护栏（需 `reset_live_view_stop_tracking()` 先清零，避免测试自身 setup 的 cancel 干扰） |
| **C1（medium）**：`!health.busy` 会让"忙碌中的 Stop"整段跳过预停、退回已被实机证明失败的旧顺序 | **已修**：去掉 `busy` 判断，只保留 `connected && live_view.active`；预停仍有 3 s 界，超时即继续原 fail-closed 清理，安全 Stop 最多被推迟该预算 |
| **C6（medium）**：cleanup 的 LV 预算 15 s 超出安全 Stop 的 6 s 窗口，确认落在 6–15.75 s 仍会置 fault+锁 | **已修**：cleanup 的 LV 确认预算收紧为 `min(wait_limit, 2 s)` + 取消后重试 750 ms，整段落在 6 s 窗口内；未确认即快速失败并让 `transport.close()` 释放设备（迟到确认仍由 `reconcile_camera_cleanup_lock` 解锁） |
| **D-4（low）** 注释称"mirrors"参考顺序但已故意去掉前置 DeviceReady；`LV_STATUS!=0` 分支无步骤名 | **已修**：注释改写为明确差异与理由；该分支改为带步骤名的错误 |
| **D-6（low）** 干净 Stop 后相机 health 仍带 `camera operation cancelled; cleanup evidence required` 而顶层 `lastError=null` | **已修**：确认清理后清除该取消占位串（保留真实早前设备错误）。实机复验：stop 后 `lastError=null` |
| D-1/D-2/D-3/D-5/D-7/D-8/D-9/D-10 | D-1/D-2 已随上表关闭；D-5 已在本文档按实测数字（947–1077 ms、854 ms 普通 LV stop）更正，不再写"~0.6 s"；D-7/D-9/D-10 属证据范围说明，已如实保留 |

**整改后实机复验（构建 `4790180B`）**：LV 期间全局 Stop **947 ms**、`phase=stopped`、`cleanupConfirmed=true`、**`lastError=null`**、无 fault/锁；立即 `camera_connect` 成功（新 session `072fee20…`）+ `camera_read_properties` 成功；正常退出 **app 0 / engine 0**。离线：**326 passed / 0 failed / 7 ignored**、Node 165、typecheck、module-map 全绿。

**仍保留的残余（下一轮候选，非阻塞）**：验证者指出参考实现里本引擎仍缺 4 点，其中最关键的是**帧读让步**——`GetLiveViewImage` 用 `Monitor.TryEnter(Locker, 10)`，而本引擎帧循环内联 `GET_LV`（2 s deadline）只在帧边界检查 `live_stop_pending`，因此 `LvStop` 可能要排在在飞帧读之后（这正是预停需要 3 s 预算、极端情况下仍可能超时的形态）。其余三点：参考可在其它操作内重入 StopLiveView；参考把 `MTP_Not_LiveView` 当"LV 已停"的正常结果而本引擎对任何 `GET_LV` 错误都 bump generation + cleanup；参考 `DeviceReady` 对 busy 最多 50×5 ms。

## 5n. F-08（第 9 轮新发现并修复）：重复 `camera_connect` 会锁故障并丢掉健康会话

**发现过程（实机，构建 `4790180B`）**：在一次 T5 探测中我误对**已连接**的相机再发了一次带 `deviceId` 的 `camera_connect`。原始证据（`tmp/tests/desktop/20261007-033000-t5c-probe/steps.jsonl`、引擎日志）：

| 观测 | 值 |
| --- | --- |
| 引擎日志 | `ERR: camera busy: disconnect the active camera before selecting another` |
| 引擎 phase | `idle` → **`fault`** |
| 相机会话 | `connected:false`、`isolated:true`、`cleanupConfirmed:true`、`lastError` = 上述文本 |
| 影响 | 原本健康的会话（Live View 刚跑完 129 帧）被隔离并断开；此后所有相机命令都 `CAMERA_NOT_READY`，直到重连 |

也就是说：**操作员多按一次"连接"（或前端重发）会让引擎锁故障并丢失会话**，属于"必须人工干预才能恢复"的同一类问题。

**修复**：在 `enqueue_camera_connect` 中，**在锁/清理判定之后、任何入队与设备操作之前**，若带非空 `deviceId` 且当前会话 `connected && !isolated`，直接返回 **`CAMERA_ALREADY_CONNECTED`**。顺序刻意如此：维护锁/busy、`DEVICE_CLEANUP_UNCONFIRMED` 与 `acknowledgeUnconfirmedCleanup` 恢复路径**优先级更高**（恢复入口要求 isolated，天然不受影响）；无 deviceId 的发现刷新在有会话时仍然允许。

**回归保护**：新增 `repeated_camera_connect_is_rejected_without_faulting_or_dropping_the_session`（断言返回码、未入队任何操作、会话仍连接且未隔离、`fault_latched=false`、无 `manual_lock`，并确认发现刷新仍可用）。该改动暴露并修正了 `discovery_only_camera_connect_reports_a_discovery_receipt` 的一处不真实 setup（脚本相机默认已连接，而真实发现不会开会话），现已在测试开头显式 `disconnect_confirmed()`。离线总计 **327 passed / 0 failed / 7 ignored**。

## 5o. F-09（第 9 轮新发现并修复）：LV 控制事务阻塞时可复现 F-01 的失败模式

**发现过程（实机，构建 `1E9A332E`）**：在一次 Live View 探测中，我连续发送 `camera_live_view_read` 后**未等待其完成**就发全局 `stop`。原始证据（`tmp/tests/desktop/20261007-052000-lv-valid/`）：

| 观测 | 值 |
| --- | --- |
| 引擎操作表 | `camera_live_view_read` 状态仍为 **`queued`**（已入队、worker 尚未处理完） |
| `stop` | **6226 ms** → `phase=fault`，相机 `cleanup_unconfirmed`，诊断 `EndLiveView 0x9202: camera timeout`（两次尝试都超时） |

**根因**：`camera_live_view_read/configure` 使用 `COMMAND_TIMEOUT`（15 s）作为整笔事务预算，而它们与 Stop 的"先停 LV"共用**同一个单线程相机 worker**。一个卡住的 LV 控制事务会把队列里的 `LvStop` 堵住超过 Stop 的 3 s 抢占预算 → 预停失败 → 随后 `request_cancel()` 让 `EndLiveView` 永不返回 → 与 F-01 完全相同的失败模式。这也与独立验证者 §5m 指出的参考实现缺口（本引擎帧读/控制事务没有为控制命令让步）是同一条。

**修复**：新增 `LV_CONTROL_TIMEOUT = 2 s` 用于 `read_live_view_controls` 与 `configure_live_view`（整笔 LV 控制事务的最长占用），并把 Stop 的预停预算提到 **5 s**，使"一个被限界的在飞 LV 事务 + 一次 end-LV"必然落在预算内。设备级 fail-closed 语义不变：超时仍按未确认处理。

**实机复验（构建 `A4592051`，`tmp/tests/desktop/20261007-063000-f09-verify/`）**——用**完全相同**的触发序列（LV 活动 + 紧跟一个 `camera_live_view_read` 后立即 Stop）：

| | 修复前 | 修复后 |
| --- | --- | --- |
| `stop` | 6226 ms → `fault` / `cleanup_unconfirmed` | **815 ms** → `stopped`，`cleanupConfirmed=true`，`lastError=null` |
| 相机 | 隔离，需 ack 恢复 | 干净断开；`camera_connect` **直接成功**（新 session `4b5f5824…`） |

**回归保护**：既有 `stop_during_live_view_ends_live_view_before_cancelling` 覆盖"同步先停 LV"；LV 控制事务的预算由常量注释与本文档固化（无计时型单测，避免 flaky），实机复现序列保存在上述证据目录。离线：**327 passed / 0 failed / 7 ignored**（xray `owner_monitors_interlock…` 为既有 flaky：单独运行通过、全量复跑通过）。

## 5p. 第 10 轮：LV 控制事务耗时实测、F-10（设备拒绝请求值不再毁会话）与规范整改

**本轮最终候选**：host `6377CF10CE5D93D1C46173A92C19C8F33F4FB785FBFAC01140F8D3FA71E4D57F`，内嵌引擎 `70A4343EBA5CAE2A41FE5F0105D8160BDFFD88CBAE2AFB9272870992E4236C1E`（第 9 轮 F-08 门 + 第 10 轮 F-10/空白 id 修复均已进入）。离线门禁：Rust **328 passed / 0 failed / 7 ignored**、Node 165、typecheck、module-map 全绿；正式入口未改（`C19B828D…`）。

### 实测耗时（构建 `A4592051`，`tmp/tests/desktop/20261007-074000-lv-control-timing/`）

| 事务 | 引擎时间戳 | 说明 |
| --- | --- | --- |
| `camera_live_view_start` | **192 ms** | — |
| `camera_live_view_read` | **66 ms**（两次一致） | 与 `LV_CONTROL_TIMEOUT = 2 s` 相比余量约 **30×** |
| `camera_live_view_configure {cameraZoom:1}` | 111 ms 后失败 | 设备返回 `PTP 0x201c` |

结论：F-09 的 2 s 上限不会误杀正常 LV 控制事务；真正的问题在**失败后的处置**。

### F-10：设备拒绝请求值却被当成设备故障（已修 + 实机验证）

**发现**：`camera_live_view_configure {cameraZoom:1}`（descriptor 报 0..7 可写）在实机返回 `PTP 0x201c`。该错误进入 `device_controls.rs:511` 的 `return Err("Live View control application incomplete: …")`，于是 `record_operation_error`（`:177-187`）把它当成设备故障：**`fault_latched=true` + 全局 Stop + 相机会话隔离**，随后所有相机命令 `CAMERA_NOT_READY`。`camera_apply_properties` 的部分失败（`:514`）走的是同一条路径 —— 即 §5d 记录的 F-05 与本轮 F-10 是同一个缺陷，只是分别在参数与 LV 控制两条入口上。这同时与 `AGENTS.md` 的规定相矛盾：「部分失败返回实际或未知状态并失效预检，不伪造回滚」。

**修复**：新增 `device_value_rejection_error()` 区分「设备拒绝请求值」（两种 partial-apply 文本前缀，且显式排除 `camera unavailable:`/`camera I/O failed:`/`camera timeout:`/`camera session isolated:`/`camera operation cancelled` 等真实传输失败）。命中时：操作 `status` = **`rejected`**（新状态，前端 `OperationStatus` 已同步）、`last_error` 与聚合日志给出逐键原因、`preflight=false`（fail-closed，扫描仍被阻断），但**不置 `fault_latched`、不 Stop、不隔离会话**。真实传输失败与清理未确认的处置**完全不变**。

**实机验证（构建 `75EDCCCF`，`tmp/tests/desktop/20261007-084000-f10-verify/`）**：

| | 修复前（`A4592051`） | 修复后（`75EDCCCF`） |
| --- | --- | --- |
| `camera_live_view_configure {cameraZoom:1}` | 失败 → **`phase=fault`**，会话隔离/断开 | 操作 **`status="rejected"`、`stage="rejected"`**，错误文本保留 `PTP 0x201c` |
| 引擎 | `fault_latched=true` + 全局 Stop | **`phase=idle`**、无 fault、无锁 |
| 相机会话 | 断开，后续命令 `CAMERA_NOT_READY` | **保持连接**（同一 session），LV 仍在运行（新 nonce） |
| 后续 | 需断开/ack 重连 | `camera_live_view_read` 仍可用；`camera_live_view_stop` **327 ms** 干净停止 |

**回归保护**：新增 `device_value_rejection_does_not_latch_a_fault_but_a_transport_failure_does`（先断言分类器：两种 partial 文本 ⇒ 拒绝；带 `camera unavailable`/I/O 的 partial 文本、清理未确认 ⇒ 仍故障；再对三条错误逐一跑引擎路径，断言 `fault_latched`、是否入队 Stop、`preflight`、操作 `status`）。离线总计 **328 passed / 0 failed / 7 ignored**。

### 规范整改（按独立 `spec-auditor` 的 10 条必改清单）

`tmp/tests/desktop/20261007-072000-spec-conformance/`（typecheck 0、module-map 0、node 165/165）指出 spec 存在 3 处 STALE 与 1 处 OVER-CLAIMED。本轮已改：离线计数 324→328、轮次与实机归属更新为 1–10 轮并列出两代候选哈希与已复核项；§5 补 F-09（2 s 上限、实测余量、前端不得并发 LV 控制/不得在控制未完成时 Stop）、补 F-05/F-10 的 `rejected` 语义；拒绝顺序补第 0 层锁码与 `CAMERA_ALREADY_CONNECTED`；`force_live_view_off` 由「不需要操作员按键」软化为「先尝试自动清除，清不掉仍拒连并要求检查机身，该成功路径尚无实机执行证据」；`disabledReasons` 补全键集；`camera_live_view_stop` 补 6 s 清理路径；T7 补阻塞原因（`driveMode=32785` 机身拨盘、`nativeWritable=false`）；`deviceId` 补「trim 后非空」。

### 独立验证（task-10）与 D-1 整改

独立验证者复核第 9 轮改动：F-08 (a)(b)(c)、F-09 (a)(b)、测试判别力、`ever_camera`/`preflight` 一致性均 **PASS**（含用实机证据交叉核对；并确认 F-01 的回归测试「终于具备真判别力」，因为断言落在 `pump` 之前）。它同时给出两条真实残留：

| 发现 | 处置 |
| --- | --- |
| **D-1（medium）**：`deviceId:""`/纯空白**绕过** F-08 门——门用 `trim()` 判据，而 kind/worker 用 `is_some()`，于是空串仍走显式 connect 并复现 fault+隔离 | **已修**：`enqueue_camera_connect` 只做一次判据——`trim()` 后为空的 deviceId **一律视为未提供**（发现刷新），门、ack 守卫与 kind 决策共用同一 `selected`。实机复验（`6377CF10`）：`deviceId:"   "` → `camera_discover:completed`、`phase=idle`、会话保持连接、`isolated=false`，随后 LV start/stop 正常（319/858 ms） |
| **D-3（medium）**：LV 控制超时会**破坏性退役会话**（`call()` 到点即 `invalidate_epoch`），不是"先重试一次" | 记录为下一轮候选：把 LV 控制超时先按可重试处理再退役；本轮已用 2 s 上限 + 实测 66 ms 余量把触发概率压到很低 |

## 5q. 第 11 轮：`rejected` 前端契约修复、值拒绝分类收紧、最终修订实机复核

**本轮最终候选**：host `3127477DD24F829B477E3157A7D81F15AA2F216C65F3E6A9C187AD36EF85BE68`，内嵌引擎 `6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7`。

### 独立验证（task-10）发现的 HIGH 与处置

验证者指出第 10 轮新增的 `Operation.status="rejected"` **破坏前端契约**：`src/engine/tauriAdapter.ts` 的快照校验白名单不含它，而 `assertSnapshot` 作用于**每个**快照且 operations 历史会保留 → 生产 UI 在一次值拒绝后会**持续**抛 `Engine protocol error`；`src/camera-draft.ts` 的终态集合也不含它 → `camera_apply_properties` 的等待器会一直轮询到 110 s 超时；且 `tests/*.mjs` 没有任何 `"rejected"`，165 项全绿也发现不了。

**修复**：`tauriAdapter.ts` 白名单、`camera-draft.ts` 的 `TERMINAL`、规范 §2 的状态集合三处同步加入 `rejected`；并在 `tests/native-device-controls.test.mjs` 增加判别断言——构造一个含 `status:"rejected"` 的 operations 快照必须能通过 `assertSnapshot`，且 `camera-draft.ts` 的终态集合必须包含 `"rejected"`（该断言在修复前会失败）。

### 值拒绝分类收紧（fail-closed 方向）

验证者同时指出原实现只匹配文本前缀，会把 `camera busy:`、`camera capability unavailable:` 及**任意** PTP 码都当成"可纠正拒绝"（相对第 9 轮是 fail-open）。现已收紧：仅当 partial-apply 文本**且包含 `PTP response 0x201c`**（唯一有实机证据的设备拒绝值码，`InvalidDevicePropValue`）且不含传输失败标记时才判为值拒绝；其它 PTP 码（如 `0x2019`）、`camera busy:`、`camera capability unavailable:`、`camera safety check failed:` 一律仍按设备故障处理。单元测试逐条覆盖这两类。

### 最终修订实机复核（回应验证者 D-8/D-9：此前无实机覆盖最终修订）

| 运行 | 覆盖 | 结果 |
| --- | --- | --- |
| `20261007-110000-round11-final` | T2 链路、连接、**重复 connect**、**空白 deviceId**、T3 读取、T4 apply/还原、T5 LV start/stop、T8 退出 | 重复 connect → `CAMERA_ALREADY_CONNECTED`（14 ms，无 fault）；空白 id → `camera_discover:completed`（330 ms）；`iso 1000→200→1000` 且终态 `iso=1000`；LV start 313 ms / stop 901 ms；退出 **app 0 / engine 0** |
| `20261007-113000-rejected-final` | **值拒绝路径 + 拒绝后快照仍可服务** | `camera_live_view_configure:rejected`（硬件 `PTP 0x201c`），`phase=idle`、会话保持连接、**拒绝后连续两次快照正常返回**（`lv` nonce 更新）、`camera_live_view_stop:completed` 591 ms |

离线：Rust **328 passed / 0 failed / 7 ignored**、Node **165/0**（含新增 `rejected` 契约断言）、typecheck、module-map 全绿；正式入口未改（`C19B828D…`）。

**仍保留的验证者残留**：D-3（LV 控制超时会破坏性退役会话，建议先重试再退役）、已证实的 TOCTOU（"首次 connect 尚未发布 connected"时并发双 connect 仍可能 fault，UI 靠 pending 缓解）、非 LV 在飞请求可撑爆 5 s 预停。

## 5r. 第 12 轮：把 task-10 的两条残留用实机证据收敛（无新增缺陷）

本轮**未新增代码改动**：把独立验证者（task-10）列出的两条残留做了针对性实测，一条判定为不存在、一条量化到可接受边界。候选保持 host `3127477D…`／引擎 `6C2E587F…`。

### 残留 (e)「并发双 connect 的 TOCTOU」——实机判定：**不会 fault**

实验（`tmp/tests/desktop/20261007-120000-toctou/`）：用 `engine_command` **背靠背**发两次带同一 `deviceId` 的 `camera_connect`（第一次仅入队即返回，不等待完成）：

| 步骤 | 观测 |
| --- | --- |
| connect #1 | 接受（operation `connect` 状态 `queued`） |
| connect #2（立即发出） | **`ENGINE:DEVICE_CONTROL_LOCKED`**（6 ms）——被既有锁/并发保护拒绝，**不是** fault |
| 2.5 s 后 | `camera.connected=true`（session `4ab441b4…`）、`phase=idle`、无 fault、无 Stop |
| 冷静后再 connect | **`CAMERA_ALREADY_CONNECTED`**（23 ms） |
| 退出 | app 0 / engine 0 |

说明：`operations.spawn` 对同设备并发操作返回 `DEVICE_OPERATION_ACTIVE`（`operations.rs:32`），且设备操作进行中 `blocks_controls()` 为真 → 第二次 connect 在锁层被拒。验证者标为 low-medium 的这条残留**在本修订上不可复现**；仍保留 UI 层 pending 提示的建议。

### 残留「非 LV 在飞请求可撑爆 5 s 预停」——实机量化

实验（`tmp/tests/desktop/20261007-123000-lv-concurrency/`）：**LV 活动期间**再发 `camera_read_properties`（该命令**被接受并入队**，与 connect 不同，它不触发连接锁）：

| 步骤 | 观测 |
| --- | --- |
| LV 活动 + `camera_read_properties` | 操作入队并完成（`camera_read_properties:completed`） |
| 立即 `stop` | **663 ms** → `phase=stopped`、`cleanupConfirmed=true`，无 fault/锁 |

结论：排队在 LvStop 前面的非 LV 相机操作**不会**破坏 Stop 预停；其自身预算与实测耗时（`camera_read_properties` 约 0.6 s、LV 控制事务 2 s 硬上限、帧读 2 s）之和仍在 5 s 预停预算内。理论边界：若设备在某个 15 s 预算的只读事务上停顿超过预停预算，仍会按 fail-closed 走 `cleanup_unconfirmed` 并隔离——这是设计内的失败方向，保留为已知边界。

### 其它残留处置

| 项 | 处置 |
| --- | --- |
| D-3（LV 控制超时会破坏性退役会话） | **保留为设计内 fail-closed**：`call()` 在截止时 `invalidate_epoch`（`actor.rs:212-217`）是"迟到结果不得提交/解锁"的基石，不能为可重试而放宽；实测正常 LV 控制事务仅 66 ms、硬上限 2 s（约 30× 余量），触发概率极低。若将来实机观察到误触发，再考虑"同 epoch 内一次重试"的窄化实现 |
| T7（独立 NEF） | 仍 **BLOCKED**：本轮再次实读 `driveMode=32785`（自拍）且 `nativeWritable=false`；需要把机身拨盘拨到单张 |
| 参考实现缺口（帧读让步 `Monitor.TryEnter(Locker,10)`、可重入停止、`MTP_Not_LiveView` 视为正常、`DeviceReady` busy 重试） | 记录在案；本轮实测表明现有预算组合已覆盖观察到的硬件行为 |

## 5s. 第 13 轮：补齐 T6 / T9 在最新候选上的实机证据（含任务清单第八节更正）

第 12 轮后重读 `docs/backend-next-agent-task-list-2026-10-06.md`，发现两处**验收证据缺口**（此前只在更早构建或仅离线覆盖）：T6 要求"LV 中的普通 Apply（含 empty/no-op）、拍摄、非法步长正确拒绝"，T9 要求"恢复/放弃/重构流程对照"。本轮在同一会话（候选 host `3127477D…`／引擎 `6C2E587F…`，`tmp/tests/desktop/20261007-132000-t6-t9-gaps/`）逐条实测：

### T6：LV 活动期间的拒绝路径（全部不中断 LV、不锁故障）

| 请求 | 实测结果 | 耗时 |
| --- | --- | --- |
| `camera_apply_properties {iso:200}` | **`LIVE_VIEW_APPLY_FORBIDDEN`** | 18 ms |
| `camera_apply_properties {}`（empty） | **`LIVE_VIEW_APPLY_FORBIDDEN`**（LV 禁令先于空对象计数码） | 17 ms |
| `camera_capture` | **`LIVE_VIEW_CAPTURE_FORBIDDEN`** | 14 ms |
| `camera_focus {step:0}` | **`INVALID_CAMERA_FOCUS_STEP`** | 14 ms |
| `camera_autofocus` | **`CAMERA_AUTOFOCUS_UNQUALIFIED`** | 16 ms |
| 以上全部之后 | `phase=idle`、**LV 仍活动且 nonce 不变**（`66bd3c62…`）、无 fault | — |
| `camera_live_view_stop` | 成功（617 ms），会话保持连接 | — |

即：无效请求**不会**自动停 LV、不会制造成功、不会绕过占用锁（第 12 轮另已实测：设备操作进行中第二次 connect 被锁层拒绝）。

### T9：恢复/重构/放弃的拒绝路径

| 请求 | 实测结果 | 耗时 |
| --- | --- | --- |
| `restore_previous` | `PREFLIGHT_REQUIRED` | 24 ms |
| `start_reconstruction` | `INVALID_RECONSTRUCTION_METHOD` | 13 ms |
| `abandon_scan` | `NO_ACTIVE_TASK` | 16 ms |

三者均无 fault、相机会话保持连接；退出 **app 0 / engine 0**。

### 文档更正

`docs/backend-next-agent-task-list-2026-10-06.md` 第八节仍写着首轮交接时的过期事实（324 测试、候选 `AE1D5192…`、未完成项含已修的 F-01/F-05）。本轮已重写该节：当前候选哈希（host `3127477D…`／引擎 `6C2E587F…`）、门禁 **328/0/7**、T0–T11 逐项状态表、F-01…F-10 修复摘要，以及仍需人工决定或 BLOCKED 的具体条件（T7 拨盘、射线/HOME/扫描授权与 `multipleExposureEvidence=UNKNOWN`、D-3 记录项）。

第 13 轮末的独立规范终审（`task-12`，`tmp/tests/desktop/20261007-130000-spec-final-audit/`；typecheck 0 / module-map 0 / Node 165）确认 **task-11 的 10 条必改已全部落实**、10 项源码级主张全部 PASS（`rejected` 全链、F-01 5 s 与无 busy 门、`force_live_view_off` 措辞、F-09 2 s、拒绝顺序、哈希与 328、任务清单第八节），并提出 5 条文档准确性必改，本轮**已全部修完**：

| 审计项 | 处置 |
| --- | --- |
| MF-01（high）规范 §1「1–9 轮」与候选轮次归属错 | 改为 **1–13 轮**，四个候选按轮次分列（第 8/10 验证/10 最终/11–13），并补第 13 轮的 T6/T9 复核项 |
| MF-02（medium）129 帧统计被并入多个候选 | 明确该统计**只来自第 9 轮探针 host `4790180B`**（并列第 1 轮 127 帧、第 8/13 轮仅 start/stop） |
| MF-03（medium）`disabledReasons` 键集不全 | 补 `capture`、`liveView` 与动态镜头键，并说明键集是 capabilities 布尔的子集/变体 |
| MF-04（low）未点名唯一计入 `rejected` 的 PTP 码 | 明确**只有 `PTP response 0x201c`** 且须满足两个 partial 前缀之一、且不含 8 类传输标记 |
| MF-05（high）本文档 §7 与 §5d 仍是第 2/3 轮过期文本 | §7 已按当前证据重写（列出已完成实机项、F-07 改进、T7 与射线/HOME/扫描/重构/维护锁 BLOCKED、D-3 记录项）；§5d 加"后续处置"指针说明 F-05 已按方案 A 落地 |

## 5t. 第 14 轮：T4 正向路径与 readiness 门禁联动实测（含 T7 阻塞的当前证据）

候选不变（host `3127477D…`／引擎 `6C2E587F…`）。本轮做了一次此前缺失的 **T4 正向验证**：不是只验证"拒绝"，而是验证"机身支持的 readiness 参数能显式 Apply、能读回确认、并让门禁随实际状态变化"，随后恢复原值并核对。

会话 `tmp/tests/desktop/20261007-150000-readiness-apply/`（12 步全部 `ok:true`，无 fault，会话 `8f89bf19…` 稳定）：

| 阶段 | `cameraScanReadiness.issues` |
| --- | --- |
| 基线（本次鲜读） | `multipleExposure=null`、`driveMode=32785`、`longExposureNoiseReduction=1`、`recordingMedia=0` |
| Apply `recordingMedia=1` + `longExposureNoiseReduction=0` 后 | **只剩 `multipleExposure=null`、`driveMode=32785`** |
| 恢复 `longExposureNoiseReduction=1` + `recordingMedia=0` 后 | 回到基线四项（逐项一致） |

结论：
1. **T4 正向路径成立**：机身支持的值可以显式 Apply 并读回确认（每次 Apply/Read 约 0.56–0.61 s），且 readiness 是**由当前读回派生**而非缓存——两个可写阻塞项被清除，恢复后又出现。
2. **T7 的剩余阻塞恰好两项**：唯一剩下的可写项是机身释放模式（`driveMode=32785` 自拍、descriptor `nativeWritable=false`，须拨盘）；另一项 `multipleExposure` 是**设计内**的 UNKNOWN（无机器可读的 OFF 证据，需人工覆盖策略）。也就是说：**拨盘拨到单张后，相机侧就再无软件可消除的 readiness 阻塞**（多重曝光仍按门禁阻止真实扫描）。
3. 本轮再次实读确认 `driveMode=32785`、`captureReady` 原因文本为 `camera safety check failed: scan requires driveMode=1`（`tmp/tests/desktop/20261007-144000-drivemode-check/`）。

## 5u. 第 14 轮：接口库存重建 + 一处真实 DTO 缺陷（`taskLock.retained`）修复

**本轮最终候选**：host `F17F2F414ABEAEF9C39750E23E9CA3771CC0E97301DDDE9C91D64CD29B92CD7C`（前端 DTO 修复后重建），内嵌引擎不变 `6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7`（Rust 未改）。

### 发现并修复的 DTO 缺陷（HIGH）

独立接口库存（`task-13`，`tmp/tests/desktop/20261007-140000-interface-inventory-v2/`）指出：`deviceControls.taskLock.retained` 在**生产引擎**里是 `TaskRecord[]`（`task_registry.rs:27,134`；实机快照为 `"retained":[]`），而 `src/engine/types.ts:338` 声明为 **`boolean`**，浏览器预览也写布尔（`rts9060/deviceControls.ts:79`）。因为 `App.tsx:550` 只校验键存在、没有消费者读它的值，`typecheck` 与 165 个测试**都发现不了**；而空数组在 JS 中是真值，任何"`if (taskLock.retained)`"都会误判，属静默的 UI 陷阱。

**修复**：新增 `RetainedTaskRecord` 类型并把 `retained` 改为 `RetainedTaskRecord[]`（`parameters` 按引擎 `Parameters` 的 5 个字段）；预览适配器改为发出同形状数组（未完成时 1 条记录，否则空数组）；新增两条**判别性**测试——`preview-domain-parity` 断言"真实引擎快照与预览快照的 `retained` 都是数组、且 `taskLock` 键集恰为 6 个"，`workstation-ui-operation` 把 `rejected` 加入终态拒绝断言。

**实机复核**（`tmp/tests/desktop/20261007-164000-dto-fix-smoke/`，新 host）：连接 573 ms、读回 581 ms，**实时快照 `taskLock.retained` = `[]` 且 `Array.isArray` 为真**；退出 app 0 / engine 0。离线：Rust **328/0/7**、Node **166/166**（新增 1 条）、typecheck、module-map 全绿。

### 接口库存（当前修订）

| 项 | 当前 | 第 1 轮 |
| --- | --- | --- |
| EngineCommand `type` | **61**（TS 有 Rust 无 = 0；字段不一致 = 0；Rust 有 TS 无 = 1：`camera_pipe_descriptor`，由宿主内部发出） | 61 |
| 自定义 Tauri IPC | **17**（另有 1 个平台孪生属性） | 17 |
| 错误码字符串 | **130**（125 命令拒绝码 + 3 operation 失败串 + 2 `cfg(test)`） | 129 |
| 规范已逐条解释 | **42**（未解释 88） | 32 |
| `Operation.status` 取值 | **8**（含 `rejected`），引擎→TS→适配器白名单→等待器→测试全链一致 | 未盘点 |

**任务书更正（避免把错误前提带下去）**：我在下发审计任务时列举的"第 2–13 轮新增码"里，相对第 1 轮库存**真正新增的只有 `CAMERA_ALREADY_CONNECTED`**；`DEVICE_CONTROL_LOCKED`、`PREFLIGHT_REQUIRED`、`LIVE_VIEW_APPLY_FORBIDDEN`、`NO_ACTIVE_TASK`、`INVALID_LIVE_VIEW_SESSION`、`STALE_LIVE_VIEW_SESSION` 第 1 轮已存在；`INVALID_CONFIGURATION…` 在仓库中**不存在**。库存已按集合差集重算并给出证据。

### 规范补充（本轮已写入 `frontend-functional-spec.md`）

`taskLock.retained` 语义与键集、`scanFlow.stage` 11 值闭集（含 `persisting_completed` 不是完成）、`camera.lastSeenAt` 与 `capturedAtMs`、`disabledReasons` 生产/预览键集差异、以及"130 码中 88 个未逐条文档化时必须原样显示、不得静默吞掉"的处理约定。

### task-14「第四节接口精度」核对结论

10 条中 **8 条仍成立、2 条需更正**：第 5 条（TS facade 已收窄为 `Promise<UpdateState>`，原"仍声明较宽 nullable"已过期）、第 1 条（漏写 `camera_live_view_stop` 在活动会话期间以 `camera_cleanup` 标签入队、其 `deviceSessionId` 回退 engineSessionId 的例外）。另确认第 10 条**源码强于文档**：非 test 构建的采集 profile 恒为未知且无任何人工覆盖入口，只能改源码。已把 10 条更正写入任务清单第八节（上文第四节保持交接原样）。

## 5v. 第 15 轮：补齐两处引擎已发送但前端未声明的字段（含判别断言）

**本轮最终候选**：host `2C423B80A1647158605174AE0D67C85FA4B2FE5CF64309033771941D7B891C5B`，内嵌引擎不变 `6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7`。

第 14 轮接口库存（task-13）指出两处"引擎已发送、TS 未声明"的字段（HIGH/MED）。本轮补齐并用真实载荷复核：

| 字段 | 引擎来源 | 修复 | 实机/录制证据 |
| --- | --- | --- | --- |
| `deviceControls.camera.lastSeenAt` | `devices/camera/types.rs:333`（`last_seen_at`，serde camelCase） | `src/engine/types.ts` 的 `CameraHealth` 增加 `lastSeenAt?: string \| null` 并注明语义（与 session/连接证据分开判断） | 实机快照 `"lastSeenAt":"2026-10-06T19:23:12.850034400+00:00"`、键存在；录制快照 `tests/first-launch-response.jsonl` 的 `lastSeenAt` 为 `null`（键存在） |
| 帧 `capturedAtMs` | `src-tauri/src/camera_forwarder.rs:42`（`frame.captured_at_ms`） | `src/platform/cameraControls.ts` 的 `CameraLiveFrame` 增加 `capturedAtMs: number` 并注明新鲜度仍以 session+sequence 判定 | 实机帧 `{"capturedAtMs":1791314593700,"sessionId":"4f5a39…","sequence":3,"width":640,"height":424}` |

**判别断言（防止再次漏声明）**：`tests/native-device-controls.test.mjs` 现在断言①录制引擎快照的 `camera` 含 `lastSeenAt` 键且类型为 string|null，②Rust 转发层源码含 `"capturedAtMs":frame.captured_at_ms`，③`validateLiveFrame` 接受带 `capturedAtMs` 的帧。若任一字段再被删除，测试即失败。

**实机复核**（`tmp/tests/desktop/20261007-180000-ts-fields-smoke/`）：连接 560 ms、LV start 314 ms、两次取帧（第二次返回真实帧）、退出 **app 0 / engine 0**。离线：Rust **328/0/7**、Node **166/166**、typecheck、module-map 全绿。

**规范同步**：`frontend-functional-spec.md` 的"字段精度补遗"已把这两项从"必须声明"改为"已声明并有判别测试固定"，避免读者以为仍是缺口。

### 错误码目录落盘（补上规范缺失的 88 个）

第 14 轮库存指出"130 个码中规范只解释了 42 个"。本轮由独立只读整理（`task-15`，`tmp/tests/desktop/20261007-172000-error-code-catalog/`）产出并自检，现已落盘为 **[docs/frontend-error-codes.md](frontend-error-codes.md)**（277 行）：逐码给出触发条件、是否 `ENGINE:<CODE>` 拒绝、是否伴随 fault/锁、四类前端呈现建议（`可恢复重试`19 / `需先修配置`61 / `需恢复流程`30 / `原样显示`20）与源码 `文件:行`；按 10 组组织，三类面（125 命令拒绝码 / 3 仅 operation 失败串 / 2 仅 `#[cfg(test)]`）分列，测试专用码单列并注明"前端永不可见"。

数量自洽并经**独立重算**（另一套解析逻辑，集合差为空）：分组小计 9+18+37+10+17+24+5+5+3+2 = **130**；三类 125+3+2 = **130**。`module-map:check` 在新增该文档后仍 PASS（1853 files / 16 modules / 25 boundaries，模块图 current）。

**重要更正（我的任务前提有误，已由整理者按源码纠正）**：`rejected` 是 **Operation.status**，不是错误码——130 个码中**没有任何一个本身是 `rejected`**；判定只匹配 `Operation.error` 全文（两种 partial-apply 前缀 + 含 `PTP response 0x201c` + 排除 8 个传输标记）。命令级的值拒绝等价物是 `CAMERA_PROPERTY_UNSUPPORTED_VALUE`。目录第 2 节已写明，规范同步引用。

## 5w. 第 16 轮：补齐只读/断开类接口条目的实机覆盖（T2/T3/T8）

此前多轮验证集中在连接、读取、Apply、Live View、拍摄门禁与 Stop；本轮把接口表里**尚未实机走过的条目**补齐（候选 host `2C423B80…`／引擎 `6C2E587F…`，`tmp/tests/desktop/20261007-190000-interface-entries/`）：

| 命令 | 实测 | 耗时 |
| --- | --- | --- |
| `camera_status`（未连接） | ok | 23 ms |
| `camera_identity`（未连接） | ok | 11 ms |
| `turntable_status`（Nano 未连接） | **`NANO_NOT_CONNECTED`**（正确拒绝，非 fault） | 9 ms |
| `turntable_identity`（未连接） | ok | 17 ms |
| `camera_connect`（显式 deviceId） | ok，session `9d1543e1…` | 562 ms |
| `camera_identity`（已连接） | ok | 20 ms |
| `camera_status`（已连接） | ok | 21 ms |
| **`camera_disconnect`** | ok，相机断开且 `cleanupConfirmed=true` | **350 ms** |
| 断开后再 `camera_connect` | ok，**新** session `03154f13…` | 571 ms |
| **`retry_device{device:"camera"}`** | ok，仅发现（kind `camera_discover`，不重连、不选设备） | 299 ms |

最终状态：`phase=idle`、相机连接、`cleanupConfirmed=true`；operation 序列恰为 `camera_discover:completed → connect:completed → disconnect:completed → connect:completed → camera_discover:completed`；退出 **app 0 / engine 0**。无 fault、无锁。

**意义**：接口库存里被标为"渲染层无生产调用点"的只读条目（`camera_status`/`camera_identity`/`turntable_identity`/`turntable_status`）与显式断开/重试路径现在都有本轮实机证据，不再是"只在源码与离线测试里存在"。

## 5x. 第 17 轮：T2 错误信封与回执关联的实机探测（含两处载荷策略确认）

T2 的验收条文要求"未知命令、缺字段和无资源错误清晰"，此前多轮未在**活动 IPC 上**逐条探测。本轮在候选 host `2C423B80…`／引擎 `6C2E587F…` 上做完（`tmp/tests/desktop/20261007-200000-t2-envelope/`）：

| 探测 | 实测响应 | 判定 |
| --- | --- | --- |
| 缺 `command` 键 | `invalid args 'command' for command 'engine_command': missing required key command` | 清晰（传输层） |
| `command.type` 非字符串 | `Command type is required` | 清晰 |
| 未知命令 `definitely_not_a_command` | **`ENGINE:UNKNOWN_COMMAND`** | 清晰 |
| `camera_focus` 缺 `step` / `step` 为字符串（相机未连接时） | **`ENGINE:CAMERA_NOT_READY`** | 正确：连接状态先于字段校验（规范已写明该顺序） |
| `turntable_status`（Nano 未连接） | **`ENGINE:NANO_NOT_CONNECTED`** | 清晰 |
| `camera_read_properties`（相机未连接） | **`ENGINE:CAMERA_NOT_READY`** | 清晰 |
| `engine_snapshot` 带无关参数 | 正常返回完整快照 | 额外参数被忽略 |
| `camera_connect` 的 `deviceId` 为**数字** | **接受**，降级为发现刷新（kind `camera_discover`，不连接、不报错、不锁故障） | 见下 |
| `camera_status` 带未知额外字段 | 接受并正常执行 | 见下 |

**两处载荷策略（本轮确认，已写入规范，不改代码）**：

1. **`deviceId` 非字符串 ⇒ 视为"未显式选择设备"**：与空白串/D-1 修复同一判据（`trim` 后非空才算选择）。这是 **fail-safe** 的：不产生任何设备动作、不锁故障；规范早已要求前端按 operation kind 判断"发现"与"连接"，因此该降级是可检测的。对比第 9 轮的空白 id 缺陷（当时会**绕过门并 fault**）——那种有实际危害的情形已修；本情形无危害，故选择**如实记录 + 规范写明**，而不是为一个无危害边界改动既有语义（`COMMAND_PAYLOAD_MISMATCH` 有特定含义：JSONL 信封 `type` 与 `command` 不一致，误用会污染该码语义）。
2. **额外未知字段被忽略**（前向兼容）。与 `camera_apply_properties` 的**属性键**严格校验是两回事：后者键名直接指涉设备状态，前者只是载荷冗余。

**回执/会话/序号关联**（同轮证据）：所有响应共享同一 `engineSessionId`（`engine-46828-…`），`snapshotSequence` 单调递增（58 → 60 → 64 → 65），被接受命令的 `operationId` 形如 `<session>-op-N`；前端运行时 `mode=production_locked`（无模拟回退）。退出 **app 0 / engine 0**。

## 5y. 第 18 轮：用户要求下的再次实机验证（T7 决定性证据 + 当前构建回归）

用户要求"再次尝试实机验证"。候选不变（host `2C423B80…`／引擎 `6C2E587F…`），证据 `tmp/tests/desktop/20261007-220000-t7-retry/`（14 步，会话 `25696a69…`，退出 **app 0 / engine 0**）。

### T7 的决定性证据：软件根本无法改释放模式

本轮不再只是"读到 `driveMode=32785`"，而是**直接尝试写入**：

| 请求 | 实测 |
| --- | --- |
| `camera_apply_properties {driveMode:1}` | **`ENGINE:CAMERA_PROPERTY_UNSUPPORTED_VALUE`**（20 ms）——descriptor 报 `nativeWritable=false`，机身不声明可写 |
| `camera_capture`（已设 `set_capture_root`） | `CAMERA_CAPTURE_UNQUALIFIED`（29 ms，原因 `scan requires driveMode=1`） |
| `camera_test_capture` | `CAMERA_CAPTURE_UNQUALIFIED`（19 ms，同一门禁） |
| 结果 | `lastCapturedImage=null`、**未创建任何目录**（`capture-root` 下无子目录），文件保护成立 |

即：**T7 无法由软件推进**——唯一路径是把机身释放模式拨盘拨到**单张**（或人工批准"接受自拍模式"的设计变更）。这不是引擎缺陷，引擎正确地拒绝了不可写属性并给出明确码。

### 当前构建回归（同会话，全部通过）

| 项 | 结果 |
| --- | --- |
| `set_capture_root` | 46 ms |
| `iso 1000 → 200 → 1000` | 570 ms / 849 ms，终态 `iso=1000`（恢复核对通过） |
| `camera_live_view_start` | 341 ms，nonce `69ab5498…` |
| **8 秒仅内存取帧** | **30 帧、0 重复、序列单调（2→70）、640×424、单一 sessionId、2 次空帧**；另有 **1 次帧管道错误**（见下） |
| `camera_live_view_stop` | 621 ms，会话保持连接 |
| 全局安全 `stop` | 289 ms → `phase=stopped`、`cleanupConfirmed=true` |

### 新观察（F-11 候选，记录未改代码）：帧管道瞬时错误

8 秒 32 次取帧中出现 **1 次** `Error: 管道的另一端上无任何进程。 (os error 233)`（Windows `ERROR_PIPE_NOT_CONNECTED`），随后立即恢复正常取帧，LV 未中断、引擎无 fault、会话无异常。这是 **Tauri 帧管道层**的瞬时空读，不是相机/引擎错误。

处理：规范明确要求"不能把所有错误当 null"（避免把真实故障静默成缺帧），但**瞬时管道错误**应与 `LV_NOT_ACTIVE`/陈旧会话区分——前端应保留最近一帧并标 STALE、下一拍重试，**不得**据此断开 LV 或报设备故障。该指引已补入规范；是否让传输层把"管道未连接"直接归一为 `null` 属后续可选项（需先确认不会掩盖真正的管道关闭/退出场景），本轮不动代码。

## 5z. 第 19 轮：T7 独立 NEF PASS与 F-12 长曝光对照

日期：2026-10-07（Asia/Shanghai）。本轮继续用户已授权的 D7100 独立 NEF 验收；真实设备由主智能体操作，全部命令经真实 Tauri → EngineClient → JSONL → ct-engine。原失败 run 保留；本轮没有修改设备源码、传输策略、安全门禁或正式入口。

### 版本、设备与源码边界

- 实际仓库：`E:/LZU_Master/MicroCT/Micro-CT-App`；分支 `native-device-controls-offline`，HEAD `11eed6cfea8f3f5118fcd07e70319f98fc398b51`。
- 启动前保存 105 处已有修改的状态和逐文件 SHA-256，见各 run 的 `baseline-source.json`；本轮只追加/纠正验收及架构文档和归档 UI 截图，生产源码保持基线字节不变。
- 候选 host `56EC7FABF9770334302CF270B5504891CFCB12C1199621D2950D797A6F03ECA8`；三个 run 实际运行引擎均为 `6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7`，由各 `process.json` 的实际路径核对。
- D7100：`USB\VID_04B0&PID_0430\8322769`，型号/序列号与鲜读身份一致；CH340 COM7 在位，本轮未连接或动作转台。9400 启动前空闲，三次退出后均释放。旧 PID 52332/40964 仍显示为零线程、零句柄残留，不是本轮 app/engine；未取得清除证据。
- 正式入口仍为 `C19B828D8F18D13D3EF2811B95B5094863AEE0E494611140001BC69716C91334`；没有发布、安装、覆盖或重新编译该入口。

### T7：PASS

证据：`tmp/tests/desktop/20261007-191045-t7-short-exposure/`，`steps.jsonl`、`fresh-baseline.json`、`restored-baseline.json`、`latest-snapshot.json`、`process.json`、`exit.json`、`result.json` 与 `verify.mjs`。

实际步骤：`node session.mjs plan-discover.json` → 显式设备连接/鲜读 → `plan-restore-prior.json` → `plan-prepare-short.json` → `plan-capture.json` → `plan-preview.json` → `plan-restore-final.json` → `plan-exit.json` → `node verify.mjs`。每项写入都等待 completed 并核对 actual/confirmed；harness 的 ok=true 只说明请求受理，验证脚本另查匹配 operation 终态。

| 项 | 真实结果 |
| --- | --- |
| 恢复上轮遗留设置 | 明确 Apply `recordingMedia=0`、`longExposureNoiseReduction=1`，Read 读回一致；不是默认保留扫描值 |
| 拍摄前条件 | Apply `shutter=65537`（1 s）、`recordingMedia=1`、`longExposureNoiseReduction=0`；Read 确认 `rawFormat=4`、`driveMode=1`；显式 LV stop 完成 |
| 文件与登记 | `captureId=operationId=909515c5c237a698c15d00783f2e9686-op-14`，稳定连接 session `909515c5c237a698c15d00783f2e9686`；23,901,927 字节完整 NEF，独立 SHA 与登记一致 |
| 可靠曝光结束 | 本次同 session/op 中恰好一个 C101、一个 C102；`exposureEndedObserved=true`、`lastCapturedImage.exposureEnded=true`；不是从快门时长或文件出现推定 |
| 内存预览 | `camera_capture_preview {captureId}` 成功；返回同 session/op，5,887,466 字节 JPEG 只在内存，harness 仅保存 byte count；capture-root 只有一个 .nef，没有 JPEG、base64 或截图 |
| 扫描隔离 | 使用者本轮明确确认按现有独立照片契约验收：`imageCount=0`、`progress.current=0`。源码 `lib.rs` 的 imageCount 来自扫描 real_frames；单拍只增加独立登记，不增加扫描帧数/manifest |
| 恢复与退出 | shutter 恢复 1966081（30 s）、recordingMedia 恢复 0、longExposureNoiseReduction 恢复 1；34 项 actual/confirmed/error 与恢复基线 diff=0；app 0 / engine 0，无强杀 |

该 run 的发现之后还出现过 UI 发出的连接/Apply 回执；本轮显式重复 connect 收到 `CAMERA_ALREADY_CONNECTED`，无 fault，鲜读正常。完整回执历史保留，不把这次拒绝报告成连接成功。

### F-12：原失败保留；本轮未复现，根因仍 BLOCKED

原始失败：`tmp/tests/desktop/20261008-020000-t7-capture/`，实际 capture 时间 `2026-10-07T07:01:18.013Z`，耗时 23,771 ms，shutter=1966081（30 s）；WPD `0x8007001F`、无 C101/C102、无 ExposureEnded/NEF，随后 fail-closed 隔离。匹配 capture receipt 在 Stop 取消后终态为 cancelled/cancelRequested=true，不能把 harness 的 ok=true 当成功。原 exit.json 为 FAIL（app 正常关闭未确认、engine 1）。使用者说明 USB 是失败后手动拔下；不以其后的设备缺席倒推拍摄失败原因。

本轮依次比较 1 s、5 s、15 s、30 s，再启一个新 app/engine/相机会话，直接拍首张 30 s，排除依赖短曝光预热这一解释：

| 实际快门 | capture 总耗时 | NEF 字节 | 文件 SHA-256 |
| --- | --- | --- | --- |
| 1 s | 5144 ms | 23901927 | `78f6644ce2241191f46a918a8ae5302b8190d7ffc0ce2a5383ee531c62c42823` |
| 5 s | 9211 ms | 24927634 | `75469868634290679349d9315786caa4fa7ec2d5c3cf582550c930fba104403f` |
| 15 s | 20471 ms | 26036182 | `b7cf096ec163062bb7127ca7647a93eed8324f7be6d8ce31b7f4514d71809867` |
| 30 s | 36316 ms | 27210412 | `4b85190dd34d44974d9d602279a0054ae11040a80256bcafa1e51b6a94d5037e` |
| 30 s（新会话首张） | 36286 ms | 23803424 | `a2836c40710ce9b3f435758d854452cc82ebcca31332503be8d8bdcd14bbeb67` |

五张全部 completed，每张都有匹配 C101/C102、ExposureEnded、完整 NEF/大小/独立 hash。5/15/30 s 在同一会话 `287a5d56e5ec210c49c7706533ead732`，新会话首张 30 s 在 `b0912f8e6949bb53c81d25b65540a2a1`。两轮的 34 项恢复 diff=0，app/engine 均 0/0；新会话 30 s 的 ID 预览也成功（1,242,040 字节，仅内存）。

`f12-comparison.json` 证明原失败前与本轮 30 s 前的 34 项实际摄影值差异为空，host/engine 相同。因此“长曝光必然超过 23.8 s 软件上限”不成立；源码 capture 总预算为 75 s，原错误是 WPD Unavailable，不是 Camera Timeout。成功对照只能证明当前可拍，不能证明原错误已经修复或解释 USB/驱动/机身为何返回通用设备失败。

原 capture 的已记录事件操作为 `GetEvent 0x90C7`，0x4006 参数 `0xD1A4`/`0xD1B4`。`0x802A0002` 出现在后续 cleanup 的 GetEvent 中，晚于原 `0x8007001F`；它不是首个错误。`DeviceReady 0x90C8` 的具体失败阶段没有留在原诊断；本次没有 `GET_LV 0x9203` 拍摄操作，当前相机源码也未发出 `0x9207`。不补造未记录的操作码或 PTP 响应。

Microsoft 将 0x1F 定义为通用 `ERROR_GEN_FAILURE`，不提供曝光超时的专属结论（[系统错误码](https://learn.microsoft.com/en-us/windows/win32/debug/system-error-codes--0-499-)）；WPD 定义 `0x802A0002` 为 `E_WPD_DEVICE_NOT_OPEN`（[WPD 错误常量](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/error-constants)）。原始中文错误文本仍原样保留。只读检查原失败时间窗的 System/WPD-MTPClassDriver 日志未找到对应设备事件，DriverFrameworks 窗口无记录；缺少记录不能证明链路没有异常。

处置：**PASS** 当前短/长曝光及恢复；**FAIL** 原始 F-12 拍摄/退出；**BLOCKED** 原始触发条件与精确根因，需同类再次发生时的具体 WPD/PTP 阶段或设备/驱动 trace 才能归因。本轮没有可复现根因可修，不增加推测性重试、缩短支持曝光范围或放宽未知清理门禁。F-12 不标“已修复”。缺陷记录：`tmp/tests/desktop/20261007-191803-f12-exposure-ladder/DEFECT-F12-revalidation.md`。

### 两栏确认、门禁与未测边界

使用者确认保留 Camera Actual 的 tooltip/Advanced 与 Turntable 分区微标题/tooltip。本轮没有视觉源码改动，保留已经构建的候选；最小截图与说明归档到 [两栏重设计确认](shots/2026-10-07-panel-redesign/README.md)，不把 UI 归档当作后端硬件认证。

三轮的设备/文件验证脚本全部 PASS；文档/前端门禁命令及退出码在短曝光 run 的 `gates.json`。Rust 源码未改，本轮不重复 cargo；328 passed / 0 failed / 7 ignored 是交接所列现有离线门禁，不伪称本轮重跑。射线连接/参数/开束、HOME/MOVE、真实扫描继续 BLOCKED（无本轮授权与射线连接，Multi UNKNOWN 无覆盖入口）；重构数值精度、软件更新维护锁仍仅有既有离线证据。完整联动硬件认证仍未完成。

## 5aa. 第 20 轮：F-12 原失败时序与 WPD 驱动日志审计

日期：2026-10-07（Asia/Shanghai）。**仅离线/只读审计**，没有启动候选或调用相机、转台、射线；T7 与两栏确认保持第 19 轮结论。证据为 `tmp/tests/desktop/20261007-201153-f12-timeline-audit/` 的 `audit.ps1`、`wpd-events.json`、`timeline.mjs`、`result.json`、`summary.md` 和 `command.txt`。

### 新证据与更正

第 19 轮事件检查使用 `D7100|VID_04B0|WPD|WUDF|MTP|8322769` 筛选消息文本，没有匹配到设备事件。**这不等于 WPD 日志窗口没有事件**。本轮直接读取 `Microsoft-Windows-WPD-MTPClassDriver/Operational` 的完整时间窗，不按设备字符串筛选，保存到 `wpd-events.json`：

| 时间（UTC） | 原始证据 |
| --- | --- |
| 07:01:18.024 | capture operation 开始，ID `cf2f28fcf80b542dd82a54ac1fd66494-op-7` |
| 07:01:41.5075856 | WPD event 1002，record 189，驱动宿主 PID 53460；消息“设备正在进入空闲状态(空闲状态: 4; 返回代码: 0x0)”，EventData `State=4`、`hr=0` |
| 07:01:41.512 | 引擎 safety Stop operation 开始，ID `engine-39352-1791356403481718300-op-8` |
| 07:01:41.524 | capture receipt 变为 `cancelled`、`cancelRequested=true`、`error=null` |

驱动事件比 Stop 的开始时间早约 **5 ms**；原相机 health 仍保留 `0x8007001F` 与 `cleanup_unconfirmed`，完整 NEF/ExposureEnded 缺失。三个成功会话的退出时间窗没有记录到同类事件。

该 event **不含设备 ID，也不含具体 PTP 操作/属性码或失败 HRESULT**，`hr=0` 仅是这条驱动事件的字段。本轮不猜测 State=4 的机身/USB含义，不把它当作物理拔线、超时、WPD 自主断连或 capture 首个错误的证明。使用者关于失败后才拔 USB 的说明继续保留。

### cancelled 回执不能确定 Stop 的起因

当前源码解释了为什么原 receipt 不能独立用来判断“用户先按 Stop”或“拍摄已被正常取消”：

1. `camera/actor.rs` 的 worker 在 native 错误后执行 cleanup、发布 health，然后才回送结果；未确认的原始设备错误可以先进入 health。
2. `device_controls.rs::sync_camera_readback` 发现 lost/unconfirmed health 时，可以自动 `enqueue_stop(true,false)`，使普通操作 generation/cancel 标记失效。
3. `sync_operations` 对随后收到的旧 generation 结果不执行成功 mutation；如果错误不含 cleanup-unconfirmed 标记，会将 operation 记为 `cancelled` 且 `error=None`。原始错误仍可存在于 camera health。

这是**源码允许且与原记录一致的时序**，原记录没有可区分 Stop 来源的完整证据，不能据此声称已证明唯一触发路径；本轮没有修改取消语义或安全逻辑。

### 本轮结论与剩余条件

- **PASS**：原失败回执/驱动事件/源码时序已保存，纠正了消息字符串筛选遗漏；没有活动 app/engine，9400 空闲。旧 PID 52332/40964 仍为零线程、零句柄残留。
- **BLOCKED**：WPD 返回原 `0x8007001F` 的具体 PTP 阶段和上游原因仍缺证据。现有失败材料、事件日志与成功对照不足以判定机身/USB/驱动/软件中的哪一层首先失效。
- 本轮只更新文档，生产源码、候选/实际引擎与正式入口哈希保持 §5z 身份。文档门禁、源文件保留检查的输出保存在本轮 run；没有重跑硬件或把未复现标为已修复。后续归因需要同类失败时的具体 WPD/PTP 阶段或设备级 trace。

## 5ab. 第 21 轮：使用者重启相机后的短/长曝光复测

日期：2026-10-07（Asia/Shanghai）。使用者说明已重启相机并询问是否能解决问题。本轮沿用既有相机连接/参数/独立拍摄授权，使用当前候选在新的 app/engine/相机会话验证；没有连接射线或转台，没有运动/开束/上传固件。

证据：`tmp/tests/desktop/20261007-215214-camera-reboot-check/`，原始 `steps.jsonl`/快照/计划、`process.json`、`exit.json`、`result.json`、`command.txt` 与验证脚本；拍摄经真实 Tauri→EngineClient→JSONL。

| 身份 | 实测 |
| --- | --- |
| 源码分支 / HEAD | `native-device-controls-offline` / `11eed6cfea8f3f5118fcd07e70319f98fc398b51`，保留已有修改，生产源码未改 |
| host SHA-256 | `56EC7FABF9770334302CF270B5504891CFCB12C1199621D2950D797A6F03ECA8` |
| 实际运行引擎 SHA-256 | `6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7`，路径由 process.json 核对 |
| 设备 | D7100，VID04B0/PID0430，USB serial8322769；原生 serial 字段含尾随0，保留原返回；相机会话 `eed9085f2089fe02a7842bc64af29176` |
| 正式入口 SHA-256 | `C19B828D8F18D13D3EF2811B95B5094863AEE0E494611140001BC69716C91334`，未覆盖 |

重启后的鲜读原值已经是 `recordingMedia=0`、`longExposureNoiseReduction=1`、`shutter=1966081`（30s）、`rawFormat=4`、`driveMode=1`，全部 confirmed。保存该基线后，显式 Apply 1s/RAM/NR-off、明确停LV、鲜读，捕获到本 run 隔离根；随后 Apply并读回30s后捕获。

| 快门 | 拍摄总耗时 | 完整NEF字节 | SHA-256 |
| --- | --- | --- | --- |
| 1 s | 5999 ms | 23659648 | `b3c3fe89e828988e2864084a7fc8848d16f33af6f9e590eb6d57e9e70c07e03b` |
| 30 s | 36659 ms | 26523357 | `27541e9e9bb5a61a0a68fcda85642a8012b467e7e011ce3cf99ddc65a7bab695` |

两张均 **PASS**：匹配 session/op 的 C101/C102、可靠 ExposureEnded、完整NEF/大小/独立hash/登记及 completed receipt。短曝光 ID 预览成功（6140032字节JPEG，仅内存）；隔离capture-root只含两张NEF，没有JPEG、base64或截图。扫描imageCount/progress仍0。

结束时恢复本次鲜读的30s、recordingMedia=0、longExposureNoiseReduction=1；34项actual/confirmed/error diff=0。正常退出 **app0 / engine0**，无强杀；9400释放，本轮没有app/engine残留。旧零线程/零句柄PID52332/40964环境残留保持独立，不当作本轮运行。

**对使用者问题的结论**：相机重启后当前连接、读回与短/长曝光拍摄均正常，恢复检查PASS。重启前第19轮也已有成功对照，原F-12没有可重复触发条件，因此本轮不能建立“重启消除了原根因”的因果结论。F-12仍为原失败保留、本轮未复现、精确根因BLOCKED；没有修改代码或标已修复。T7与两栏反馈确认保持完成，完整射线联动/扫描/维护锁的未测边界保持不变。

**后续只读诊断对照**：`node compare-runtime.mjs` 对原失败 `T ready` 与本轮 `F12 before native shutter 1966081` 比较，结果保存于本run的 `runtime-comparison.json`。34项摄影值/确认/错误差异为空；DeviceInfo数据hash、厂商属性目录hash、operation列表和effectiveProperties均相同。10项runtime中8项相同，仅原生 `batteryLevel` 为100→60、`exposureIndicateStatus` 为54→58；均保留原始confirmed读数和未知语义，不把差值翻译成故障原因。镜头类别、LV选择器/禁止条件、显示诊断等没有对应状态差异。该对照没有提供具体WPD/PTP失败阶段，F-12根因仍未确证；本次没有新拍摄或设备命令。

本轮文档门禁/源文件保留核对仅离线验证，输出见本run的module-map.log/source-preservation.json；不重复Rust/前端测试，不重编译未改变的候选或正式入口。

## 6. 证据目录

| 目录 | 内容 |
| --- | --- |
| `tmp/tests/desktop/20261006-150714-connected-backend-host/` | T1 构建：命令、日志、退出码、源码逐文件哈希、二进制哈希 |
| `tmp/tests/desktop/20261006-151200-connected-device-session/` | T2–T6 与 F-01 原始证据（含 `DEFECT-F01...md`、失败快照、诊断） |
| `tmp/tests/desktop/20261006-153500-connected-device-session-t7-t9/` | T7 尝试、T8、T9、正常退出证据 |
| `tmp/tests/desktop/20261006-155100-f01-fix-verification/`、`20261006-160200-f01-fix-acceptance/`、`20261006-155600-f01-retry-acceptance/` | F-01 修复的冒烟、被证伪的首次尝试、当前重试修复（后者被机身 LV 阻断） |
| `tmp/tests/desktop/20261006-151500-rust-offline-audit/`、`-contract-audit/`、`-doc-precision/` | 三个子智能体的离线审计报告 |
| `tmp/tests/ct-engine/20261006-154500-camera-cleanup-lv-off-fix/` | 修复后 cargo 日志与统计 |
| `tmp/tests/desktop/20261006-154800-fixed-host-build/`、`20261006-155300-fixed-host-build-2/`、`20261006-170500-round2-fixed-host-build/` | 三次修复构建日志与哈希 |
| `tmp/tests/desktop/20261006-163000-round2-fix-verification/` | 第 2 轮独立对抗验证报告（V-01…V-06）、cargo 日志、xray 间歇复现、改动前二进制字面量取证 |
| `tmp/tests/desktop/20261006-163000-doc-architecture/` | `docs/architecture/` 目录树与接口语义同步报告 |
| `tmp/tests/ct-engine/20261006-164500-round2-verifier-fixes/` | 整改后 cargo 连续 3 次运行日志、typecheck、module-map、Node 测试 |
| `tmp/tests/desktop/20261006-171000-round2-device-acceptance/`、`20261006-174000-round3-device-v06/` | 第 2/3 轮实机回归：Nano 连接/状态/STOP、相机缺席证据、V-06 修复前后 A/B、正常退出 0 |
| `tmp/tests/desktop/20261006-173500-round3-fixed-host-build/` | 第 3 轮构建日志与哈希 |
| `tmp/tests/desktop/20261006-180000-round3-final-verification/` | 第 3 轮最终修订独立验证报告（8 组 PASS + D-1…D-7） |
| `tmp/tests/desktop/20261006-180500-round3-final-device/`、`20261006-175500-round3-final-host-build/` | 构建 6 的实机验证（V-06、Nano、正常退出 0）与构建日志 |
| `tmp/tests/ct-engine/20261006-173000-round3-v06/` | 第 3 轮全量 Rust 回归日志（含新增 3 个回归测试） |

## 7. 未验证边界与剩余风险

> 本节原为第 2/3 轮文本（当时 D7100 未枚举）。**当前 T7/F-12 结论以 §5z 为准；第 2–18 轮保留为历史证据**；下面已按当前证据重写。

- **已完成实机复核（不再是"无证据"）**：相机身份/Read/Apply 与恢复（T3/T4）、Live View nonce 与仅内存传输（T5）、LV 中的拒绝路径（T6，第 13 轮补齐：`LIVE_VIEW_APPLY_FORBIDDEN`/`LIVE_VIEW_CAPTURE_FORBIDDEN`/`INVALID_CAMERA_FOCUS_STEP`/`CAMERA_AUTOFOCUS_UNQUALIFIED`，且 LV 不中断）、Nano 连接/状态/STOP 与退出（T8）、扫描准备/预检/恢复/重构/放弃拒绝路径（T9，第 13 轮补齐 `restore_previous`/`start_reconstruction`/`abandon_scan`）。**F-01 已修并在两代候选实测**（Stop 947–1077 ms、`cleanupConfirmed=true`、可立即重连、退出 0/0），F-02/F-03/F-04/F-06/F-07/F-08/F-09/F-10 亦已修复并验证（§5f–§5s）。
- **F-07 机身侧（已改进）**：第 7 轮起 Stop 会先用有效会话结束 Live View；若机身被其它上位机留在 Live View，connect 时会**先尝试** `force_live_view_off` 自动清除，**清不掉仍拒绝连接**并要求检查机身。该自动清除成功路径尚无实机执行证据（见 §5m 与规范 §5）。
- **T7 独立 NEF PASS（§5z）**：机身已拨到 S，鲜读 driveMode=1；1/5/15/30 s 共五张完整 NEF、关联 C101/C102/ExposureEnded、capture-ID 内存预览、恢复与正常退出均已验证。原 F-12 未复现，精确根因仍 BLOCKED；不称已修复。
- **射线全部未验证**：无连接与出束授权；2 秒最小再开束间隔、关束读回、300 秒休息策略、联锁与温度均无本轮实机证据（离线审计 PASS 不等于硬件认证）。
- **HOME/MOVE 未验证**：无现场运动空间与射线物理隔离条件。
- **真实扫描未验证**：受 `multipleExposureEvidence=UNKNOWN` 门禁阻断；当前源码不存在人工覆盖入口，前端确认不能解除。
- **重构未验证**：无已封存投影数据可算（仅验证了非法方法的拒绝路径）。
- **软件更新/维护锁**：未触发 `prepare_software_update`，维护锁与升级路径未验证（仅离线测试）。
- **已知记录项（非缺陷）**：D-3——LV 控制事务超时会按 fail-closed 退役会话（实测正常事务 66 ms 对 2 s 上限约 30× 余量）；第 12 轮实测并排队的非 LV 只读请求不会阻断 Stop（663 ms 干净停止）。
- 离线门禁通过只证明离线逻辑；以上实机项均不得标记为硬件认证完成。
