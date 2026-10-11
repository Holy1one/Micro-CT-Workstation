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

后续批次（同一分支，全部先过门禁再提交）：

| 提交 | 覆盖 | 摘要 |
| --- | --- | --- |
| `cf98be5` | 测试夹具 | `ScriptedCameraProbe::set_card_events`：让扫描测试能脚本化卡关联事件（`NikonEvent` 是私有 `pub(crate)`，扫描侧无法构造） |
| `302231d` | ① | 固件在 `READY`/`INFO` 公布 `MOVE_SETTLE_150MS`，`nano.rs` 提供 `advertises_move_settle_150ms()`；150 ms 契约因此可从**连上的控制器**校验，而不是只信源码常量 |
| `e324d1e` | 文档 | `execution-flows.md` 新增 D7100 存卡采集流程章节 |
| `830e983` | ③ ④ | **B4c 扫描协调器存卡接线**：卡目标由冻结 profile 选择；启动即经 owner 线程枚举存储并以 `plan_task_cards` 在设备动作前拒绝；参考与投影走"匹配 READY → 意图入账 → 一次标准 `InitiateCapture` → 一 `ObjectAdded` + 一 `CaptureComplete` → 严格 `GetObjectInfo` receipt → 同 ticket `CAPTURE_DONE` → IDLE"；schema-4 写入器；终态 `AcquiredPendingImport`；域侧登记 `acquired_pending_import`；actor 增 `Request::CardPlan/CaptureCard` |
| `e9b876a` | ⑥ | 前端消费引擎新增的顶层 `cardAcquisition`（此前首启校验会**拒绝每一个快照**——门禁抓不到，因为前端测试用捕获的 fixture）；按既有附加字段先例改为"存在即严格校验、缺省不判损坏" |
| `27a1e79` | 文档 | 根与 ct-engine `AGENTS.md` 的"仅 RAM NEF"规则同步为"按冻结 profile 选择 RAM/卡"，并把"不得静默回退到存储卡"精确化为"不得静默回退到另一目标" |
| `131492e` | ③ ④ | B4c 最终增量：**目的感知清理**（取消的卡操作只由 `0x400d` 证明，永不删除或重拍卡对象）、账本闸门（意图无收据 → `CARD_ACQUISITION_UNRESOLVED` 故障，绝不报已采集；卡扫描拒绝 schema-3 本地续扫）、`card_profile_readback`/`plan_card_storages` 卡向入口、快照 `cardAcquisition` 接线 |
| `b7730a0` | 加固 | **防漂移门禁**：断言引擎固件身份常量 == `build-config.json` 固定候选，且固件源声明相同 version/build；防止"改了固件源却忘移引擎常量"把过时来源冻进任务证据 |

## 2 门禁与实测证据

- `npm run verify`（= module-map:check + typecheck + build + test:frontend + test:desktop + cargo workspace + firmware:test）在 B1/B2、B3、B4a、B4b、卡路径修正、⑤ 导入与实机就绪度修正各批均**退出码 0**；其中 module-map 为 **248 文件 / 16 模块 / 39 目录边界**（PASS 且图谱 current）。
- `cargo test --manifest-path backend/Cargo.toml --workspace`：最新一次为 **428 engine + 2 entry + 56 desktop 通过 / 9 ignored / 0 failed**。
- 前端：`test:frontend` **193 通过 / 0 失败**；`test:desktop` **27 通过 / 0 失败**；`typecheck` 无错误。
- 固件：`firmware:build` PASS，`flash=14196 / 30720`、`RAM=1476 / 2048`；`firmware:test` PASS，host harness **8443 断言**。
- 固件源身份（`firmware/rts9060_nano/build-config.json` 固定）：`rts9060_nano.ino` **34818 字节**，SHA-256 `2843f57ecb3d304c47f061f5d342b1b809a807dc2a7980ccc0cf9d14f0ecaa1e`，版本 2.2.0 / 构建 20261010；上一候选（2.1.1 / 20261009）SHA-256 `ecadf6220e53954cb303e0e40fb85710f73735ebe5271a127c8a4ba93bd84423` 作为历史保留，2026-10-11 已从实机读回为回滚备份。**改动固件源必须同步该固定身份，否则 `firmware:build`/`firmware:test` 会以 "candidate source identity mismatch" 失败。**
- 卡路径测试清单：`card_capture` 23 项（含单卡成功、Overflow、StoreRemoved、RAM 事件、重复/多余对象、未知事件、LV 活跃、`recordingMedia≠0`、会话变更、边界后多余事件、意图无收据）、`storage` 16 项、`task_registry` 14 项（含 3 项 `acquired_pending_import`）、`capture_profile` 16 项（含 5 项 schema-7）。
- 零图像载荷证据（脚本传输 opcode 日志）：触发后所有 opcode ∉ {`0x1009` GetObject, `0x101b` GetPartialObject, `0x100a` GetThumb}，从未发送 `0x90C0`，恰好一次 `InitiateCapture(0x100E, 0xFFFFFFFF, DataPhase::None)`，恰好一次 `GetObjectInfo(0x1008)` 且读上限 64 KiB，无 `TransferStarted`/`FileComplete`。

## 3 已记录的教训

