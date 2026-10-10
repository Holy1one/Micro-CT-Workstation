# D7100 SD 卡采集迁移 · 离线实施与验收证据（2026-10-11）

范围：`E:/LZU_Master/MicroCT/Micro-CT-App`，分支 `native-device-controls-offline`。
状态：**仅离线验证**。未连接相机、转台或射线源，未打开串口/USB，未拍摄、未运动、未出束，未上传固件，未替换系统已安装应用。
依据：[0004 规划](../decisions/0004-camera-sd-card-acquisition-plan.md)（第 1–12 节边界，第 13 节进度）。

## 1 已锁定提交

| 提交 | 覆盖 | 摘要 |
| --- | --- | --- |
| `e209ed1` | ① ② | 固件 2.2.0：普通移动统一 150 ms、从**末 STEP 下降沿**起算、消除 `finishPhase` 隐含尾周期，零位移同等待；`scan::SCAN_MAX_XRAY_SEC=1000` 统一十处判据，前端改 mm:ss 精确换算（1000 s = 16:40），旧冻结值与手动 600 秒不变 |
| `f5f6404` | ③ | `cardCapture/cardOverflow` 能力分离（缺广告即关闭并写原因）、`require_card_destination`（只认鲜读 `recordingMedia=0`，禁止回退 RAM）、`storage.rs` 严格存储解析 + `bind_storage_ids` 位置绑定、`ptp.rs` 严格卡对象解析、存卡事务骨架 |
| `ed7fcff` | ④ | schema-7 SD profile：`CaptureDestination`、`CardCapturePolicy`（150 ms 契约 + 固件 version/build/源哈希）、`scan_camera_issues_for(destination)`；`require_current_policy` 同时接受 6（RAM）与 7（卡），1–5 仍只读，旧 profile 哈希不变 |
| `17eec7b` | ④ | schema-4 卡清单：`CardCaptureRecord` 保留卡对象原始身份（无本地路径/哈希）、显式数据状态词表、`schema_four_completion_consistent` 要求**已封存导入**；`completed_manifest_exists` 接受 schema 4 |
| `e1cf968` | ③ 修正 | 修复存卡事务**不可达**：改用卡向冻结读回，触发仅一次标准 `InitiateCapture`，唯一数据读取是 64 KiB `GetObjectInfo` 元数据；23 项剧本测试含 opcode 级"零图像载荷"证明 |
| `8608971` | ④ | `acquired_pending_import` 版本化状态：安全收尾后离开 active 槽（释放扫描占用与手动锁）但目录保留不可复用；`mark_acquired_pending_import` 要求完整卡证据；`mark_import_verified` 只在清单已证明封存导入时封为 completed |
| `f7495c8` | ③ 修正 | `confirm_runtime_diagnostic_for` 目的感知：卡会话用 `recordingMedia=0` 确认 0xD1B2，RAM 路径委托为原值 1 不变 |
| `1910571` | 文档 | 0004 第 13 节实施进度记录 |
| `0171243` | ③ | 全任务卡容量闸门：`CONSERVATIVE_NEF_BYTES`(64 MiB)、`task_card_frames()`（N+20）、`plan_task_cards()`（设备动作前 checked 校验，`CARD_*` 稳定码） |
| `96e90b3` | 测试夹具 | `card_script`/`scripted_card_adapter` 让扫描协调器用同一 `CameraAdapter` 离线跑卡扫描；脚本传输新增 `0x1004/0x1005/0x1007` 臂与 `set_storages`/`set_object_handles`（仅测试、默认关闭） |

## 2 门禁与实测证据

