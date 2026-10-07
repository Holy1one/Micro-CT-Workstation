# 后端基本流程与前端暴露接口验收（2026-10-05）

2026-10-06补入最终证据，文件名保留原日期。当前用户范围：不继续调整前端；核对后端底层、基本流程和可供新前端调用的接口，并交付[功能规范](frontend-functional-spec.md)。本轮后端离线门禁、真实Tauri IPC与Nano身份/状态/普通Stop/维护门禁分项PASS；不能宣布完整后端硬件可扫描。

当前源码0.9.0-rc.3、MSI账本0.9.3。本轮构建的是后端接口验证host，不继续发行installer、打包/签名/GitHub/MSI工序或UI设计；旧[rc2独立验收](workstation-restructure-rc2-acceptance-2026-10-05.md)及其签名产物保留独立，不删除有效过程资料。

| 接口/流程 | 本轮状态 | 已证明范围 / 尚未验证边界 |
| --- | --- | --- |
| JSONL v2与engine_snapshot/command | PASS（离线＋真实IPC） | 当前rc3完整snapshot、unknown/missing命令明确Err；typed严格字段与Value兼容边界分开，不宣称所有extra均deny |
| operation ack→同session/op完成 | PASS（离线＋Nano分项） | queued/running/cancel/失败/未知、迟到/Stop优先；实际维护session/op匹配与cancel释放，不把ack当完成 |
| 相机身份/摄影Read/Apply/依赖 | PASS（仅离线）；实机BLOCKED | 174500修复3项旧缺口；113camera＋20domain，原生descriptor/readonly/未知/提前拒绝；当前相机缺席 |
| 独立单host NEF/capture ID预览 | PASS（离线/真实IPC拒绝）；新硬件BLOCKED | RAM/NEF/hash/ID-only合同；实际未登记capture明确Err，不伪造照片；本轮无真实捕获 |
| LV启停/新nonce/read/configure/像素IPC | PASS（离线/真实inactive拒绝）；实机BLOCKED | two-key/UINT8/会话撤销与内存门禁；actual LV_NOT_ACTIVE为正确Err，本轮没有LV像素 |
| 镜头/selector/activeSlot资格诊断 | PASS（仅离线）；实机BLOCKED | 不把nativeWritable或lens1当motor能力；新D061/motor资格未经真实设备验收 |
| prepare_scan/Preflight/HOME/scan prompt | PASS（离线＋实际invalidPrepare拒绝）；真实scanBLOCKED | 原子拒绝无半提交、鲜读不隐写；本轮没有HOME、参考帧或完整扫描 |
| pause/Stop/restore/abandon持久锁 | PASS（离线＋实际Nano普通Stop/维护锁分项） | 文件保留与旧profile只读合同；实际普通Stop，不把afterStop status=null当位置归零 |
| 重构/日志/NEF-preview只读边界 | PASS（离线＋实际未登记拒绝） | frame index/captureId/method-only；无Frame/无完成重构明确Err；本轮不认证重构数值精度 |
| 更新维护/暴露平台接口 | PASS（离线＋实际Nano维护门禁） | turntableStop confirmed、camera/Xray never-owned，matchingcancel解锁；本轮无installer，旧NSIS/portable证据独立 |
| MSI真实安装/UAC选择 | BLOCKED（未执行/用户未选择） | 当前用户范围不要求继续安装发行工序，不由NSIS/portable外推 |
| 本轮最终backend/DTO/既有前端契约门禁 | PASS（仅离线验证） | 264library＋2main＋52desktop、165Node，typecheck/moduleMap0；7library ignored不计PASS |

## 当前host、实际IPC与Nano

[234619构建结果](../tmp/tests/desktop/20261005-234619-backend-current-host/result.json)PASS，host SHA-256 `f88c9da63040926be4bd94564846b8f73baec68207fa6e11d431916f64b64de1`。该构建记录的`engineSha256=af8c6757719bab348217ca67cbcf8e551b76dd25be2c8367dbcdab3aa01b8e03`是外侧`target/release`产物，**不是实际内嵌payload**；`src-tauri/build.rs`以独立profile在`target/embedded-engine/release`构建引擎。以[235531实际进程](../tmp/tests/desktop/20261005-235531-backend-ipc-contract/process.json)为准，内嵌/实际运行engine SHA-256为`e7bf81010c0fd8a393d14c5a05559cb7762e957adabd09dd4f9bb31e18a04a61`，runtime文件为`ct-engine-e7bf81010c0fd8a3.exe`。

[235531真实IPC结果](../tmp/tests/desktop/20261005-235531-backend-ipc-contract/result.json)PASS：当前rc3 snapshot；fresh唯一根resolver派生CameraCaptures且createdDirectory=false；invalidPrepare原子拒绝；unknown/missing命令、无Frame、未登记capture、无完成重构均明确Err，没有fake数据；inactive LV返回`ENGINE:LV_NOT_ACTIVE`。冷启动真实无Camera/Xray时呈现UNKNOWN与实际NoDevice，不声称beam OFF或setpoints verified。

