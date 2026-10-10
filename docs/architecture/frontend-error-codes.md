# 引擎错误码目录（前端呈现用）

日期：2026-10-07。来源：当前工作副本源码（`backend/ct-engine/src/**`、`backend/src-tauri/src/**`），以源码为唯一事实源。

本文件与 [前端基本功能与后端接口规范](frontend-functional-spec.md) 配套：规范讲命令、字段、会话与门禁；本文件讲**每个错误码的触发条件与前端应如何呈现**。

## 0. 范围与处理约定

1. 引擎共 **130** 个错误码字符串，分为三类：
   - **125 个命令拒绝码**：从 `Engine::command`/`device_command`/`command_core` 返回 `&'static str`，写入 `Response.error_code`（`backend/ct-engine/src/lib.rs:422-470`），由 `backend/src-tauri/src/engine_client.rs` 渲染为 `ENGINE:<CODE>`。前端在 `engine_command`/`engine_snapshot` 的 rejection 文本里看到它们。
   - **3 个仅 operation 失败串**：只出现在某个已受理操作的 `Operation.error`/`last_error`，**没有** `ENGINE:` 前缀，前端必须从 `operations[]` 读取。
   - **2 个仅 `#[cfg(test)]` 编译**：生产二进制中不存在，前端永远不会收到；单列于第 11 节。
2. **未知码必须原样显示**：前端不得把未在本目录或规范中出现的码归入成功、OFF、正常或安全；应显示原始码与引擎 `last_error` 文本，并记录到会话日志。本目录是枚举而非白名单——引擎新增码时前端仍应可用。
3. **`ENGINE:` 前缀只说明来源层**，不说明严重度。是否置故障、是否加锁、是否隔离、是否可重试，一律按本目录的「fault/锁」列判断，不按前缀或 HTTP 式状态码推断。
4. 呈现分类是**建议语义**，四类固定取值：`可恢复重试`（等占用结束或直接重试即可成功）、`需先修配置`（操作员必须先改设备/参数/路径/连接等前置条件）、`需恢复流程`（必须走 HOME/重新预检/恢复/放弃/重连等恢复路径）、`原样显示`（前端不可自愈，按原码显示并记录）。
5. 「面」列：`A`=命令拒绝；`B`=仅 operation 失败串；`A+B`=既可能是命令拒绝、也可能作为入队闭包的错误文本出现。

## 1. 数量总览与分组小计

| 分组 | 码数 | 面 A | 面 B（仅 operation） | 仅测试 |
|---|---|---|---|---|
| G1 · 协议信封与请求合法性 | 9 | 9 | 0 | 0 |
| G2 · 连接与占用/锁 | 18 | 18 | 0 | 0 |
| G3 · 相机（连接 / 参数 / Live View / 拍摄 / 预设） | 37 | 37 | 0 | 0 |
| G4 · 转台（Nano / HOME / 台架模式） | 10 | 10 | 0 | 0 |
| G5 · 射线与联锁 | 17 | 17 | 0 | 0 |
| G6 · 扫描准备与恢复 / 放弃 | 24 | 21 | 3 | 0 |
| G7 · 重构 | 5 | 5 | 0 | 0 |
| G8 · 软件更新与维护锁 | 5 | 5 | 0 | 0 |
| G9 · 任务登记与持久化 | 3 | 3 | 0 | 0 |
| G10 · 内部 / 测试专用（前端永不可见） | 2 | 0 | 0 | 2 |
| **合计** | **130** | **125** | **3** | **2** |

核对恒等式：命令拒绝 125 + 仅 operation 3 + 仅测试 2 = **130**；分组小计 9 + 18 + 37 + 10 + 17 + 24 + 5 + 5 + 3 + 2 = **130**。

前端呈现分类分布：`原样显示` 20、`可恢复重试` 19、`需恢复流程` 30、`需先修配置` 61。

## 2. 值拒绝（`Operation.status = "rejected"`）判定

`rejected` 是**操作状态**，不是错误码。判定在 `backend/ct-engine/src/device_controls.rs:177-195`，分类器在 `device_controls.rs:812-828`：

```text
value_rejected = !cleanup_unconfirmed && device_value_rejection_error(error)
device_value_rejection_error(error) =
    ( error 以 "Live View control application incomplete: " 开头
      或 以 "Camera parameter application partial: " 开头 )
    && error 含 "PTP response 0x201c"
    && error 不含任一传输/会话标记：
       "camera unavailable:" / "camera I/O failed:" / "camera timeout:" /
       "camera session isolated:" / "camera operation cancelled" /
       "camera busy:" / "camera capability unavailable:" / "camera safety check failed:"
```

满足时操作记为 `rejected`：**不置 `fault_latched`、不发起全局 Stop、不隔离会话**，但仍会 `preflight = false`（`device_controls.rs:188`），因此扫描必须重新预检。实测回归见 `device_controls.rs:1004-1040`。

**本目录的结论（与任务前提不同，需 Lead 注意）**：上述规则只匹配 `camera_apply_properties` 与 `camera_live_view_configure` 两个操作的 `Operation.error` 全文，要求其中含 `PTP response 0x201c`。**125 个命令拒绝码里没有任何一个本身会被判为 `rejected`**：命令拒绝在入队前发生，不产生 operation，因此也没有 `Operation.status`。前端不要期望用 `ENGINE:<CODE>` 判断值拒绝。