- `npm run verify`（= module-map:check + typecheck + build + test:frontend + test:desktop + cargo workspace + firmware:test）在 B1/B2、B3、B4a、B4b、卡路径修正各批均**退出码 0**；其中 module-map 为 **245 文件 / 16 模块 / 39 目录边界**（PASS 且图谱 current）。
- `cargo test --manifest-path backend/Cargo.toml --workspace`：最新一次为 **402 engine + 2 entry + 56 desktop 通过 / 9 ignored / 0 failed**。
- 前端：`test:frontend` **193 通过 / 0 失败**；`test:desktop` **27 通过 / 0 失败**；`typecheck` 无错误。
- 固件：`firmware:build` PASS，`flash=14160 / 30720`、`RAM=1476 / 2048`；`firmware:test` PASS，host harness **8443 断言**。
- 固件源身份（`firmware/rts9060_nano/build-config.json` 固定）：`rts9060_nano.ino` **34782 字节**，SHA-256 `811bae5c1d6e08b318c81ac8d077908df127e00d1e95bf185cf090b5ad1069ec`，版本 2.2.0 / 构建 20261010；上一候选（2.1.1 / 20261009）SHA-256 `ecadf6220e53954cb303e0e40fb85710f73735ebe5271a127c8a4ba93bd84423` 作为历史保留。**改动固件源必须同步该固定身份，否则 `firmware:build`/`firmware:test` 会以 "candidate source identity mismatch" 失败。**
- 卡路径测试清单：`card_capture` 23 项（含单卡成功、Overflow、StoreRemoved、RAM 事件、重复/多余对象、未知事件、LV 活跃、`recordingMedia≠0`、会话变更、边界后多余事件、意图无收据）、`storage` 16 项、`task_registry` 14 项（含 3 项 `acquired_pending_import`）、`capture_profile` 16 项（含 5 项 schema-7）。
- 零图像载荷证据（脚本传输 opcode 日志）：触发后所有 opcode ∉ {`0x1009` GetObject, `0x101b` GetPartialObject, `0x100a` GetThumb}，从未发送 `0x90C0`，恰好一次 `InitiateCapture(0x100E, 0xFFFFFFFF, DataPhase::None)`，恰好一次 `GetObjectInfo(0x1008)` 且读上限 64 KiB，无 `TransferStarted`/`FileComplete`。

## 3 已记录的教训

存卡事务最初复用了 SDRAM 专用的 `profile_readback` 与 `confirm_runtime_diagnostic`（二者硬要求 `recordingMedia=1`），与卡目标（`=0`）无条件矛盾，使卡路径**永远不可达**，而 9 个纯状态机单测仍然全绿。因此本迁移的验收要求是"**端到端剧本测试 + opcode 级零图像载荷证据**"，不接受仅有纯单测通过；同类问题在后续改动中应优先用端到端证据排除。

## 4 尚未完成（离线）与 BLOCKED 边界

- 尚未完成：扫描协调器存卡接线（参考与投影同目标、拍摄意图预写、schema-4 写入器、卡任务默认 1000 秒、安全收尾后登记 `acquired_pending_import`）、USB 批量导入与校验封存、前端状态/计数/UI 与重构准入、文档与模块图谱终检。
- **BLOCKED（需另行授权的真实设备动作）**：
  1. P2 无射线台架：单卡/Overflow 触发、结束与续存证据；不同角度"末脉冲下降沿 → 150 ms → READY"的实测间隔与图像质量；Stop/心跳；不得对原卡做破坏性满卡试验。
  2. P5 射线与全链：1000 秒连续出束截止、温度全过程、OFF 证据、失联/联锁故障关束，以及采集至重构整链。
  3. P6 对照实测：500 ms/150 ms 固件与 RAM/SD 条件的 360 张持续采集、导入与数据就绪时间，区分等待/下载/休息三项收益。
- 未验证即不得声称：真实 D7100 `InitiateCapture`/`ObjectAdded`/`CaptureComplete`/`StoreFull` 语义与参数、真实卡对象内容、真实 WPD 传输边界、整机联动与实测提速。以上均保持 **BLOCKED**，且 `hardwareCertified` 保持 false。

## 5 复现命令

```powershell
npm.cmd run module-map:check
npm.cmd run typecheck
npm.cmd run build
npm.cmd run test:frontend
npm.cmd run test:desktop
cargo test --manifest-path backend/Cargo.toml --workspace
npm.cmd run firmware:build
npm.cmd run firmware:test
```
证据日志按批保存在 `.cache/tests/verify/20261010-batch*/`（缓存不提交）；固件构建与 harness 证据保存在 `.cache/tests/turntable/<run-id>/result.json`。