本轮仅连接Nano COM7，设备读回RTS9060_NANO、2.1.0、build20260918、protocol2、microsteps8/pulsesPerRev96000；字符串/比例读回不单独证明flash、负载或运动精度。执行Identity/Status/普通Stop，维护报告本session/op ready、maintenanceLocked/noActiveTransactions、turntableStop=confirmed，cameraCleanup/xrayOff=never-owned；维护锁拒根目录变更，matchingcancel完成且释放维护锁。[App/engine退出](../tmp/tests/desktop/20261005-235531-backend-ipc-contract/exit.json)均0。

未执行HOME/MOVE、camera操作、射线、LV帧/拍摄、installer或视觉设计。before位置−1141 pulses、referenceValid=false/LOCKED；afterStop status=null表示未知/无当前状态，**不证明归零或已建立安全角度参考**。实际Stop和维护确认不扩大为转台运动/联动认证。

### 保留失败过程与metadata修复

[224954 FAIL](../tmp/tests/desktop/20261005-224954-backend-ipc-nano/result.json)因CDP未提取字符串型native rejection，脚本在Nano连接前失败；[231209 FAIL](../tmp/tests/desktop/20261005-231209-backend-ipc-probe/result.json)误期待inactive LV为null，实际LV_NOT_ACTIVE正确。两轮均在未发Nano连接时正常App/engine退出0；失败记录保留，不改写为后端动作失败或最终PASS。

[150000 snapshot metadata修复](../tmp/tests/ct-engine/20261005-150000-snapshot-metadata/result.json)PASS、NO HARDWARE，保留[before失败](../tmp/tests/ct-engine/20261005-150000-snapshot-metadata/before-result.json)。外层status只反映实际health，不虚构相机、beamOFF、已验证设定或联锁；断开后current UNKNOWN，原last OFF证据继续用于清理，动作安全、2秒间隔和closed-owner裁决未改。当前235531真实启动亦确认未知与NoDevice，没有用模拟成功替代设备证据。

## 最终门禁与历史范围

[最终命令](../tmp/tests/desktop/20261006-000037-backend-final-gates/command.txt)：`cargo test --workspace --offline`、`npm.cmd run typecheck`、`node --experimental-strip-types --test tests/*.test.mjs`、`npm.cmd run module-map:check`。[Rust结果](../tmp/tests/desktop/20261006-000037-backend-final-gates/rust-result.json)/[日志](../tmp/tests/desktop/20261006-000037-backend-final-gates/rust.log)264＋2＋52全绿，7ignored不计PASS；[契约结果](../tmp/tests/desktop/20261006-000037-backend-final-gates/contracts-result.json)/[日志](../tmp/tests/desktop/20261006-000037-backend-final-gates/contracts.log)165Node、typecheck/moduleMap退出0。这是后端源/协议及已有契约测试，没有重复UI设计或新增像素/字体发行验收。

已有过程可作追溯：[163000 scripted LV/camera/domain](../tmp/tests/camera/20261005-163000-live-controls-complete/result.json)仅离线；[164800预最终字体综合门禁](../tmp/tests/desktop/20261005-164800-rc3-integrated-gates/result.json)仅其冻结范围。211424门禁已完整PASS：[Rust结果](../tmp/tests/desktop/20261005-211424-rc3-font-gates/rust-result.json)、[前端/typecheck/module结果](../tmp/tests/desktop/20261005-211424-rc3-font-gates/frontend-result.json)，260library＋2main＋50desktop、163frontend；它位于下面三项backend修复之前，不能代替修复后最终接口验收。

[174500后端API审计与修复](../tmp/tests/camera/20261005-174500-backend-api-audit/result.json)复现3项旧缺口后PASS：113camera（1ignored不计PASS）及20domain均仅离线。修复D161物理MF3写入只读一致性、focus静态i32/nonzero/±500 enqueue前拒绝且无op/Fault、LV active摄影Apply连empty/all-noop也前置拒而不停止LV；Read/List/Compare及专用LV configure不被错误禁用。

legacy `resolve_camera_capture_directory`已改为async鲜读引擎唯一savePath→CameraCaptures，非法/空/失联返回错误，无Pictures fallback/建目录/配置写；235531已实际核对。Operation.deviceSessionId实际关联engineSessionId，与camera stable32hex及每Start LV32hex分别核对，功能规范已修正。早期224517构建不能代替上面最终234619/235531实际host证据。