与之等价（甚至更早）的**命令级值拒绝**是 `CAMERA_PROPERTY_UNSUPPORTED_VALUE`（`properties.rs:279`）：它在入队前返回 `ENGINE:CAMERA_PROPERTY_UNSUPPORTED_VALUE`，同样不置 fault、不 Stop、不隔离。

## G1. 协议信封与请求合法性（9）

这些码由 `Engine::handle` 的信封校验或载荷类型校验产生，写入 `Response.error_code`，前端看到 `ENGINE:<CODE>`（`backend/ct-engine/src/lib.rs:422-470`）。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `COMMAND_PAYLOAD_MISMATCH` | 请求 `payload.type` 与 `command` 字面量不一致（`lib.rs:472-480`） | A | 是 | 无 | 原样显示 | `lib.rs:479` |
| `COMMAND_QUEUE_FULL` | 主循环命令通道（容量 256）已满，请求被回填该码后拒收（`lib.rs:427-428`、`main.rs:44`） | A | 是 | 无 | 可恢复重试 | `lib.rs:428、main.rs:44` |
| `DUPLICATE_REQUEST_ID` | 同一 `request_id` 在最近 256 条内重复（`lib.rs:436-437`） | A | 是 | 无 | 原样显示 | `lib.rs:437` |
| `ENGINE_SESSION_MISMATCH` | `engine_session_id` 与当前引擎会话不符且不是首次 snapshot 握手（`lib.rs:438-440`） | A | 是 | 无 | 需恢复流程 | `lib.rs:440` |
| `INVALID_ENVELOPE` | `request_id`/`timestamp` 为空、请求自带 `error_code`，或 JSONL 行无法解析为 2 MiB+64 KiB 内的信封（`lib.rs:429-433`、`main.rs:72`） | A | 是 | 无 | 原样显示 | `lib.rs:433` |
| `INVALID_PARAMETERS` | 命令载荷缺字段、类型不符或数值越界（`device_controls.rs:37,51,247,342,490,585,611,715,717,727,732`；`lib.rs:134-222,941,964,1015,1162,1733-1736`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:51、device_controls.rs:343、device_controls.rs:611 等 32 处` |
| `PROTOCOL_VERSION_MISMATCH` | `protocol_version` 不是当前 `PROTOCOL_VERSION`（`lib.rs:425-426`） | A | 是 | 无 | 原样显示 | `lib.rs:426` |
| `STALE_REQUEST` | `sequence` 未大于上一次已接受的序号（`lib.rs:434-435`） | A | 是 | 无 | 原样显示 | `lib.rs:435` |
| `UNKNOWN_COMMAND` | 命令名不在设备白名单、也不在领域/核心分发表内（`device_controls.rs:353,745`；`lib.rs:1901`） | A | 是 | 无 | 原样显示 | `device_controls.rs:353、device_controls.rs:745、lib.rs:1901` |

## G2. 连接与占用/锁（18）

锁与占用类拒绝：命令被接受前检查 `maintenance_lock`/`busy()`/`manual_lock`/持久任务锁；除 `DEVICE_CLEANUP_UNCONFIRMED`、`FAULT_RECOVERY_REQUIRED` 外都不改变安全状态。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `ALREADY_CONNECTED` | legacy `connect` 时已有 Nano 适配器（`device_controls.rs:259`；`lib.rs:892-893`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:259、lib.rs:893` |
| `DEVICE_CLEANUP_UNCONFIRMED` | 存在清理未确认的隔离会话，普通设备命令/维护准备被拒（`device_controls.rs:125,394`；`software_update.rs:23,64`） | A | 是 | fault+锁+隔离 | 需恢复流程 | `device_controls.rs:125、device_controls.rs:394、software_update.rs:23 等 4 处` |
| `DEVICE_CONTROL_LOCKED` | 扫描/清理/重构占用期间提交普通设备命令（`device_controls.rs:124,563`） | A | 是 | 锁 | 可恢复重试 | `device_controls.rs:124、device_controls.rs:563` |
| `DEVICE_OPERATION_ACTIVE` | 同一设备已有在飞操作，或全局并发已达 16（`operations.rs:31-32,51-52`） | A | 是 | 无 | 可恢复重试 | `operations.rs:32、operations.rs:52` |
| `FAULT_RECOVERY_REQUIRED` | 已锁存故障或处于 Fault 相位时提交需要安全恢复的命令（`device_controls.rs:318`；`lib.rs:1183,1310,1372,1398,1417,1622,1675`） | A | 是 | fault | 需恢复流程 | `device_controls.rs:318、lib.rs:1183、lib.rs:1310 等 8 处` |
| `INVALID_ADAPTER` | legacy `connect` 的 `adapter` 既不是 `developer_preview` 也不是 `real_hardware`（`device_controls.rs:257`；`lib.rs:930`） | A | 是 | 无 | 原样显示 | `device_controls.rs:257、lib.rs:930` |
| `NO_ACTIVE_TASK` | 要求存在活动任务的命令（放弃、完成登记）当前没有活动任务（`device_controls.rs:237`；`task_registry.rs:75,185`） | A+B | 是 | 无 | 需先修配置 | `device_controls.rs:237、task_registry.rs:75、task_registry.rs:185` |
| `NOT_CONNECTED` | `require_connected()` 失败：预检/启动/恢复前未建立连接（`lib.rs:1920-1921`） | A | 是 | 无 | 需先修配置 | `lib.rs:1921` |
| `OPERATION_CONTINUATION_CANCELLED` | `continue_job` 时该 receipt 已取消或状态不是 `completed`（`operations.rs:51-54`） | A | 是 | 无 | 需恢复流程 | `operations.rs:54` |
| `OPERATION_RECEIPT_MISSING` | `continue_job` 在历史记录中找不到该 `operation_id`（`operations.rs:53`） | A | 是 | 无 | 原样显示 | `operations.rs:53` |
| `PREFLIGHT_REQUIRED` | 未通过本次预检就请求启动/恢复/暂停恢复（`device_controls.rs:564,648,681`；`lib.rs:1180,1301,1366,1426`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:564、device_controls.rs:648、device_controls.rs:681 等 7 处` |
| `PREVIEW_ADAPTER_MISMATCH` | 生产模式下把 `connect` 解析到预览适配器分支（`lib.rs:927-928`） | A | 是 | 无 | 原样显示 | `lib.rs:928` |
| `PRODUCTION_LOCKED` | 生产模式请求 `connect{adapter:developer_preview}`（`device_controls.rs:257`；`lib.rs:929`） | A | 是 | 无 | 原样显示 | `device_controls.rs:257、lib.rs:929` |
| `RECONNECT_REQUIRED` | `retry_device{device:turntable}` 但引擎已无 Nano 代理（`lib.rs:1475-1490`） | A | 是 | 无 | 需恢复流程 | `lib.rs:1478、lib.rs:1486` |
| `RECOVERY_REQUIRED` | 任务处于 Stopped 相位，`restore_previous` 不再适用，必须走恢复流程（`lib.rs:1419-1420`） | A | 是 | 无 | 需恢复流程 | `lib.rs:1420` |
| `SCAN_ACTIVE` | `busy()` 为真（运行/暂停/收尾/停止/重构/占用）时提交受控命令（`device_controls.rs:609,647,680`；`lib.rs:938,961,1006,1177,1298,1363,1423,1470,1584,1619,1685,1692,1728`） | A | 是 | 锁 | 可恢复重试 | `device_controls.rs:609、device_controls.rs:647、device_controls.rs:680 等 16 处` |
| `SCAN_CLEANUP_PENDING` | 真实扫描线程仍存活时请求关束或标记停止（`lib.rs:589-590,616-617`） | A | 是 | 无 | 可恢复重试 | `lib.rs:590、lib.rs:617` |
| `UNFINISHED_TASK_LOCKED` | 持久未完成任务锁生效时改任务/参数/运动/换目录（`device_controls.rs:119,565,608`；`lib.rs:959`；`software_update.rs:21`；`task_registry.rs:59`） | A+B | 是 | 锁 | 需恢复流程 | `device_controls.rs:565、device_controls.rs:608、device_controls.rs:119 等 5 处` |

## G3. 相机（连接 / 参数 / Live View / 拍摄 / 预设）（37）

参数类拒绝码由 `devices/camera/properties.rs` 的 `PropertyRejection::code()` 表产生（`properties.rs:270-281`），域入口用 `camera.property_rejection(...).map_err(|r| r.code())` 上抛（`device_controls.rs:491`），**在入队前拒绝**：不产生 operation、不置 fault、不触发 Stop。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `CAMERA_ALREADY_CONNECTED` | 存在 `deviceId`（trim 后非空，`device_controls.rs:390`）且当前相机已连接且未隔离时的重复连接（`device_controls.rs:416-420`） | A | 是 | 无 | 可恢复重试 | `device_controls.rs:420` |
| `CAMERA_AUTOFOCUS_UNQUALIFIED` | `camera_autofocus` 入队前，worker 同源的自动对焦资格不成立（`device_controls.rs:517`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:517` |
| `CAMERA_BACKEND_UNAVAILABLE` | WPD 相机后端无法建立（`device_controls.rs:425`；`lib.rs:1023,1501`；测试路径 `device_controls.rs:427`） | A | 是 | 无 | 可恢复重试 | `device_controls.rs:425、lib.rs:1023、lib.rs:1501` |
| `CAMERA_CAPTURE_DIRECTORY_REQUIRES_ROOT` | `camera_capture` 传入的兼容 `directory` 不等于唯一根下的 `CameraCaptures`（`device_controls.rs:496`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:496` |
| `CAMERA_CAPTURE_ROOT_REQUIRED` | `CameraCaptures` 需要引擎保存根，但 `Parameters.savePath` 为空（`device_controls.rs:495,498`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:495、device_controls.rs:498` |
| `CAMERA_CAPTURE_UNQUALIFIED` | `camera_capture` 与 `camera_test_capture` 都走 `capture_readiness`，在入队前发现策略读回 / LV OFF / 已确认运行诊断不成立（`device_controls.rs:515`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:515` |
| `CAMERA_CONNECTION_FAILED` | `command_core` 预览路径下预检或 `retry_device{camera}` 自动连接 D7100 失败（`lib.rs:1024-1030,1500-1502`） | A | 是 | 无 | 可恢复重试 | `lib.rs:1030、lib.rs:1502` |
| `CAMERA_FOCUS_MODE_LOCKED` | 目标属性的对焦模式被当前 D061/镜头资格锁住（`properties.rs:277,390`） | A | 是 | 无 | 需先修配置 | `devices/camera/properties.rs:277` |
| `CAMERA_FOCUS_UNQUALIFIED` | `camera_focus` 入队前，镜头步进资格（active LV + focusStep 能力）不成立（`device_controls.rs:516`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:516` |
| `CAMERA_NOT_CONNECTED` | 需要相机会话的命令在无适配器时提交（`device_controls.rs:53,289,464,613,650,684,718,719,725,726,731`；`lib.rs:1322`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:53、device_controls.rs:289、device_controls.rs:464 等 12 处` |
| `CAMERA_NOT_READY` | 相机未连接或会话已隔离时提交相机命令（`device_controls.rs:476`） | A | 是 | 无 | 需恢复流程 | `device_controls.rs:476` |
| `CAMERA_PARAMETER_SESSION_UNAVAILABLE` | 参数校验时相机会话不可用（`properties.rs:273,320`） | A | 是 | 无 | 需恢复流程 | `devices/camera/properties.rs:273` |
| `CAMERA_PROPERTY_DEPENDENCY_UNMET` | 父属性依赖未满足（如模式/白平衡父项未确认，`properties.rs:280,422`） | A | 是 | 无 | 需先修配置 | `devices/camera/properties.rs:280` |
| `CAMERA_PROPERTY_UNDESCRIBED` | 相机未提供该属性的可验证描述，或 bracketing 描述不符 0xD0C0/UINT8（`properties.rs:276,382-384`） | A | 是 | 无 | 需先修配置 | `devices/camera/properties.rs:276` |
| `CAMERA_PROPERTY_UNSUPPORTED_VALUE` | 请求值不在当前 descriptor 支持值/范围内（`properties.rs:279,407`） | A | 是 | 无 | 需先修配置 | `devices/camera/properties.rs:279` |
| `CAMERA_PROPERTY_WRITE_UNSUPPORTED` | 相机未广告属性写入操作（`properties.rs:278,398`） | A | 是 | 无 | 需先修配置 | `devices/camera/properties.rs:278` |
| `CAMERA_SHUTTER_UNCONFIRMED` | 生产 `prepare_scan` 需要已确认的 native 快门读回（`device_controls.rs:54`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:54、device_controls.rs:54` |
| `CAPTURE_DIRECTORY_IS_SCAN_TASK` | 拍摄目录落在保留扫描目录/已知扫描根之下（`device_controls.rs:39,505`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:505、device_controls.rs:39` |
| `CAPTURE_DIRECTORY_UNVERIFIED` | 独立拍摄目录的祖先不可验证（`device_controls.rs:64`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:64` |
| `CAPTURE_NOT_REGISTERED` | `camera_capture_descriptor` 查询的 captureId 不在本次引擎生命周期登记内（`lib.rs:819`） | A | 是 | 无 | 原样显示 | `lib.rs:819` |
| `INVALID_CAMERA_FOCUS_STEP` | `camera_focus.step` 缺失/为零/非整数/超出 −500..=500 或 i32 范围（`device_controls.rs:478-479`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:478、device_controls.rs:479` |
| `INVALID_CAMERA_PARAMETER_COUNT` | Apply 的属性个数为 0 或超过 64（`properties.rs:274,326`） | A | 是 | 无 | 需先修配置 | `devices/camera/properties.rs:274` |
| `INVALID_CAPTURE_DIRECTORY` | 拍摄目录不是绝对路径或含 NUL（`device_controls.rs:38,58,503`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:38、device_controls.rs:58、device_controls.rs:503` |
| `INVALID_LIVE_VIEW_CONTROLS` | LV 专用配置为空、超过 2 个 key、key 不是 `cameraZoom`/`focusMode` 或值非严格 UINT8（`device_controls.rs:486-487`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:487、device_controls.rs:486、device_controls.rs:486` |
| `INVALID_LIVE_VIEW_SESSION` | `camera_live_view_read/configure` 的 `sessionId` 缺失或不是 32 位 hex（`device_controls.rs:482`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:482` |
| `LIVE_VIEW_APPLY_FORBIDDEN` | LV 激活期间提交普通摄影参数 Apply（含空对象/no-op，`properties.rs:275,323`） | A | 是 | 无 | 需先修配置 | `devices/camera/properties.rs:275` |
| `LIVE_VIEW_CAPTURE_FORBIDDEN` | LV 激活期间请求拍摄；恢复/启动的入队闭包内也复用该码（`device_controls.rs:489,655`） | A+B | 是 | 无 | 需先修配置 | `device_controls.rs:489` |
| `LV_NOT_ACTIVE` | 请求 LV 像素管道描述符时没有活动端点（`device_controls.rs:233`） | A | 是 | 无 | 可恢复重试 | `device_controls.rs:233` |
| `PRESET_DELETE_FAILED` | 预设删除计划无法建立（`device_controls.rs:715`） | A | 是 | 无 | 可恢复重试 | `device_controls.rs:715` |
| `PRESET_EXPORT_FAILED` | 预设导出 JSON 生成失败（`device_controls.rs:711`） | A | 是 | 无 | 可恢复重试 | `device_controls.rs:711` |
| `PRESET_IMPORT_INVALID` | 导入文件未通过 schema/字段/能力整文件校验（`device_controls.rs:728`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:728` |
| `PRESET_MODEL_MISMATCH` | 预设机型与当前相机机型不符（`device_controls.rs:720`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:720` |
| `PRESET_NOT_FOUND` | 指定的预设 id 不存在（`device_controls.rs:507,717,732`；`presets.rs:65,95`） | A+B | 是 | 无 | 需先修配置 | `device_controls.rs:507、device_controls.rs:717、device_controls.rs:732` |
| `PRESET_STORAGE_INVALID` | 预设存储无法编写新种子（`device_controls.rs:736`） | A | 是 | 无 | 需恢复流程 | `device_controls.rs:736` |
| `REAL_CAMERA_REQUIRED` | 预览模式下请求 `camera_test_capture`（`lib.rs:1615`） | A | 是 | 无 | 原样显示 | `lib.rs:1615` |
| `STALE_LIVE_VIEW_SESSION` | read/configure 的 `sessionId` 不是当前 active LV 会话（`device_controls.rs:483`） | A | 是 | 无 | 可恢复重试 | `device_controls.rs:483` |
| `UNSUPPORTED_CAMERA_PROPERTIES` | `preset_load` 载入草稿前的整组属性校验失败（`device_controls.rs:721`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:721` |

## G4. 转台（Nano / HOME / 台架模式）（10）

转台命令要求 Nano 独立连接；普通运动只沿规定方向，HOME 是唯一保留固件内部有界反向退让的例外。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `BENCH_CANNOT_OVERRIDE_XRAY_SESSION` | 台架模式启用时引擎已在本进程拥有过射线会话（`device_controls.rs:248-249`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:249` |
| `BENCH_MODE_XRAY_FORBIDDEN` | 台架隔离模式下请求开束（`device_controls.rs:317`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:317` |
| `HOME_REQUIRED` | 未建立有效 HOME/Hall 参考（`device_controls.rs:648,681`；`lib.rs:1304,1369,1429,1963`） | A+B | 是 | 无 | 需恢复流程 | `device_controls.rs:648、device_controls.rs:681、lib.rs:1304 等 6 处` |
| `HOME_REQUIRES_XRAY_OFF_OR_ISOLATED_BENCH` | 连接射线时请求 HOME 而 OFF 未读回确认且非隔离台架（`device_controls.rs:569`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:569` |
| `INVALID_DEVICE` | `retry_device` 的 `device` 不是 turntable/camera/xray（`device_controls.rs:305`；`lib.rs:1465-1467`） | A | 是 | 无 | 原样显示 | `device_controls.rs:305、lib.rs:1467、lib.rs:1465` |
| `INVALID_MOTION_ANGLE` | 角度非有限、越界、超过 3 位小数，或反向/整圈（`device_controls.rs:586-588`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:586、device_controls.rs:588` |
| `NANO_COMMUNICATION_FAILED` | Nano 串口通信失败或 XRAY_WARNING 关闭未被确认（`lib.rs:605-614,639`） | A | 是 | fault | 需恢复流程 | `lib.rs:639` |
| `NANO_NOT_CONNECTED` | 需要 Nano 的命令在无适配器时提交（`device_controls.rs:223,279,567,584,612,651,683`；`lib.rs:1121,1187,1321,1956`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:223、device_controls.rs:279、device_controls.rs:567 等 11 处` |
| `NANO_STATUS_UNKNOWN` | 需要新鲜 Nano STATUS 时 health 中没有状态（`lib.rs:1957`） | A | 是 | 无 | 需恢复流程 | `lib.rs:1957` |
| `PHYSICAL_ISOLATION_CONFIRMATION_REQUIRED` | 启用台架模式未显式确认射线物理隔离（`device_controls.rs:250`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:250` |

## G5. 射线与联锁（17）

射线设定与实测分离；所有开束入口经 Moxtek adapter，关束读回确认后 2 s 内不得再次开束。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `INVALID_CURRENT` | `send_current.ua` 缺失或不是数值（`device_controls.rs:332`；`lib.rs:1832`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:332、lib.rs:1832` |
| `INVALID_VOLTAGE` | `send_voltage.kv` 缺失或不是数值（`device_controls.rs:330`；`lib.rs:1763`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:330、lib.rs:1763` |
| `INVALID_XRAY_SETPOINT` | 电压/电流超出 4..70 kV、0..1000 µA 或功率越限的设定校验（`lib.rs:1764-1765,1833-1834,1909-1912`） | A | 是 | 无 | 需先修配置 | `lib.rs:1765、lib.rs:1834、lib.rs:1912` |
| `USB_AUTO_SHUTDOWN_RELEASE_REQUIRED` | 新任务预检需要本次会话已释放 USB 自动关断（`device_controls.rs:619`；`lib.rs:1087,1314`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:619、lib.rs:1087、lib.rs:1314` |
| `XRAY_ALREADY_CONNECTED` | 射线源已连接且 OFF 已确认时再次连接（`device_controls.rs:358`；`lib.rs:1517-1520`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:358、lib.rs:1520` |
| `XRAY_BEAM_ACTIVE` | 出束中请求改设定或执行需 OFF 的操作（`device_controls.rs:327`；`lib.rs:1591,1783,1855`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:327、lib.rs:1591、lib.rs:1783 等 4 处` |
| `XRAY_BEAM_ON_FAILED` | 预览开束路径失败（`lib.rs:1668`） | A | 是 | fault | 需恢复流程 | `lib.rs:1668` |
| `XRAY_CONFIG_FAILED` | USB 关断配置写失败（`lib.rs:1709,1758`） | A | 是 | 无 | 需恢复流程 | `lib.rs:1709、lib.rs:1758` |
| `XRAY_CONNECTION_FAILED` | 预检自动连接/`retry_device{xray}` 连接 Moxtek 失败（`lib.rs:1046-1053,1530-1537`） | A | 是 | 无 | 可恢复重试 | `lib.rs:1053、lib.rs:1537` |
| `XRAY_DISCONNECT_FAILED` | 断开时无法再次确认 Moxtek OFF（`lib.rs:860-867,1608`） | A | 是 | fault | 需恢复流程 | `lib.rs:867` |
| `XRAY_NOT_CONNECTED` | 需要射线源的命令在无适配器时提交（`device_controls.rs:311,319,321,335,340,614`；`lib.rs:1101,1326,1593,1625,1635,1697,1742,1788,1860`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:319、device_controls.rs:311、device_controls.rs:321 等 15 处` |
| `XRAY_OFF_UNCONFIRMED` | 关束未读回确认（`lib.rs:618-619,622-628`） | A | 是 | fault | 需恢复流程 | `lib.rs:619、lib.rs:628` |
| `XRAY_POWER_LIMIT` | 设定组合超过 12 W 上限（`lib.rs:1914-1915`） | A | 是 | 无 | 需先修配置 | `lib.rs:1915` |
| `XRAY_PREFLIGHT_FAILED` | 预检中射线健康（联锁/温度/设定）不满足（`lib.rs:1109,1119`） | A | 是 | fault | 需恢复流程 | `lib.rs:1109、lib.rs:1119` |
| `XRAY_SETPOINT_CONFIRMATION_REQUIRED` | 开束前设定值尚未本次会话确认（`device_controls.rs:320,620`；`lib.rs:1095,1642`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:320、device_controls.rs:620、lib.rs:1095 等 4 处` |
| `XRAY_SETPOINT_FAILED` | Moxtek 设定写入或读回失败（`lib.rs:1796,1868`） | A | 是 | 无 | 需恢复流程 | `lib.rs:1796、lib.rs:1868` |
| `XRAY_STAGE_LOCKED` | 生产模式请求 `timer_toggle`（`lib.rs:1680-1682`） | A | 是 | 无 | 原样显示 | `lib.rs:1682` |