存卡事务最初复用了 SDRAM 专用的 `profile_readback` 与 `confirm_runtime_diagnostic`（二者硬要求 `recordingMedia=1`），与卡目标（`=0`）无条件矛盾，使卡路径**永远不可达**，而 9 个纯状态机单测仍然全绿。因此本迁移的验收要求是"**端到端剧本测试 + opcode 级零图像载荷证据**"，不接受仅有纯单测通过；同类问题在后续改动中应优先用端到端证据排除。

## 4 尚未完成（离线）与 BLOCKED 边界

- 尚未完成（离线）：180/360/720 投影取整、无重复端点、末张不多转与暂停/Stop 窗口的专门用例（B4e）；真实浏览器 1920×1080 目视核对。
- 已由实机台架覆盖：转台连接与身份、D7100 发现/选择/能力协商/属性读回、固件 2.2.0 烧录与读回校验、卡策略固件门禁与目的感知就绪度；见[存卡迁移实机台架记录](card-sd-migration-hardware-2026-10-11.md)。
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

## 6 交付状态（截至代码尖端 `399aa55`；其后为文档提交）

全量 `npm run verify` 在最终尖端**退出码 0**：module-map PASS（248 文件 / 16 模块 / 39 边界）、typecheck、build、`test:frontend` 193 通过、`test:desktop` 27 通过、`cargo test --workspace` **428 engine + 2 entry + 56 desktop 通过 / 9 ignored / 0 failed**、`firmware:test` host harness **8443 断言**。

| 目标项 | 状态 | 说明 |
| --- | --- | --- |
| ① 固件 150 ms 末 STEP 下降沿握手 | **PASS（离线）** | 2.2.0；含 `MOVE_SETTLE_150MS` 能力令牌与 `advertises_move_settle_150ms()`；HOME/曲线/脉宽/方向/心跳/Stop/同 ID 释放不变；harness 覆盖末下降沿窗口、未满 150 ms STOP、零位移、单次 READY、micros/millis 回绕 |
| ② 扫描上限 1000 秒 | **PASS（离线）** | `scan::SCAN_MAX_XRAY_SEC` 统一十处判据；旧冻结值与手动 600 秒不变；前端 mm:ss 精确换算（1000 s = 16:40） |
| ③ 单卡/Overflow 存卡采集 | **PASS（离线，协议语义待实机）** | 能力分离、严格存储/卡对象解析、标准 `InitiateCapture` + 唯一元数据读取、零图像载荷（opcode 级证据）、目的感知清理与取消 |
| ④ schema 4 / profile 7 / 意图与 receipt / 状态与占用分离 / 崩溃窗口 | **PASS（离线）** | schema-4 清单与完成规则、`acquired_pending_import`、账本闸门（意图无收据即故障）、暂停/Stop/崩溃窗口测试 |
| ⑤ USB 批量导入与校验封存 | **PASS（离线）** | `card_import.rs`：owner 线程内枚举/下载、长度与 NEF 结构校验、流式 SHA-256、原子不覆盖封存、续导与取消、拒绝码（缺失/歧义/意外/不可读/超限/变更/身份变更/存储变更/截断/NEF 无效/目标冲突/路径逃逸/封存不一致）；域命令 `start_card_import`（带 `taskId`）与 `cancel_card_import`（无参）及引擎别名 |
| ⑥ 前端类型/快照/计数/状态/文案 | **PASS（离线）** | 类型与严格校验（兼容"基础 8 键"与"基础+8 导入键"两种形态）、状态词与三计数、SD 模式不取缩略图并提示 "Images are on the camera SD card"、导入对话框与进度/取消/续导入口、运行期禁用拍摄/LV/改参数/新扫描、重构三处准入拒绝；引擎已发布完整 16 键 `cardAcquisition` 并接受前端命令名。**未做**真实浏览器 1920×1080 目视核对 |
| 故障注入与 180/360/720 全链 | **BLOCKED（未实施）** | 投影取整/无重复端点/末张不多转的专门用例尚未补 |
| 实机台架（授权范围内） | **部分 PASS（实机）** | 转台 COM7 连接与身份、D7100 发现/选择/能力协商/属性读回、固件 2.2.0 烧录与读回校验、卡策略固件门禁 + 目的感知就绪度经生产引擎复核；**未运动、未拍摄、未出束**。详见[存卡迁移实机台架记录](card-sd-migration-hardware-2026-10-11.md) |

**FAIL：无。**

**引擎与前端对齐（已完成）**：命令名 `start_card_import`（带 `taskId`）/`cancel_card_import`（无参）
已在引擎侧提供（并保留 `camera_card_import`/`camera_card_import_cancel` 别名）；引擎一次性发布完整
16 键 `cardAcquisition`，前端同时兼容 8 键与 16 键。

**实机验收进度**：P2/P5/P6 的授权边界见第 4 节。2026-10-11 已获授权完成只读台架检查与固件
2.2.0 烧录（记录见实机台架文档）；转台运动与拍摄、以及全部射线相关项仍未授权、仍为 **BLOCKED**。
`hardwareCertified` 保持 false，本候选仍只能标记为**仅离线验证 + 部分实机台架检查**。

**已知需在交付说明中同时声明的事项**：卡清理等待仍复用"曝光 + 2 s"（SD 实测界限属 P2）；容量上界为 64 MiB/张的保守估算而非实测 NEF 尺寸；策略中的固件源哈希是候选 provenance，不证明控制器当前刷入的镜像；卡策略还要求控制器公布 `MOVE_SETTLE_150MS`，否则以 `CARD_FIRMWARE_UNVERIFIED` 拒绝。