[165500 PnP](../tmp/tests/camera/20261005-165500-device-enumeration/result.json)当时无D7100、控制进程0；新LV实际硬件验收BLOCKED。[153200真实相机maintenance cleanup](../tmp/tests/camera/20261005-153200-connected-update-cleanup/result.json)限定engine7610、相机清理分项，非当前新增LV源码或安装器联动；Nano/Xray仍never-owned。完整射线联动、负载与重构精度未测。

Multi物理适用但不可观测UNKNOWN，当前profile5真实扫描BLOCKED是正确门禁；用户人工确认策略尚未答，无人工覆盖。线上GH发布未授权，Authenticode无证书，不伪造新包/未来stable升级证明。文档门禁为本地引用、格式及module map；本文件编写不连接设备、不编译、不安装、不打包。

## 2026-10-06 修订说明与本轮范围

本文件保留 2026-10-05 的历史结论，不追溯改写。以下两点按 2026-10-06 本轮实测修正，并以[后端重验证记录](backend-revalidation-2026-10-06.md)为准：

1. 原文中「`Operation.deviceSessionId` 实际关联 `engineSessionId`」的表述不精确。实测：相机发现/连接阶段回退 `engineSessionId`，相机业务操作使用当时的相机连接 session（例：`a027ef5ad3cf337e6924482fa59c9e18-op-3`），转台/安全/任务操作使用 `engineSessionId`。两种取值同时存在，不得统一强制相等。
2. 原文未涵盖本轮新增的实机缺陷：F-01（Live View 期间全局 Stop 导致相机 `cleanup_unconfirmed` 永久隔离且正常退出失败）、F-02（Apply 拒绝原因折叠为单一错误码）、F-03/F-04（未就绪的 capture/focus 在入队前不被拒绝，worker 内失败并断开会话）、F-05（descriptor 区间不等于机身接受的值域）、F-06（`retry_device camera` 产生 connect/completed receipt 但未连接）、F-07（失败的 LV 清理会把机身留在 Live View 并阻断重连）。逐条证据与修复状态见重验证记录第 4 节。

本轮仍未完成：独立 NEF 拍摄（需操作员将机身释放模式拨到单张）、F-01 修复的实机验收、HOME/MOVE、射线与完整扫描、软件更新维护锁。本文件不构成本轮实机验收完成声明。

## 2026-10-07 后续轮次（1–17）范围与指向

上面两节保留交接当时的事实，不追溯改写。**第 1–17 轮的完整验收、缺陷清单、修复与证据索引以[后端重验证记录](backend-revalidation-2026-10-06.md)（§5a–§5x）为准**；前端接口规范见[前端功能与接口规范](frontend-functional-spec.md)，逐码说明见[引擎错误码目录](frontend-error-codes.md)。

**当前交付候选**：host `2C423B80A1647158605174AE0D67C85FA4B2FE5CF64309033771941D7B891C5B`／内嵌引擎 `6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7`；正式入口未改动（`C19B828D8F18D13D3EF2811B95B5094863AEE0E494611140001BC69716C91334`）。离线门禁：`cargo test --workspace` **328 passed / 0 failed / 7 ignored**、Node **166**、typecheck、module-map 全绿。

**后续轮次已修复并验证的缺陷**：F-01（LV 期间全局 Stop 卡死＋退出挂起——已修，实机 947–1077 ms、`cleanupConfirmed=true`、可立即重连、退出 0/0）、F-02（Apply 拒绝码折叠）、F-03/F-04（拍摄/对焦门禁先于设备操作且不锁故障）、F-05=F-10（设备拒绝请求值不再毁会话，操作状态 `rejected`）、F-06（发现回执 kind）、F-07（connect 时自动清除机身残留 LV，清不掉才拒连）、F-08（重复 connect 不再锁故障丢会话，含空白 `deviceId`）、F-09（LV 控制事务 2 s 上限，避免阻塞 Stop 预停）。另修两处前端接口缺陷：`Operation.status` 新增 `rejected` 未同步到快照校验/等待器、`taskLock.retained` 类型（数组而非布尔）；并补齐 `camera.lastSeenAt`、帧 `capturedAtMs` 的 TS 声明。

**本轮范围（T0–T11）结论**：T1–T6、T8–T11 已在当前候选上取得实机证据（含 T2 错误信封、T6 LV 期拒绝路径、T9 恢复/重构/放弃拒绝路径、只读与断开类接口条目）；**T7 独立 NEF 仍 BLOCKED**，缺失条件是把机身释放模式拨盘拨到**单张**（实测 `driveMode=32785`、`nativeWritable=false`；两个可写 readiness 参数 Apply 后剩余阻塞只剩它与设计内的多重曝光 UNKNOWN）。射线参数/开束、HOME/MOVE、真实扫描仍 BLOCKED（无当前连接与出束授权；`multipleExposureEvidence=UNKNOWN` 按设计阻止扫描且源码中**不存在人工覆盖入口**）。软件更新维护锁仅有离线测试证据。本文件仍不构成"完整硬件可扫描"声明。