## G6. 扫描准备与恢复 / 放弃（24）

`prepare_scan` 原子提交实测几何（必须 `measurement=measured` + `counterclockwise`），`maxXraySec` 1..=600；相机预检使用当前 profile 6 的原生参数读回。`camera safety check failed` 中的结构化要求显示为可读的相机参数问题，不分类为控制服务未响应。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `CAPTURE_PROFILE_REQUIRED` | 启动/恢复需要已冻结的采集 profile 而当前没有（`device_controls.rs:138,685`；`lib.rs:1346,1433`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:138、device_controls.rs:685、lib.rs:1346 等 4 处` |
| `EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY` | 生产模式经兼容入口 `update_scan_setup` 改曝光（`lib.rs:969`） | A | 是 | 无 | 需先修配置 | `lib.rs:969` |
| `EXPOSURE_NOT_NATIVE_CAMERA_VALUE` | 目标曝光无法匹配相机原生档位（`lib.rs:983`） | A | 是 | 无 | 需先修配置 | `lib.rs:983` |
| `EXPOSURE_OUTSIDE_CAMERA_RANGE` | 曝光超出相机上报的 min..max（`lib.rs:974-976`） | A | 是 | 无 | 需先修配置 | `lib.rs:976` |
| `INVALID_SCAN_GEOMETRY` | 几何非有限/非正、方向或来源非法，或非 measured+counterclockwise（`device_controls.rs:50`；`lib.rs:1218-1221`；`scan.rs:52-68`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:50、lib.rs:1221、lib.rs:1218 等 5 处` |
| `INVALID_SCAN_PREPARATION` | `prepare_scan` 载荷不满足 `deny_unknown_fields` 结构（`device_controls.rs:48`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:48` |
| `INVALID_SCAN_STAGE` | `confirm_scan_stage`/`cancel_scan_stage` 的 stage 不是 geometry/preReferences/placeSample（`lib.rs:1214,1228-1229,1236`） | A | 是 | 无 | 原样显示 | `lib.rs:1229、lib.rs:1214、lib.rs:1236` |
| `NOT_RESUMABLE` | `resume` 时相位不是 Paused，或预检/HOME 已失效（`lib.rs:1400-1401`） | A | 是 | 无 | 需恢复流程 | `lib.rs:1401` |
| `NOT_RUNNING` | `pause` 时相位不是 Running（`lib.rs:1383-1384`） | A | 是 | 无 | 可恢复重试 | `lib.rs:1384` |
| `RESERVED_OR_INVALID_TASK_ID` | taskId 命中保留名/非法字符/结尾点空格（`lib.rs:154-156`） | A | 是 | 无 | 需先修配置 | `lib.rs:156` |
| `RESERVED_TASK_DIRECTORY` | 任务目录落在 `CameraCaptures` 语义之下（`device_controls.rs:65`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:65` |
| `RESTORE_PREFLIGHT_CONFIRMATION_REQUIRED` | 恢复预检未同时给出 `restore:true` 与 `confirmUsbAutoShutdownRelease:true`（`device_controls.rs:616-617`） | A | 是 | 无 | 需恢复流程 | `device_controls.rs:617` |
| `RESTORE_VALIDATION_FAILED` | 已提交的恢复资料（哈希/几何/参数）校验失败（`lib.rs:1433-1438`） | A | 是 | 无 | 需恢复流程 | `lib.rs:1438` |
| `SCAN_GEOMETRY_REQUIRED` | 启动扫描时仍有未确认提示或缺少几何（`device_controls.rs:682`；`lib.rs:1307,1351`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:682、lib.rs:1307、lib.rs:1351` |
| `SCAN_OUTPUT_EXISTS` | 目标任务目录已有内容且不是同一参数的重复登记（`task_registry.rs:44`） | B（仅 operation 失败串） | 否 | 无 | 需先修配置 | `task_registry.rs:44` |
| `SCAN_OUTPUT_EXISTS_USE_NEW_TASK` | 预检/prepare 时任务目录已有内容（`device_controls.rs:60-61`；`lib.rs:1012`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:61、device_controls.rs:60、lib.rs:1012` |
| `SCAN_OUTPUT_RETAINED` | 已结束的 retained 任务目录仍存在 | A+B（restore/start 准入或登记失败） | 否 | 不新建未开始任务的锁 | 使用新 taskId，或删除整个旧目录后重试；根目录不可读另报 SCAN_OUTPUT_UNVERIFIED，现存数据不覆盖 | `device_controls.rs` / `task_registry.rs` |
| `SCAN_OUTPUT_UNVERIFIED` | 任务目录内容无法核验（读目录失败，`device_controls.rs:61,63`） | A | 是 | 无 | 需恢复流程 | `device_controls.rs:61、device_controls.rs:63` |
| `SCAN_PREPARATION_REQUIRED` | 非恢复路径下预检时缺少 `prepare_scan` 几何（`device_controls.rs:615-616`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:616` |
| `SCAN_PROMPT_MISMATCH` | 确认/取消的 stage 与引擎当前提示不一致（`lib.rs:1216,1231,1237`；`scan.rs:421`） | A | 是 | 无 | 原样显示 | `lib.rs:1216、lib.rs:1237、lib.rs:1231 等 4 处` |
| `SCAN_SHUTDOWN_TIMEOUT` | `command_core` 的 `disconnect` 路径等待真实扫描结束超过 70 s（`lib.rs:846-856`）；生产 `disconnect` 走 `enqueue_stop(true,true)`，不用该码 | A | 是 | fault | 需恢复流程 | `lib.rs:856` |
| `SCAN_STOP_FAILED` | 扫描 STOP 后相位仍为 Fault（`lib.rs:591`） | A | 是 | fault | 需恢复流程 | `lib.rs:591` |
| `STAGE_1_NANO_ONLY` | 生产模式请求兼容入口 `set_parameters`（`lib.rs:933-935`） | A | 是 | 无 | 原样显示 | `lib.rs:935` |
| `START_ADMISSION_INVALIDATED` | 入队后启动前参数/预检/HOME 被并发改变（`device_controls.rs:700`） | B（仅 operation 失败串） | 否 | 无 | 可恢复重试 | `device_controls.rs:700` |

## G7. 重构（5）

重构只读取已封存且哈希核对的扫描数据；方法可用性由 `available_methods()` 与预算决定，CGLS 保持禁用。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `INVALID_RECONSTRUCTION_METHOD` | method 不是 fdk/sirt/cgls（`device_controls.rs:666`；`lib.rs:1252-1256`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:666、lib.rs:1256` |
| `RECONSTRUCTION_ACTIVE` | 已有重构任务在运行（`lib.rs:1247-1251`） | A | 是 | 锁 | 可恢复重试 | `lib.rs:1251` |
| `RECONSTRUCTION_CACHE_INVALID` | 既有缓存版本/哈希校验失败（`lib.rs:1275-1281`） | A | 是 | 无 | 需恢复流程 | `lib.rs:1281` |
| `RECONSTRUCTION_LOCKED` | 占用中/任务锁未释放，或缺少已完成清单（`device_controls.rs:665,670`；`lib.rs:1248-1249,1268`） | A+B | 是 | 锁 | 可恢复重试 | `device_controls.rs:665、lib.rs:1249、lib.rs:1268` |
| `RECONSTRUCTION_METHOD_UNAVAILABLE` | 方法未启用、投影数 <3 或 SIRT 超出 CPU 预算（`device_controls.rs:667`；`lib.rs:1258-1264`） | A | 是 | 无 | 需先修配置 | `device_controls.rs:667、lib.rs:1259、lib.rs:1261 等 4 处` |

## G8. 软件更新与维护锁（5）

维护占用建立后除 snapshot/stop/off 与只读身份/状态外一律拒绝；`ready` 报告必须匹配本次 engine session/op。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `SOFTWARE_UPDATE_BUSY` | 存在活动操作/占用/未完成扫描或保持相位时请求维护准备（`software_update.rs:22,24`） | A | 是 | 锁 | 可恢复重试 | `software_update.rs:22、software_update.rs:24` |
| `SOFTWARE_UPDATE_LIVE_VIEW_ACTIVE` | Live View 激活时请求维护准备（`software_update.rs:25`） | A | 是 | 锁 | 需先修配置 | `software_update.rs:25` |
| `SOFTWARE_UPDATE_MAINTENANCE_LOCKED` | 维护占用生效后提交非豁免命令（`device_controls.rs:123`；`lib.rs:816`；`software_update.rs:20`） | A | 是 | 锁 | 可恢复重试 | `device_controls.rs:123、lib.rs:816、software_update.rs:20` |
| `SOFTWARE_UPDATE_NOT_PREPARED` | 未先建立维护准备就取消（`software_update.rs:60`） | A | 是 | 无 | 需先修配置 | `software_update.rs:60` |
| `SOFTWARE_UPDATE_OPERATION_MISMATCH` | 取消携带的 `operationId` 或会话与当前报告不符（`software_update.rs:61`） | A | 是 | 无 | 原样显示 | `software_update.rs:61` |

## G9. 任务登记与持久化（3）

任务登记是持久未完成锁的唯一事实源；写入前先预留内存锁，提交前比对期望文件内容。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `TASK_COMPLETION_PREPARATION_FAILED` | 完成登记计划无法建立（任务身份不匹配，`device_controls.rs:71`；`task_registry.rs:66-68`） | A | 是 | 无 | 需恢复流程 | `device_controls.rs:71` |
| `TASK_REGISTRY_INVALID` | 任务身份未验证通过，禁止放弃/完成（`device_controls.rs:238`） | A | 是 | 无 | 需恢复流程 | `device_controls.rs:238` |
| `TASK_REGISTRY_SAVE_FAILED` | 登记写入计划无法建立（校验失败或注册表已有错误，`device_controls.rs:137`；`task_registry.rs:56-59`） | A | 是 | 无 | 需恢复流程 | `device_controls.rs:137` |

## G10. 内部 / 测试专用（前端永不可见）（2）

这两个码只存在于 `#[cfg(test)]` 编译分支的测试注入路径（`device_controls.rs:361,427` 所在行由 `#[cfg(test)]` 属性覆盖），生产二进制中不存在，前端永远不会收到。

| 码 | 触发条件 | 面 | `ENGINE:<CODE>` | fault/锁 | 前端呈现 | 源码 |
|---|---|---|---|---|---|---|
| `CAMERA_TEST_FACTORY_REQUIRED` | 仅测试：未注入脚本化相机工厂（`device_controls.rs:427`，`#[cfg(test)]`） | B（仅 #[cfg(test)] 编译） | 否（前端不可见） | — | 原样显示 | `device_controls.rs:427` |
| `XRAY_TEST_FACTORY_FAILED` | 仅测试：脚本化射线工厂注入失败（`device_controls.rs:361`，`#[cfg(test)]`） | B（仅 #[cfg(test)] 编译） | 否（前端不可见） | — | 原样显示 | `device_controls.rs:361` |

## 11. 附录 A · 仅 `#[cfg(test)]` 编译（前端永不可见）

| 码 | 触发条件 | 源码 |
|---|---|---|
| `CAMERA_TEST_FACTORY_REQUIRED` | 仅测试：未注入脚本化相机工厂（`device_controls.rs:427`，`#[cfg(test)]`） | `device_controls.rs:427` |
| `XRAY_TEST_FACTORY_FAILED` | 仅测试：脚本化射线工厂注入失败（`device_controls.rs:361`，`#[cfg(test)]`） | `device_controls.rs:361` |

## 12. 附录 B · 常规生产启动不可达的命令拒绝码

这些码在生产二进制中存在，但需要非常规路径（预览分支、已连接射线源、维护/退避路径、关闭超时等）才可能出现。前端仍应能原样显示，不要因为它们「不会发生」而崩溃：

`CAMERA_CONNECTION_FAILED`、`PREVIEW_ADAPTER_MISMATCH`、`REAL_CAMERA_REQUIRED`、`RECONNECT_REQUIRED`、`RECONSTRUCTION_ACTIVE`、`RECONSTRUCTION_CACHE_INVALID`、`RECOVERY_REQUIRED`、`RESTORE_VALIDATION_FAILED`、`SCAN_SHUTDOWN_TIMEOUT`、`XRAY_BEAM_ON_FAILED`、`XRAY_CONFIG_FAILED`、`XRAY_CONNECTION_FAILED`、`XRAY_DISCONNECT_FAILED`、`XRAY_PREFLIGHT_FAILED`、`XRAY_SETPOINT_FAILED`

## 13. 重新计数方法（可复核）

叠层重构新增：

| 码 | 条件 | 操作建议 |
|---|---|---|
| `INVALID_SCINTILLATOR_STACK` | 几何中的叠层超过 5 层，或材料、发光颜色、厚度不合法；`scan.rs::ScanGeometry::validate` | 在 Scan Preparation 按入射顺序补齐；空行不算层；厚度须为正且不超过 100 mm |
| `INVALID_RECONSTRUCTION_OPTIONS` | 模式字段非法，多色模式同时指定单波段，单色模式指定材料标定，或自动颜色处理选择非 FDK；`device_controls.rs` / `lib.rs` | 选择 FDK 和 Monochrome/Multicolor；材料标定只在 Multicolor 中选择 |

1. 用 `recount.mjs` 以与库存报告不同的解析逻辑重扫 `backend/ct-engine/src/**`、`backend/src-tauri/src/**`：收集所有无空白、含下划线的大写字面量，且出现在 `Err("…")`、`Err("…".into())`、`ok_or("…")`、`map_err(|_|"…")`、`error_code=Some("…".into())`、`error_code:Some("…".into())`、信封 `Some("…")` 选择链、以及 `=> "…",` 码表分支中；再按「`#[cfg(test)] mod tests` 之后」或「紧邻 `#[cfg(test)]` 属性」把站点划为测试站点。
2. 只要一个码存在任何非 `Err_into` 的站点即为**命令拒绝码**；只有 `Err("…".into())` 站点的是**仅 operation 失败串**；只有测试站点的是**仅测试**。
3. 本次重算结果与库存报告 `inventory.json.errorCodes` 的集合差为空：总数 130、命令拒绝 125、仅 operation 3、仅测试 2（见 `recount-mine.json` 与本 run 的 `result.json`）。
