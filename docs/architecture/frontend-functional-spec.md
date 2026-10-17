# 前端基本功能与后端接口规范

本文维护当前必须保留的功能、接口与安全语义；布局与尺寸规则见 frontend/AGENTS.md 和现行样式。源码字段和能力快照是事实源，不作为某次重设计的交接提示词。

## 1. 事实源与边界

优先读取[EngineCommand/EngineSnapshot](../../frontend/src/engine/types.ts)、[生产adapter](../../frontend/src/engine/tauriAdapter.ts)、[Rust DTO与领域入口](../../backend/ct-engine/src/lib.rs)、[独立设备命令](../../backend/ct-engine/src/device_controls.rs)、[Tauri invoke入口](../../backend/src-tauri/src/main.rs)。本文不能替代其字段校验。设备能力详见[相机矩阵](native-camera-capability-matrix.md)。

生产链：React → Tauri → versioned JSONL → Rust ct-engine。引擎唯一持有设备连接、安全状态、任务占用、扫描阶段和提交进度。前端仅保存输入草稿、显示选择和请求生命周期；不访问设备，不根据点击、计时或目标值自行宣布完成，不维护第二份扫描状态机。Tauri只转发设备领域命令及提供桌面能力。

检测到Tauri runtime即使用生产adapter，IPC失败不得回退浏览器模拟。developer_preview明确NO HARDWARE，不生产真实NEF/设备证据。未知、未连接、超时、失联、清理未确认不能显示为OFF、正常或安全。软件门禁不替代现场屏蔽、联锁、急停及操作规程。

当前版本从 package.json、backend/Cargo.toml 与 tauri.conf.json 核对；安装身份、实机分项和重构边界见[验证索引](../validation/README.md)。源码、离线测试、安装和硬件认证分别报告。


## 2. 请求、响应与完成

| 操作 | Tauri invoke及参数 | 返回 / 关联 |
| --- | --- | --- |
| 读取全状态 | `engine_snapshot`，无参数 | 完整EngineSnapshot，不是局部patch |
| 发送领域命令 | `engine_command {command: EngineCommand}` | 完整EngineSnapshot；command的type及字段见后面表格 |

引擎JSONL v2内部Request/Response包含`protocol_version/request_id/command/payload/timestamp/sequence/engine_session_id/error_code`，由EngineClient管理；renderer不自行拼协议或伪造会话。Tauri调用对象使用camelCase字段。`EngineCommand.type`必须与引擎命令一致；前端只提交当前契约字段，不添加额外字段或随意转换名称。这是调用约束：后端部分typed入口deny_unknown_fields，部分兼容Value入口可能忽略extra，不可宣称所有命令均拒绝未知字段，也不能依赖忽略extra取得新能力。

前端按`engineSessionId`、单调`snapshotSequence`接收最新快照。会话更换淘汰旧响应，低序号/迟到快照不能覆盖当前状态。长操作初次返回是受理，不是完成：保存该响应的`operationId`，继续读取同引擎会话的`operations[]`，核对相同`operationId`及`deviceSessionId`、设备、kind和terminal status。

HOME 与手动 MOVE 的界面等待上限为 360 秒，覆盖 Nano 的 300 秒运动时限及前后读回；其他普通设备操作保持 110 秒，打开扫描工程保持 300 秒。终态失败、清理未确认和会话改变仍立即结束等待并报告，只有关联的 completed 回执才能显示完成。界面等待不延长设备安全时限、不重发指令、不自动 Stop。

会话字段不能混为一谈：`EngineSnapshot.engineSessionId`是引擎生命周期opaque标识，不规定32hex格式。当前`Operation.deviceSessionId`由领域入队时确定：相机操作使用当时的`CameraHealth.sessionId`；发现/连接尚无相机会话时回退到engineSessionId；其它设备及安全、任务存储、维护操作使用engineSessionId。一处已知例外**：活动相机会话期间的 `camera_live_view_stop` 以设备标签 `camera_cleanup` 入队，因此其 `deviceSessionId` 回退为 engineSessionId（不是相机连接会话）——按 `deviceSessionId` 分组的前端逻辑必须容忍这一条。关联长操作应保存受理记录本身的deviceSessionId，不能要求所有记录都等于engineSessionId，也不能用后续重连产生的camera.sessionId替换旧记录。`CameraHealth.sessionId`及`CapturedImage.sessionId`是稳定相机连接/capture会话32hex；**`liveView.sessionId` 与 `liveViewControls.sessionId` 是同一个「每次 Start 新建」的 32hex LV nonce，二者必须相等**（不是两个独立 nonce），且独立于稳定连接会话。完整照片的captureId、operationId与相机操作receipt相互核对；LV像素核对自己的nonce，不跨层替换。

`DeviceOperation.status`为`queued/running/cancelling/completed/cancelled/failed/cleanup_unconfirmed/rejected`；字段包括`stage/cancelRequested/error/startedAt/updatedAt`。只有匹配完成记录和领域读回才能报告成功；cancelRequested不等于清理完成，cleanup_unconfirmed必须保持错误/锁。`rejected` 表示可修正的领域拒绝；相机参数拒绝、未建立射线会话的发现失败和扫描输出目录准入拒绝按各自 device/kind 呈现（见[错误码目录](frontend-error-codes.md)）：它是终态（前端等待器必须在此停止轮询），操作拒绝本身不新增设备故障，仍须保留快照已有故障和锁；相机参数拒绝展示逐键原因并允许按当前会话资格改正后重试。Stop优先绕过普通命令队列，使未执行/迟到普通请求失效。命令pending期间继续轮询，状态读取与Stop保持可用。

`DeviceOperation.kind`是操作种类：相机控制保留完整命令名（如camera_capture），连接/断开/HOME等可用connect/disconnect/home；不能用命令字面值强行匹配全部kind。`stage`是操作生命周期细节（如queued/device_io/registry_persistence/cleanup_requested/confirmed），不同于`workstation.scanFlow.stage`的扫描阶段。真正扫描提示的`prompt.stage`是geometry/preReferences/placeSample；前端只回传引擎当前提示，不凭操作stage推导下一扫描阶段。接口没有通用subtype字段，不能自己加一个来驱动后端。

**字段与请求精度**：

- `deviceControls.taskLock.retained` 是**任务记录数组** `RetainedTaskRecord[]`（每项 `{parameters:{taskId,savePath,projectionCount,angleStepDeg,exposureMs},status,registeredAt,updatedAt}`），**不是布尔**；空数组在 JS 里是真值，判断"是否有保留任务"必须用 `length`。`taskLock` 的键固定为 `locked/reason/taskId/savePath/status/retained`。
- **请求载荷约定**：① 缺 `command` 键或 `command.type` 非字符串 → 传输层报"missing required key command" / "Command type is required"；② 未知命令 → `ENGINE:UNKNOWN_COMMAND`；③ 未连接时读设备 → `ENGINE:NANO_NOT_CONNECTED` / `ENGINE:CAMERA_NOT_READY`（连接状态先于字段校验）；④ **额外未知字段被忽略**（前向兼容；不要指望拼错字段会报错——`camera_apply_properties` 的**属性键**是例外，会被严格拒绝）；⑤ `camera_connect` 的 `deviceId` **必须是字符串**：省略、`null`、空白串或非字符串类型都被视为"未显式选择设备"，结果是**发现刷新**（operation kind `camera_discover`，不连接、不报错、不锁故障）——前端必须按 kind 判断，不得把发现回执当连接成功；`acknowledgeUnconfirmedCleanup` 只在严格等于布尔 `true` 时生效。
- `deviceControls.camera.lastSeenAt`（string|null，相机 worker 最近可见时间，与 session/连接证据分开判断）与连接帧的 `capturedAtMs`（引擎侧采集时间；新鲜度仍以 session + sequence 判定）由引擎发送，**前端类型已声明并经判别测试固定**。
- `disabledReasons` 的**键集因实现而异**：生产引擎用 `hardwareCertification`/`capture`/`liveView`/`autofocus`/`focusStep`/`captureReady`（键存在即该项不可用，值为原因文本）；浏览器预览用 `scanNef`。前端不得按预览键集硬编码。
- 已声明接口是否仍有效以 Rust DTO、领域入口、Tauri 注册及生产消费方为准；当前 UI 没有调用点不能单独证明废弃，也不应将历史接口库存当作现行调用数量。
- 错误码、触发条件和呈现建议见[引擎错误码目录](frontend-error-codes.md)，具体拒绝与故障语义以当前入口和操作结果为准。未知码保留原文并记日志，按 failed/rejected/cleanup_unconfirmed 与实际故障锁分流，不静默吞掉或归为成功；rejected 是操作状态。

准备窗口提交需要自己的请求代次：同会话/任务转换才能接收prepare→preflight结果；提交中不可重复提交/关闭/跳其它参数入口，Stop仍可用。Stop或异常任务/会话更换使旧请求失效，迟到成功不得关闭新窗口或继续发送preflight。已提交原子参数不会因关窗口自动撤销。

桌面启动：桌面首次状态读取与命令等待引擎 bootstrap（最长 15 秒），不把正常启动误报成通信故障；真正启动失败或超时仍显示错误，已送达命令不重试。未连接设备属于初始状态。设备连接、相机固定配置、存储目录及射线参数的缺项在 Scan Preparation 中展示；提交后的进度、同次日志、未通过原因及修正建议保留在准备窗口。页面可用状态只是快照的只读展示，不替代引擎完整 Preflight；清理未确认和真实失联仍保留全局错误与领域锁。

## 3. 五组菜单与功能去向

这是现有[menuActions](../../frontend/src/menuActions.ts)的29项功能，不要求保持现菜单视觉。每个入口复用同一领域/平台函数，禁止另建草稿或设备状态。

| 组 | 功能ID | 功能去向 |
| --- | --- | --- |
| Task | `task.new`, `task.resetDraft` | 打开/重置新任务准备草稿；不动作、不清既有任务数据 |
| Task | `task.prepare` | 原子准备与Preflight；有保留锁时转受控恢复 |
| Task | `task.play` | 依据后端可用状态发送start_scan/pause/resume，不自行推进phase |
| Task | `task.stop` | 全局stop，保持易达 |
| Task | `task.restore`, `task.abandon` | 恢复冻结任务；或明确放弃并保留文件 |
| Task | `task.openFolder` | 打开当前引擎任务目录，不改imageRoot |
| Devices | `devices.camera`, `devices.cameraSettings` | 相机基本/Advanced功能；共用摄影草稿 |
| Devices | `devices.turntable`, `devices.turntableHome`, `devices.turntableBench` | 转台状态、HOME、射线物理隔离确认 |
| Devices | `devices.xray` | 射线连接、设定、实际状态与手动安全控制 |
| View | `view.equipment`, `view.result` | 已有设备/已验证重构结果显示，切换不发送运动/重构命令 |
| View | `view.photo`, `view.live` | 已登记独立照片/当前LV显示，不隐式捕获/启动 |
| View | `view.images`, `view.reconstruction`, `view.logs` | 已提交投影、重构功能、会话聚合日志 |
| Tools | `tools.deviceDiagnostics`, `tools.exportLog`, `tools.preferences` | 只读诊断、native保存日志、显示偏好 |
| Help | `help.userGuide`, `help.safetyNotes`, `help.about` | 操作说明、安全边界、实际软件版本 |
| Help | `help.checkUpdate` | 打开后立即检查最新稳定版本，有更新才提供下载；设备服务未就绪仍可打开平台功能 |

菜单可用性与相同功能按钮必须一致，展示后端拒绝原因。不存在文件/结果/已登记照片时禁对应查看；持续任务或操作占用时禁普通参数、对焦与运动，安全Stop/状态读取可用。菜单不得以隐藏或绕过后端检查提供“强制成功”。

## 4. 相机基本、Advanced与草稿

### 4.1 摄影参数

基本功能：显式发现/选择/连接，型号/状态，统一imageRoot，ISO/快门/光圈/白平衡/色温/Long exposure NR 六项编辑，模式/Auto ISO/对焦实际状态，Read、Apply、独立拍摄、LV启停及有证据的AF/步进。Long exposure NR 占色温右侧空位，固定扫描要求 Off，使用 Advanced 同一草稿、descriptor 和确认读回。Advanced展示全部当前摄影目录、依赖、预设及只读诊断。当前摄影目录34项，LV另2项，不声称全部厂商功能完备。

基本与Advanced共用camera连接会话关联的摄影草稿。首次确认读回可载入草稿，不写设备；后续外部变化不能静默覆盖用户草稿。更改父属性草稿不预测子属性已生效，必须明确Apply后重新描述/读回才解锁依赖。快门是扫描exposureMs唯一来源，按原生code/精确分数计算，不能另设可写扫描曝光或四舍五入。

`CameraProperty`保留`key/label/code/dataType/supported/readable/nativeWritable/effective/writable/disabledReason/options/supportedValues/range/actual/target/confirmed/error`。原生writable不等于当前可写。只有引擎允许的属性及选项可交互；不支持、只读、当前无效、未确认或占用时灰禁并给理由。枚举用下拉，option分别按supported/enabled禁用；真实数值range不能被少数标签缩窄。actual、target和未Apply草稿分开显示，null/confirmed=false/error表示未知。主相机面板的无值参数与未知只读值以主题主文字色的 `—` 占位，tooltip/可访问名称继续说明 Unknown；陈旧快照不展示旧值为当前读数，也不清除用户草稿。已确认的 0/Off 和显式草稿保持原值；连接、清理、定位参考及射线安全状态不使用这个占位规则。

Advanced必须能展示当前摄影目录的34个key，是否可写及有哪些选项仍取实际descriptor；分类只是展示建议，不规定布局：

| 功能组 | 原生属性key |
| --- | --- |
| 曝光与测光 | exposureMode、iso、autoIso、autoIsoHighLimit、shutter、aperture、exposureCompensation、meteringMode、centerWeightedArea |
| 白平衡与色彩 | whiteBalance、whiteBalanceFluorescent、whiteBalanceTemperature、colorSpace、pictureControl |
| 文件与采集 | rawFormat、rawBitDepth、rawCompression、crop、imageSize、driveMode、burstNumber、recordingMedia、focusMode、exposureDelay |
| 自动处理 | bracketing、activeDLighting、hdrMode、hdrExposureDifference、hdrSmoothing、longExposureNoiseReduction、highIsoNoiseReduction |
| 闪光设置 | flashMode、flashCompensation、flashSyncSpeed |

`runtimeProperties`是另外的只读原始诊断，包含设备实际广告的lensSort、liveViewSelector、activeSlot、batteryLevel、liveViewProhibitCondition、exposureDisplayStatus、exposureIndicateStatus、infoDisplayErrorStatus、exposureIndicateLightup、continuousShootingCount。未广告/失败显示UNKNOWN，`semanticStatus="unknown"`时不把原生0/1或PC屏显直接翻译成整体正常/故障，也不把连续拍摄计数当BurstNumber；这些字段不入摄影草稿或预设。连接/capture事件的诊断保留code/count/datasetSha256及本session/op，不把诊断存在当拍摄完成证据。

| 功能 | EngineCommand精确形状（均经engine_command） | 类别 / 成功条件 |
| --- | --- | --- |
| 发现/连接 | `{type:"camera_connect", deviceId?:string, acknowledgeUnconfirmedCleanup?:boolean}` | 无deviceId仅发现（operation kind `camera_discover`，完成即回执）：发现到 **0 或 ≥2** 台设备**不是错误**，回执仍为 completed，设备数只写引擎日志——前端必须按 `deviceControls.camera.discovered` 的长度决定提示（0 台=无设备、1 台=可显式选择、多台=必须选一台），不得把发现完成当连接成功，也不得依赖某个错误码。有明确deviceId才连接（kind `connect`），不隐式选择首台（`deviceId` 必须 **trim 后非空**才算显式选择，空白串等同未提供）。`acknowledgeUnconfirmedCleanup:true` 仅在「相机确实 isolated 且 cleanup 未确认」且锁原因确为 `Camera cleanup unconfirmed…` 时生效，且必须同时给出明确 deviceId：它会**退役**那台传输已关闭、永远无法确认清理的相机代理，让操作员重新选择设备，并释放该锁；**不会**清除跨设备共享的 `fault_latched`，也**不会**释放 Abandon 锁，且仍先判维护锁/busy。前端必须把它做成需要明确确认的危险动作，并在确认后说明旧会话终止证据仍为未知。**重复连接已有会话**（带 deviceId 调用 `camera_connect`，而当前会话 `connected && !isolated`）在锁/清理判定之后、任何入队与设备操作之前被拒绝，返回 **`CAMERA_ALREADY_CONNECTED`**：不锁故障、不隔离、不断开会话、不释放相机（原行为在实机上会锁 `fault` 并丢掉会话）。前端应把它当作可恢复的界面状态提示（禁用重复点击 + 明确提示"先断开再切换设备"），不得据此显示故障或要求重新预检；无 deviceId 的发现刷新在有会话时仍然允许 |
| 断开/读状态/身份 | `{type:"camera_disconnect"}` / `{type:"camera_status"}` / `{type:"camera_identity"}` | 断开需清理；只读身份不证明捕获资格 |
| 摄影Read | `{type:"camera_read_properties"}` | 只读，载入confirmed actual可由用户确认；不改变设备 |
| 摄影Apply | `{type:"camera_apply_properties",properties:Record<string,number|string>,presetId?:string}` | 明确写入+串行读回；只用原生数值/字符串；整请求先校验，部分失败保留真实/未知状态，不伪回滚。**设备拒绝请求值（部分 Apply）不锁故障、不断开会话**：该操作 `status` 为 **`rejected`**，`preflight` 失效，前端提示逐键原因并允许在同一会话内改正后重试（第 10 轮实机验证，见 §5 补充）。只有真正的传输/会话失败才按 `failed` + 故障处理 |
| 正常独立照片 | `{type:"camera_capture"}` | 有动作，不自动开束/AF/停LV；完整登记后才成功 |
| 隔离诊断捕获 | `{type:"camera_test_capture",directory?:string}` | 当前契约存在；不作为正常前端保存入口，目录权限/隔离由域校验 |
| LV启停 | `{type:"camera_live_view_start"}` / `{type:"camera_live_view_stop"}` | 动作与读回关联；inactive→Start生成新LV会话。`camera_live_view_stop` 是相机命名操作：若该相机操作进行中，Stop 会走有界（6 s）清理等待后按结果返回。前端应等待操作回执，不要重发 |
| AF/步进 | `{type:"camera_autofocus"}` / `{type:"camera_focus",step:number}` | 实际capabilities及领域有界校验，不能仅凭按钮或镜头类别证明可驱动 |
| 预设列表/导出 | `{type:"preset_list"}` / `{type:"preset_export"}` | 只读；export结果在deviceControls.exportJson |
| 加载/比较/删除 | `{type:"preset_load",id:string}` / `{type:"preset_compare",id:string}` / `{type:"preset_delete",id:string}` | Load只草稿，Compare只读；删除持久预设，非设备写 |
| 保存 | `{type:"preset_save",name:string,description?:string,source?:string}` | 保存当前已确认摄影值，不用未Apply草稿伪造已生效 |
| 导入 | `{type:"preset_import",jsonText:string}` | 全文件schema/model/字段/能力校验，不接受未知key |

预设只含相机配置，不含imageRoot/任务/射线/转台/绝对焦点坐标。Compare在supported/readable/confirmed且无error时才显示Match/Different；保留旧actual即使相等也应UNKNOWN。未连接镜头绝对焦点一直UNKNOWN，不能从step累加制造实测。

新后端静态参数门禁：`camera_focus.step`必须为i32范围内、非零整数且−500至500；缺失、零、小数、超范围在enqueue前拒绝，不产生操作receipt或Fault。D161的物理MF option3只读，UI选项与写入校验一致。LV active时普通camera_apply_properties在入队前一律拒绝，包含空对象或全部no-op，不停止LV；只读摄影Read/preset List/Compare与专用LV read/configure按各自能力保留。前端不能把no-op当作LV期间摄影Apply许可。

相机资格不足在入队前拒绝，不生成 operation 或触发全局故障：camera_capture/camera_test_capture → CAMERA_CAPTURE_UNQUALIFIED，camera_focus → CAMERA_FOCUS_UNQUALIFIED，camera_autofocus → CAMERA_AUTOFOCUS_UNQUALIFIED。真实传输与设备故障仍按故障路径处理。

主相机面板直接显示 Photo destination 与实际读回值。新连接会话在设备确实广告可写 SD 目标时将 `recordingMedia=0` 放入草稿，只在操作员明确 Apply 后写入并确认；Read/reload 保留实际值。继承 RAM 时引擎拒绝独立单拍与新扫描的下载；只有当前会话明确选择并 Apply Host RAM=1 才授权本地传输，值已经为 1 时也提交该明确选择。其他参数 Apply、Read/reload、重连和重启不授权。SD 下所有暗场、空场与投影统一存卡。旧 RAM 任务保留冻结内容，完成清理并明确放弃后才可创建新的 SD 任务。

独立单拍按实际目标选择门禁：RAM 使用 `capture_readiness()` / `disabledReasons.captureReady`；SD 使用 `card_capture_readiness()` / `disabledReasons.cardCaptureReady`，并要求 `capabilities.cardCapture`。SD 使用固定卡参数策略，成功只显示存卡结果，不下载预览、不需要照片根目录。两种门禁均覆盖无设备往返可判断的策略读回、LV OFF 和已确认只读的运行诊断；worker 仍鲜读参数、目标、LV、卡容量并校验事务证据，失败继续 fail closed。

就绪原因由 `deviceControls.camera.capabilities.disabledReasons` 发布。拍摄入口按已确认实际目标选 `captureReady` 或 `cardCaptureReady`，RAM 额外检查 `hostTransferOptIn`；原因键缺失仍需对应捕获能力、目标确认和单张数量成立。`hardwareCertification`、`capture`、`liveView`、`autofocus`、`focusStep` 及动态镜头键继续用于各自入口，不能用 RAM 的拒绝原因灰禁已就绪 SD。静态输入、占用、连接与 Live View 禁令仍先于拍摄门禁。

`camera_apply_properties` 的拒绝原因不再折叠为单一 `UNSUPPORTED_CAMERA_PROPERTIES`，前端应按以下码给出具体原因：`LIVE_VIEW_APPLY_FORBIDDEN`（LV 期间含空对象/no-op）、`INVALID_CAMERA_PARAMETER_COUNT`（空或 >64 项）、`CAMERA_PROPERTY_UNDESCRIBED`、`CAMERA_FOCUS_MODE_LOCKED`、`CAMERA_PROPERTY_WRITE_UNSUPPORTED`、`CAMERA_PROPERTY_UNSUPPORTED_VALUE`（不可写/值不在当前描述支持集）、`CAMERA_PROPERTY_DEPENDENCY_UNMET`（父参数依赖未满足）、`CAMERA_PARAMETER_SESSION_UNAVAILABLE`（仅当连接状态在同一次请求内发生竞态时才可能出现；稳态下连接问题先返回 `CAMERA_NOT_READY`）。

**拒绝码的实际顺序（按源码，前端排错时按此顺序理解）**：

0. 占用与清理锁（最外层，先于连接状态）：`SOFTWARE_UPDATE_MAINTENANCE_LOCKED`、`DEVICE_CONTROL_LOCKED`、`DEVICE_CLEANUP_UNCONFIRMED`、`UNFINISHED_TASK_LOCKED`。锁层拒绝时不会触碰设备；`acknowledgeUnconfirmedCleanup` 只对「相机确实 isolated 且锁原因确为 `Camera cleanup unconfirmed…`」且同时给出明确 `deviceId` 有效。
1. 连接/隔离状态：`CAMERA_NOT_READY` / `CAMERA_NOT_CONNECTED`（`camera_live_view_stop` 的空闲短路除外）。**它先于所有请求字段校验**——例如相机未连接时 `camera_focus{step:0}` 返回 `CAMERA_NOT_READY`，不是 `INVALID_CAMERA_FOCUS_STEP`。`CAMERA_ALREADY_CONNECTED` 也在此层之后、任何入队之前：已连接（未隔离）时带非空 `deviceId` 再发 `camera_connect` 会被拒绝，不锁故障、不隔离、不断开会话（无 `deviceId` 的发现刷新不受影响）。
2. 命令自身输入：`INVALID_CAMERA_FOCUS_STEP`、`INVALID_LIVE_VIEW_SESSION` / `STALE_LIVE_VIEW_SESSION`、`INVALID_LIVE_VIEW_CONTROLS`、`INVALID_PARAMETERS`、以及 apply 的属性分类码。
3. 拍摄的 Live View 禁令：`LIVE_VIEW_CAPTURE_FORBIDDEN`。**它先于拍摄目录校验**——LV 正在运行且 `savePath` 为空时 `camera_capture` 返回 `LIVE_VIEW_CAPTURE_FORBIDDEN`，不是 `CAMERA_CAPTURE_ROOT_REQUIRED`。
4. RAM 拍摄目录校验：`CAMERA_CAPTURE_ROOT_REQUIRED`、`CAMERA_CAPTURE_DIRECTORY_REQUIRES_ROOT`、`INVALID_CAPTURE_DIRECTORY`、`CAPTURE_DIRECTORY_IS_SCAN_TASK`；SD 单拍不创建或校验本地图像目录。
5. 预设 id：`PRESET_NOT_FOUND`。
6. 已发布 readiness：`CAMERA_CAPTURE_UNQUALIFIED`、`CAMERA_FOCUS_UNQUALIFIED`、`CAMERA_AUTOFOCUS_UNQUALIFIED` —— 最后一层，只在请求本身与目录都已合法后才可能返回。

同一请求可能同时不满足多项，前端只应显示引擎实际返回的那一个码；不要自己按"哪一项更重要"重新排序。

**Apply 部分失败的边界**：`camera_apply_properties` 中只要有一个属性被机身拒绝，操作以 **`rejected`** 结束并携带逐键原因（如 `Camera parameter application partial: {"whiteBalance": "camera command failed: PTP response 0x201c"}`）；`preflight` 被失效（fail-closed：不得用未验证的 profile 开始扫描），但**不锁 `fault_latched`、不触发全局 Stop、不隔离相机会话**，因此可以在同一会话内改正后重试或还原原值。前端仍应把"写值"当作需要判断的动作：写前保存本次鲜读原值、只写已确认过（`confirmed=true`）的值、并在 `rejected` 后明确展示被拒键与原因。只有真实的传输/会话失败才按 `failed` + 故障处理：**目前唯一计入 `rejected` 的 PTP 码是 `PTP response 0x201c`（InvalidDevicePropValue）**，且必须同时满足「文本以 `Live View control application incomplete: ` 或 `Camera parameter application partial: ` 开头」并「不含 `camera unavailable:` / `camera I/O failed:` / `camera timeout:` / `camera session isolated:` / `camera operation cancelled` / `camera busy:` / `camera capability unavailable:` / `camera safety check failed:` 任一标记」。其它任何 PTP 码（如 `0x2019`）与上述任一传输标记都仍然是 `failed` + 故障处置。历史故障只按原版本记录，不作为当前接口语义。

**重要边界二（F-05，已按方案 A 处置）**：`CameraProperty` 的 `range`/`options`/`supportedValues` 是**原生格式域**，不等于机身上实际接受的离散值集合；实测机身会拒绝区间内的值（PTP `0x201c`）。**处置结果**：该值拒绝现在记为 `rejected`（不是设备故障，会话保留、`preflight` 失效），前端据此提示逐键原因并允许在同一会话内改正后重试；决策记录与实机证据见[重验证记录 §5d/§5p](../validation/README.md)。前端仍应只提供已确认过的值并解释格式域与实测值域的差别。

**故障锁的清除路径**：`fault_latched` 是**跨设备共享**的全局锁，唯一清除点是**预检成功**（`preflight`）；在此之前 `xray_toggle` 一律返回 `FAULT_RECOVERY_REQUIRED`。`acknowledgeUnconfirmedCleanup` **不会**清除它（只退役那台不可用的相机代理并释放相机清理锁），相机再次掉线或清理未确认也会重新置位。因此前端不能把"恢复入口成功"当成"系统已无故障"。

`preset_load` 仍使用旧的粗粒度 `UNSUPPORTED_CAMERA_PROPERTIES`。仍须准备处理 `DEVICE_CLEANUP_UNCONFIRMED`。前端已通过 engine/native-labels.ts、deviceControls.ts 映射拒绝原因和处理建议。引擎仅在明确 deviceId、隔离且清理未确认的相机上接受 acknowledgeUnconfirmedCleanup:true；当前生产 UI 未提交该字段，不能把普通重连冒充已确认释放或绕过共享故障锁。

### 4.2 独立照片边界

SD 独立单拍只确认本次卡回执，不自动导入或显示本地预览；完整扫描的自动导入另走扫描收尾。以下文件登记与照片预览契约针对显式选择的 RAM 路径。

当前完整主机捕获仅D7100 RAM NEF。独立入口可用已确认M/P/A/S、自动ISO/WB及有证据AF模式，不隐式AF；只保证单个验证完整host NEF，未承诺一次物理曝光。多重曝光没有当前运行时状态字段或准入门禁；Scan Preparation 显示机身检查提示，不能把USB连接或菜单变灰解释为OFF。Live View开启期间严格拒捕获，不能自动停预览→拍摄→恢复。

90C0、一个与本操作关联的C101对象（C102完成可选：可早到、迟到或不到）、完整NEF长度/hash、秒级对象窗口、多对象拒绝、取消及原子不覆盖门禁由相机/引擎保持。`deviceControls.camera.lastCapturedImage`登记`captureId/sessionId/operationId/path/bytes/sha256/format:"NEF"/exposureEnded`，不能从临时路径或计时推断完成。JSONL保存图像证据，不传实时像素。登记是本次引擎生命周期内最多128项的有界集合，重启后或被淘汰的captureId不可据文件路径重新授权预览；原始NEF仍保留。

不开放未适配JPEG/TIFF/RAW+JPEG多对象、Bulb/Time、视频、9205区域AF或完整Picture Control编辑。未知型号不试发Nikon私有操作。

## 5. Live View控制和像素

相机稳定连接会话`camera.sessionId`用于身份/捕获；每次inactive→Start另生成32hex `camera.liveView.sessionId`。Stop/失联/隔离撤销LV会话，下一次Start换nonce；旧会话读写和迟到帧不能作用新会话。

| 功能 | 领域命令/IPC | 输出 / 门禁 |
| --- | --- | --- |
| LV Read | EngineCommand `{type:"camera_live_view_read",sessionId:string}` | 鲜读当前LV两项控制；返回health.liveViewControls |
| LV Configure | `{type:"camera_live_view_configure",sessionId:string,controls:Record<string,number|string>}` | 仅`cameraZoom`(D1A3)/`focusMode`(D061)，当前广告、原生整数descriptor（D1A3类型1–8且0–255；D061严格UINT8）、鲜读权限及同类型写后读回；拒其他key/任意setter |
| 当前LV metadata | EngineSnapshot `camera.liveView` | active/sessionId/sequence/lastFrameAt/fresh/width/height/fpsLimit/error |
| 当前LV controls | `camera.liveViewControls={sessionId:null|string,properties:Record<string,CameraProperty>}` | 与LV metadata/pipe session一致；不放摄影草稿、presets或固定profile |
| 像素拉取 | Tauri `camera_live_view_frame {afterSeq:number}` | `null`或`{sessionId,sequence,width,height,capturedAtMs,jpeg:number[]}`；renderer只能给afterSeq，不能给pipe/path |

普通摄影Apply仍禁止LV。D061有据目标0 AF-S/2 AF-F/4软件MF，模式3物理MF只读；实际枚举决定支持，不能用标签当编码。LV motor还需fresh lensSort(D0E1)=1、photo/movie selector(D1A6)=0、实际操作/DeviceReady与D061：mode0才AF；0/4允许有界step；AF-F2禁止有界motor，hardMF3禁写/motor。D1F2 activeSlot未广告时UNKNOWN。只读runtime保留nativeWritable证据，但不开放setter；lensSort1本身不是motor能力。

引擎到桌面为受控ACL named pipe二进制帧，renderer消费JPEG IPC。帧必须匹配当前LV nonce、sequence>afterSeq、合法JPEG与尺寸/大小限制；使用现有`validateLiveFrame`。active且暂未产生新鲜新帧时返回null；inactive调用报`ENGINE:LV_NOT_ACTIVE`，撤销/陈旧会话也明确报错，不能把所有错误当null。**瞬时管道错误要与上述三类区分**（第 18 轮实测：8 秒 32 次取帧中出现 1 次 `os error 233`／`ERROR_PIPE_NOT_CONNECTED`，随后立即恢复，LV 未中断、引擎无 fault）：遇到这类可自愈的管道读错误时，前端应保留最近解码画面并标 STALE、下一拍重试，**不得**据此断开 LV、重连相机或报设备故障；只有 `LV_NOT_ACTIVE`、nonce/会话不匹配、`validateLiveFrame` 拒绝这三类才是"该预览会话已失效"的终态。暂无帧/暂时错误保留最近解码内存画面并标STALE；新鲜匹配帧才LIVE。禁止应用保存LV像素、临时JPEG、帧日志或磁盘缓存；data URL保持有界当前/待退旧帧。Stop/断开/隔离/会话改变清引用、淘汰未完成decode。设备停止确认与回到Equipment分别表达，不把关闭画面当安全清理。

**Live View 停止清理**：Live View 开启期间的全局 Stop 原先会让相机进入 `cleanup_unconfirmed` 并被隔离（相机/连接命令一律 `DEVICE_CLEANUP_UNCONFIRMED`），正常退出也会挂起；根因是 Stop 先取消了相机会话，导致 `EndLiveView (0x9202)` 事务不再返回。现在 Stop 会在取消之前先用**有效会话**结束 Live View（有界 **5 s**，且**不因相机忙碌而跳过**），实机结果：Stop **947–1077 ms**、`cleanupConfirmed=true`、`phase=stopped`、无故障锁，随后 `camera_connect` 与 `camera_read_properties` **直接成功**，正常退出 **app 0 / engine 0**。前端仍需在 Stop 期间展示"正在安全停止"并等待引擎快照确认，不得自行推断完成；`acknowledgeUnconfirmedCleanup` 与 `DEVICE_CLEANUP_UNCONFIRMED` 保留为异常/旧会话的二线恢复路径。若机身被其它上位机留在 Live View，引擎在 connect 时会**先尝试自己清掉**（`force_live_view_off`）；**清不掉时仍拒绝连接**并报 `camera safety check failed: camera Live View could not be cleared in software (LV_STATUS=…)`，此时需要操作员检查机身（该自动清除成功的路径尚无实机执行证据，只能说"不需要为常规情形按键"）。

**Live View 控制超时**：Live View 的控制事务（`camera_live_view_read`/`camera_live_view_configure`）在单线程相机 worker 上有 **2 s** 上限（`LV_CONTROL_TIMEOUT`；实测正常读回仅 **66 ms**、LV 启动 192 ms，余量约 10–30 倍）。若某个 LV 控制操作在飞时立即按 Stop，修复前会让 end-LV 事务被堵住并在 **6.2 s** 后以 `cleanup_unconfirmed` + fault 收场；修复后同一序列 **815 ms** 干净停止。前端要求：**不要并发发送 LV 控制命令**，也不要在 LV 控制尚未完成时按 Stop（引擎已能处理，但会让 Stop 变慢）；LV 控制超时按故障级展示并等待引擎状态。

**可修正的参数拒绝**：设备**拒绝了请求值**（部分 Apply / LV 控制 apply 不完整，例如 `cameraZoom` 实测返回 `PTP 0x201c`）**不再**锁 `fault`、不再自动 Stop、也不断开会话；该操作的 `status` 为 **`rejected`**（新状态，区别于 `failed`），`lastError` 与聚合日志给出逐键原因，`preflight` 仍被失效（fail-closed：不允许用未验证的 profile 开始扫描）。前端应把 `rejected` 呈现为「可修正的拒绝」：保留会话、提示具体键与原因、允许在同一会话内改正后重试或还原，不要显示设备故障或要求重新连接。真正的传输/会话失败仍为 `failed` 且按故障处理。

## 6. 转台与射线

| 功能 | EngineCommand精确形状 | 类别 / 条件 |
| --- | --- | --- |
| 转台连接/断开/状态/身份 | `{type:"turntable_connect"}` / `turntable_disconnect` / `turntable_status` / `turntable_identity` | 各无其它字段；独立连接，不要求完整扫描参数 |
| HOME | `{type:"turntable_home"}`；扫描受控入口`{type:"home"}` | 有运动；连接射线必须OFF读回；不自动开束；HOME固件有界反向例外 |
| 正向相对/到phase | `{type:"turntable_relative",angleDeg:number}` / `{type:"turntable_to_angle",angleDeg:number}` | 正数沿规定方向，领域保持生产协议负符号；拒逆向/无界多圈；实际到位/ticket释放才完成 |
| 台架隔离 | `{type:"turntable_bench_mode",enabled:boolean,physicallyIsolated:boolean}` | 人工明确射线物理隔离；射线保持UNKNOWN/未监测且禁开束，曾连接OFF未知不能覆盖 |
| 射线连接/断开/状态/身份 | `xray_connect/xray_disconnect/xray_status/xray_identity` | 各仅type；独立连接，设定与实际分开 |
| 手动开关/优先OFF | `{type:"xray_toggle"}` / `{type:"xray_off"}` | 有辐射动作；OFF安全入口优先；未知读回不当OFF |
| 设定电压/电流 | `{type:"send_voltage",kv:number}` / `{type:"send_current",ua:number}` | 明确写+实际确认；不能仅显示目标为实测 |
| USB关断设置 | `{type:"usb_auto_shut_down_toggle"}` / `{type:"set_usb_shutdown_delay",delay:number}` | 领域校验；恢复所需release单独人工确认 |
| timer | `{type:"timer_toggle"}` | 仅浏览器预览兼容入口；当前生产明确拒绝XRAY_STAGE_LOCKED，不展示成可用的硬件定时器 |
| 历史连接兼容/重试 | `{type:"connect",adapter:"developer_preview"|"real_hardware"}` / `{type:"disconnect"}` / `{type:"retry_device",device:"turntable"|"camera"|"xray"}` | 生产connect只连接Nano，adapter=developer_preview拒绝；disconnect走全局清理。retry camera仅刷新发现，不隐式选择设备；其 operation kind 现为 `camera_discover`（不再是 `connect`，F-06 已修），因此前端不会把发现回执误当连接成功。无 deviceId 的 `camera_connect` 同样使用 `camera_discover`；只有带明确 deviceId 才是 `connect`。新前端使用分设备显式入口 |

位置读数消费`confirmedAngleDeg`和Nano status，不用目标角度冒充实测。未知或陈旧的位置显示 `— °` 并保留未知原因提示；`REFERENCE UNKNOWN` 仍明确显示，已确认零位保留数值。扫描/暂停/收尾/停止清理/重构及保留taskLock锁住手动参数/对焦/运动，受控恢复HOME例外。STOP、OFF及只读状态持续可用。

转台标题标签只表达连接（CONNECTED/DISCONNECTED；快照陈旧为UNKNOWN），motion state放在提示中；位置与REFERENCE UNKNOWN继续独立显示。普通Stop/故障清理可保留健康空闲设备会话，完成回执仍必须等待OFF/STOP及相机空闲确认；正在调用、曝光未决或清理失败的相机继续隔离。显式Disconnect、退出与更新维护关闭连接。连接保留不撤销持久任务锁、故障锁或重新预检/建立运动基准的要求。空闲相机会话保留路径目前仅离线验证，不继承此前关闭再连接的硬件验收。

转台栏固定高度不变：位置与按钮/运动控件各30px，状态行保持单行省略并保留完整 tooltip。底部只留一个入口：无保留任务时 Prepare scan / Preflight，有锁时 Resume / end retained task，后接进度条；不叠放重复恢复按钮。

电压/电流行按 SET 草稿 → 灰色 Applied → Measured 遥测呈现。Applied 使用设备已确认的 setKv/setUa，开束前可读；草稿变化、待确认、连接缺失或陈旧状态不能生成确认值。SEND V/I 的请求、设备确认结果和另一通道受 12 W 限制调整进入引擎聚合日志与射线日志。

所有开束经Moxtek adapter，每次确认OFF后至少2秒再ON，连接或UI计时不能绕过；Stop/故障/联锁/未知立即OFF。完整联锁、温度、设定与实际状态由引擎裁决，前端不计算另一份“安全”。

### 6.1 输入范围与单位

| 输入 | 当前生产校验 / 单位 |
| --- | --- |
| camera属性 | 按当前原生descriptor的dataType、枚举或`range=[min,max,step]`校验；传原生整数/字符串，不传显示标签，不能把色温index当Kelvin或快门原生分数当毫秒 |
| 扫描曝光 | 确认快门派生毫秒，当前有界主机采集0.125–30000ms；`cameraExposure.known=false`时默认范围只用于展示，不证明快门已确认 |
| `camera_focus.step` | 非零整数−500至500；方向/量是镜头相对操作，不是绝对焦点读数；原生资格仍须满足 |
| `turntable_relative.angleDeg` | 大于0、至多360度；最多3位小数（毫度），实际按96000脉冲/圈量化；无有效HOME/reference不能运动 |
| `turntable_to_angle.angleDeg` | 0≤phase<360度，最多3位小数；只沿规定方向到相位，目标相位不等于已确认位置 |
| `send_voltage.kv` | 有限数值4–70kV；联动电流限幅维持≤12W，实际值须读回 |
| `send_current.ua` | 有限数值0–1000µA；联动电压限幅维持≤12W，实际值须读回 |
| `set_usb_shutdown_delay.delay` | 原生整数1–65535计数，单位与当前设备协议一致；不擅自按UI秒数重解释 |
| 预设JSON | schemaVersion=1，UTF-8文件≤1MiB，最多128项；id≤96字节、name≤80、description≤1024、source≤256、models每项≤128且1–16项；整文件校验后才持久化 |
| LV JPEG | 4字节至4MiB，宽1–4096、高1–3072，合法完整JPEG；fpsLimit由后端提供，当前10，不作为曝光或计时证据 |
| 已登记照片预览 | captureId非空、≤256字节且无NUL；完整NEF≤128MiB且匹配登记长度/hash，提取JPEG≤16MiB |
| 重构预览 | `shape`三个轴各1–160；spacingMm三个有限正值，windowMax>windowMin；voxels为0–255字节数组，长度等于shape乘积且hash确认；不是完整科学体的原始浮点数组 |

前端可以提前提示这些静态范围，但实际能力、参数依赖、安全准入与最终读回仍由后端判定。错误的静态输入不应被夹成合法值后偷偷发送。

## 7. imageRoot、准备与扫描

`Parameters.savePath`是唯一图像根目录：`root/taskId/`原扫描组织；`root/CameraCaptures/`独立照片。通过`{type:"set_capture_root",savePath:string}`提交用户native picker选择；不另维护相机/扫描两根。新根失效预检，保留任务冻结路径不随意变更。taskId合法路径、保留名、别名/链接、已有manifest及不覆盖由引擎验证。正常照片不增加扫描帧数/manifest。

### 7.1 统一原子准备

```ts
{ type: "prepare_scan",
  taskId: string, projectionCount: number, maxXraySec: number,
  earlyRelease?: boolean, // default true; frozen at scan start
  geometry: { sodMm: number, objectToDetectorMm: number, detectorWidthMm: number,
    centerOffsetXMm: number, centerOffsetYMm: number,
    rotationDirection: "clockwise" | "counterclockwise",
    mirrorX: boolean, measurement: "measured" | "estimated" }
}
```

一次收集全部数据并原子提交；projectionCount整数1–3600，角步长由引擎360/N派生；maxXraySec为整数秒、范围1–1000（界面用 mm:ss 或十进制分钟收集并精确换算，默认600秒；旧任务沿用原冻结值，独立手动出束默认期限保持600秒）。通用几何DTO可表示两方向和真实/估计来源，用于读取既有资料；当前prepare_scan要求rotationDirection="counterclockwise"且measurement="measured"，estimated或clockwise明确拒绝。SOD、物体到探测器距离及屏宽必须有限且大于0；两轴偏移必须有限；mirrorX必须boolean。只读展示当前根、任务目录和确认原生快门，不能在此重复编辑快门。取消未提交草稿无文件/任务锁/动作。

prepare成功的同会话/同任务响应后调用`{type:"preflight",restore?:boolean,confirmUsbAutoShutdownRelease?:boolean}`。新任务不带恢复覆盖；恢复需明确本次confirmUsbAutoShutdownRelease。Preflight/Start/恢复鲜读比较，不写快门/RAM/burst，不自动停LV。自动配置、未知、漂移、未Apply均阻塞；展示`cameraScanReadiness.issues[]`的key/actual/requirement/remedy并允许转Advanced，关闭提示不解除门禁。相机预检按当前已确认参数逐项检查；界面使用英文参数标签、实际值、要求和建议呈现，原始错误保留在日志与 Copy details。高级设置左右栏目撑满标题和固定按钮区之间的可用高度。

SD 扫描的 `earlyRelease` 默认 true，Scan Preparation 的 “Move as soon as exposure ends” 允许关闭，快照 `workstation.scanSetup.earlyRelease` 回显引擎接受值。每张单独识别 D1A4 bit15 1→0；缺失时等本张唯一存卡回执持久化再释放，不等待一个必达的早事件、不沿用前帧证据。下一张仍必须等前张回执和本位点到位，关闭即使用串行存卡时序。该选项属于扫描调度，扫描开始后冻结，不改变摄影 profile、回执或 manifest schema；环境验收标志不再控制生产扫描选择。

完整 SD 扫描安全收尾并登记 acquired 后自动启动现有 USB 卡导入，下载前再次确认 X-ray OFF。拍摄阶段仍零图像载荷；投影导入 `frames/frame-NNNN.nef`，暗场与空场导入 `references/pre-dark|pre-flat/frame-NNNN.nef`。全部本地文件完整性/hash 和封存通过后，只删除本次回执对应的卡图片并确认不存在，随后显示流程 Completed；已有卡文件保留。导入期间显示传输，清卡阶段显示 Verifying and removing this scan’s SD images；失败显示原错误，不伪报已清卡。Stop 优先取消；清卡失败保持本地 verified 清单及剩余卡对象、待处理任务，现有 Card import 入口跨重启重试清卡，不重复下载/拍摄。相机常驻面板保持三行双列：第三行 Color temperature / Photo destination，Long exposure NR 在 Advanced 设置中保留。运行状态栏减少顶部留白，给卡状态、统计及完整进度条留足间距。

当前固定摄影策略适用于 RAM profile 6 与 SD profile 7：M、Auto ISO OFF、MF、固定 WB、单 NEF、严格 C0 OFF、ADL OFF4、HDR OFF 或仅未广告时的 NEF 互斥证明。采集目标分别为 RAM/SD；SD 使用独立卡策略及 schema 4。旧 profile 1–5 保持原 hash 和只读重构能力，不续扫；当前任务按实际冻结的 profile 验证。

### 7.2 扫描事务和控制

| 功能 | EngineCommand | 前端应等待/显示 |
| --- | --- | --- |
| 启动 | `{type:"start_scan"}` | 真实预检/设备/冻结证据；Start拒绝不创建假任务锁 |
| 现场阶段确认 | `{type:"confirm_scan_stage",stage:"geometry"\|"preReferences"\|"placeSample",geometry?:ReconstructionGeometry}` | 只确认引擎当前prompt；新任务几何已prepare，不造第二套窗口。注意 `stage="geometry"` 时 **`geometry` 是必填**（缺失即 `INVALID_SCAN_GEOMETRY`）；正常 `prepare_scan` 流程总会写入几何，因此该 prompt 主要出现在旧数据/异常路径 |
| 阶段取消 | `{type:"cancel_scan_stage",stage:ScanPromptStage}` | 引擎安全收尾/断点；Esc/关闭提示走此命令，不本地推进 |
| 暂停/继续 | `{type:"pause"}` / `{type:"resume"}` | 完整事务安全边界暂停；当前采集及持久任务锁保留，可 Resume |
| 全局Stop | `{type:"stop"}` | 并行相机取消/Nano STOP/Moxtek OFF；等待设备清理、扫描线程退出和持久终态登记后结束当前采集并解锁；保留所有文件，可准备新任务 |
| 恢复 | `{type:"restore_previous"}` | 冻结任务比较、必要受控HOME、已有图像hash核对 |
| 明确放弃 | `{type:"abandon_scan"}` | 清理确认后持久放弃并解锁，保留所有文件，不能标Completed |
| 兼容参数入口 | `{type:"set_parameters",parameters:ScanParameters}` / `{type:"update_scan_setup",setup:ScanSetupUpdate}` | 已有接口，受引擎占用/校验。**生产模式下 `set_parameters` 直接拒绝 `STAGE_1_NANO_ONLY`**（不是"可用但受限"）；`update_scan_setup` 携带 `exposureMs` 时拒绝 `EXPOSURE_EDIT_REQUIRES_CAMERA_APPLY`。新主流程使用完整 prepare_scan+set_capture_root+摄影Apply，不用兼容入口覆盖冻结任务或绕过准备 |

引擎流程：预检→必要HOME→Start→前参考提示→前暗场10张→一次连续开束前空场10张并确认OFF→装样提示→投影事务→封存。正常投影跨角度连续出束，最终/暂停按本会话关联ExposureEnded关束，完整文件/hash及匹配Nano ticket提交后才完成。前空场组超出连续上限不能为补照片反复短间隔开束。达到maxXraySec按既定300秒OFF休息策略继续尚未开始的投影；Stop/异常立即OFF。完成后不采后暗/后空、不额外HOME/整圈移动，原图保留。

`scanFlow.prompt/stage/references/geometry/checkpointPath`、phase/progress/imageCount、operations、workstation.dock可用性均由引擎提供。**`scanFlow.stage` 是闭集（11 值）**：`idle`/`preparing`/`prompt`/`reference_capture`/`ready`/`scanning`/`paused`/`persisting`/`persisting_completed`/`stopping`/`completed`；其中 `persisting_completed` 是"投影已持久化、尚未收尾"的中间态——**不得当作扫描完成**，完成仍以引擎 `phase`/`progress` 与任务提交为准。任务锁跨重启/taskId保留；不能本地清空锁/manifest就认为恢复或放弃成功。前端不能按拍摄定时器推进扫描或猜ExposureEnded。

## 8. 重构、媒体与日志IPC

`Task → Open Scan Project…` 通过 `open_scan_project {path}` 异步校验已封存项目。Reconstruction 控制区按 `task_project/open_project` 的可选 `progress={processedBytes,totalBytes,verifiedFiles,totalFiles,percent,estimatedRemainingSec}` 显示实际文件校验百分比、已核验文件数及剩余时间；总字节数尚未知时只显示准备条和估算提示。时间来自引擎实测读取速率，超过 10 秒向上取整到 10 秒，10 秒以内显示 **Finishing soon…**。取消、失联不继续显示完成预测；worker 进度最多 99%，只有引擎提交成功才确认 100% 并展示项目数量、启用重构。失败保留原项目。前端不以定时器推进百分比。

首次未连接设备的会话查看已确认历史项目时，Operation Status 显示中性的 **Offline project view · device states unknown**。判定要求同一引擎会话的成功打开回执、完全一致的封存投影数量、实测几何、10 暗/10 空参考，以及仅元数据/项目/重构准备回执；复用首次启动的全部设备未知、无故障和无占用检查，只排除已确认历史数据与重构字段。失联、过往设备会话、真实故障、活动或未知回执仍显示 Fault；图像数量不充当设备连接或射线 OFF 证据。

扫描准备中的 **Stacked scintillator scan** 勾选后展开最多 5 行，按射线入射方向填写材料、发光颜色和厚度；完整行数就是记录层数，空行忽略，半填行禁止 prepare。`ReconstructionGeometry.scintillatorLayers` 为可选字段并由引擎冻结，旧几何无需迁移。

重构入口提供 **Monochrome / Multicolor** 手动选择，`start_reconstruction.options={mode,materialCalibrationPath?}`。默认单色，波段由引擎按实测参考可靠性自动选择；多色按已记录层发光颜色选观测，当前颜色流程只启用 FDK。Multicolor 可选择原生 JSON 标定文件，内容由引擎校验；未标定只显示颜色响应。旧无 options 调用仍保留旧缓存和方法语义。

默认单色/多色重构沿用现有入口，由引擎自动执行颜色无关缺陷修补、暗空场平衡、去噪、轴校准与锥束 FDK，单色非材料模式再尝试通过独立验证门槛的 PWLS-Huber 粗修正。v3 自动预览按有符号 P1–P99.5 拉伸，科学体和原始数据不被显示操作修改。质量提示明确估计区域与未通过的修正，不要求操作员配置处理流水线；当前 task5 的质量限制见[自动重构记录](../validation/automatic-cbct-2026-10-15.md)。

`reconstruction_preview` 增加可选 `colors:number[]`（逐体素交错 RGB）、`colorLegend:string[]`、`materialLabels:boolean`、`mode`、`selectedBand`。桌面壳验证灰度及谱结果封存哈希、RGB 元数据/字节数/哈希，读完再次核对会话和 cachePath；前端在提交显示前校验所有字节。三切片和 3D 布局不变，MIP 显示灰度峰值处的颜色，避免分别取三个颜色的峰值合成虚假的材料组合。公共量化覆盖外仍显示灰度；图例区分颜色响应与已标定基材料。无 WebGL2 时关闭 3D，只显示状态。

| 功能 | invoke / 参数 | 返回 / 边界 |
| --- | --- | --- |
| 扫描投影预览 | `frame_preview {index:number}` | `number[]` JPEG，index必须是snapshot登记的已提交帧；后台复核NEF大小/hash；无任意path |
| 独立照片预览 | `camera_capture_preview {captureId:string}` | `{captureId,sessionId,operationId,jpeg:number[]}`，后台通过引擎descriptor查登记并前后重核会话/内容；无任意path |
| 内部登记读取 | EngineCommand `{type:"camera_capture_descriptor",captureId:string}` | `cameraCaptureDescriptor`；不是用户文件读能力 |
| 启动重构 | EngineCommand `{type:"start_reconstruction",method:"fdk"|"sirt"|"cgls"}` | 只用封存且hash核对NEF；方法enabled/reason决定可选，CGLS仍禁用 |
| 重构预览 | `reconstruction_preview {method:string}` | 只取已完成、引擎登记同method结果；shape/voxels/spacing/window等按现调用校验，不给cache path |
| 图像根默认 | `resolve_default_image_directory` | 当前用户默认路径，无动作；选择目录用native dialog再set_capture_root |
| legacy独立目录解析 | `resolve_camera_capture_directory`，无参数 | async鲜读engine_snapshot.parameters.savePath，仅派生root/CameraCaptures；空根/缺字段/非绝对根/失联返回错误，无Pictures回退、不建目录、不写配置、不授权拍摄 |
| 打开目录 | `open_directory_in_shell {path:string}` | 只对已有目录，路径来自任务/用户选定；不授予图片读取能力 |
| 导出日志 | `export_session_log {path:string,contents:string}` | native保存对话框选定.log/.txt，不用于图像/设备操作 |
| 预设文件picker | `camera_preset_import` | `string|null`，再经preset_import整文件域校验 |
| 预设导出picker | `camera_preset_export {contents:string}` | `string|null`保存结果，contents来自preset_export，不构造设备安全状态 |

照片保持至操作员关闭/返回；预览失败说明“文件已保存，预览不可用”，不伪造照片。重构首次计算时保留设备视图，验证完成预览后可切结果；Equipment/Result切换不重新计算。LV停止返回设备视图且清迟到帧。所有媒体切换用展示代次防旧响应抢回。

`chooseCameraCaptureDirectory`为当前UI无消费方的legacy诊断picker，不能建立正常capture的第二保存根；生产正常根只有native选择→set_capture_root。目录解析查询不能被当作已存在目录、写入成功或捕获授权证据。

日志按引擎稳定ID/时间去重，聚合system/xray/nano/camera/preflight/operator/reconstruction，保留Image列表和重构进度。重构worker是真实事件源，组件不能因轮询/useEffect重复造日志。完整错误保留，UNKNOWN、OFF未确认、安全故障不能被成功反馈覆盖。3D只读展示已确认姿态/体数据，不生成设备命令或实测几何。

## 9. 软件更新与维护

| 功能 | Tauri invoke精确参数 | 返回 / 类型 |
| --- | --- | --- |
| 状态/实际软件版本 | `software_update_status` | UpdateState.currentVersion，不硬编码包版本 |
| 检查最新版本 | `software_update_check {channel:"stable"}` | UpdateState；只发现新版本，不自动下载；网络失败不报告最新版 |
| 下载已发现更新 | `software_update_download {updateId:string}` | UpdateState；仅允许同一 available 操作，完整校验后进入 ready |
| 取消网络/验证 | `software_update_cancel {updateId:string}` | UpdateState；preparing/applying不通过此口取消设备清理 |
| 明确安装并退出 | `software_update_confirm_apply {updateId:string,expectedVersion:string}` | 匹配已验证缓存、分发kind；先权威维护停机再退出/helper |
| 领域维护准备/取消 | EngineCommand `{type:"prepare_software_update"}` / `{type:"cancel_software_update_prepare",operationId:string}` | 软件更新host自动消费领域报告；普通前端不可伪造ready |

UpdateState字段：updateId/phase/currentVersion/targetVersion/channel/kind/source/downloadedBytes/totalBytes/verified/notice/error。phase为idle/checking/no-update/available/downloading/verifying/ready/preparing/applying/cancelled/failed；kind=msi/nsis/development。成功notice是status，不是error；失败/旧回执/未完成不能变成功。检查最新稳定版本；有更新才提供下载，完成验证才提供安装；当前版本不低于最新版本显示已是最新版，查询失败保留错误和重试。正常菜单隐藏渠道、源地址和离线导入，安全验证继续由后端执行。

后端控制URL、公钥、kind和安装目标；renderer不能传任意下载源、信任key或target路径。公开HTTPS mirror→GitHub及离线签名包共同验证manifest/artifact原始签名、版本、目标、类型、大小/hash、防降级。当前仅交付Windows x64 MSI；升级保留现有安装位置，支持安装、卸载和Repair。历史升级和 Repair 只认证原包；当前构建、签名与安装身份按发行证据核对。用户明确要求安装后按既有授权执行；公开发布须有相应请求，不从历史说明推定授权。

安装前`softwareUpdatePreparation`必须匹配本engine session/op，ready/maintenanceLocked/noActiveTransactions及设备confirmed或明确never-owned。活动扫描/LV/拍摄/运动/重构/busy/保留任务锁拒安装。unknown或清理未确认维持锁。普通Stop使旧ready失效但不解维护锁；匹配cancel需清理确认，不自动重连、恢复LV、开束或放弃任务。权威报告→owned engine EOF0→App正常exit0→helper→安装/替换→真实新启动核对receipt，任一步不能仅用进程exit0替代安全报告。

## 10. 重设计验收原则

保留上述功能和精确IPC，单独验证读/写/未知/拒绝/迟到/取消路径。使用现有生产adapter和domain helpers；改类型须同步Rust/Tauri/preview及测试。仅界面重排不修改设备协议、扫描事务、安全门禁或原始数据。源代码边界与动作授权仍按仓库AGENTS执行。

真实硬件未验证的新增LV控制、镜头资格与完整扫描须明确待验；不能把离线fixture、模拟照片或旧版本实机结果升级为新版本认证。当前RAM/SD profile不含多重曝光的人工确认门禁。机身OFF检查是操作前置条件；提示不产生机器OFF证据，不覆盖引擎readiness，不写人工确认字段。一次物理曝光与C101/C102的对应关系仍需独立实测。

## 11. 失败呈现、Troubleshooting 与全英文呈现层

### 11.1 统一失败面（`frontend/src/FailureBanner.tsx` + `deriveFailureReport`）

- 唯一失败出口：`FailureBanner` 由 `App.tsx` 在正常与 boot 两条路径都渲染，portal 到 `body`（设计画布带 CSS `zoom`），层级高于所有页面与弹窗；层本身 `pointer-events: none`，安全 Stop 始终可点。
- 去重按**稳定 key + dismissed 集合**，不是清状态：同一个终端操作或同一条错误消息在 400 ms 轮询下只出现一次；新的尝试（`useDeviceActions.errorAttempt`）或新的 operationId 产生新 key，重新弹出。
- 判定来源优先级：最新终端操作（按 `snapshot.operationId` 匹配，避免历史操作复活）→ `actionError` → 引擎 `error`。
- **`cancelled` 本身不是失败**：Stop 与安全清理会主动取消在飞操作。只有当 cancelled 回执**带错误**时才升级为 `failed` 呈现，保证迟到/清理类真实失败仍可见。
- 关闭只隐藏通知：`cleanup_unconfirmed` 与 `transport` 两类保留明确文案，说明未确认清理、未清除故障或未完成任务锁。
- 未知错误码原样显示（`docs/architecture/frontend-error-codes.md` §0.2），不得改写成成功/OFF/正常/安全。
- 同时识别 `ENGINE:<CODE>` 和 operation.error 开头的原始 `CODE` / `CODE: detail`。SCAN_OUTPUT 类错误显示 Scan folder needs attention、实际 code 与 files 建议，不用相机拒绝或通用通信错误代替。目录不存在时后台撤销登记；目录仍在、读失败或有采集线程时保持相应占用。
- 关闭窗口等待引擎 EOF 清理；已经异常退出的引擎不再造成永久拦截，桌面保留失败返回码后退出。真实仍在运行的清理超时给出英文原生提示；更新仍要求严格 exit0，不把“进程不存在”当设备安全完成。

### 11.2 单一建议表与 Troubleshooting

`FAILURE_ADVICE`（`frontend/src/engine/deviceControls.ts`）是失败卡片 `nextAction`/`group` 与 Help → Troubleshooting 页面的同一份数据源。分组固定为 `camera` / `readiness` / `turntable` / `xray` / `files`；每组的 code 必须真实存在于 `docs/architecture/frontend-error-codes.md`，每条给出具体动作，不得出现「看日志」式空话，也不得承诺重启即可解决未知或未支持条件。`readiness` 组另附当前 session 的 `cameraScanReadiness.issues` 原样表格（key/actual/requirement/remedy）。

### 11.3 产品界面全英文（`frontend/src/engine/native-labels.ts`）

- 呈现在前端的英文层是唯一映射点：34 项 `property.label` 中 15 项、全部枚举 `options[].label`、`options[].reason`、`disabledReason` 与 `capabilities.disabledReasons` 目前由引擎返回中文，必须在呈现层映射；不得改 Rust，也不得渲染中文。
- 语言中立的引擎标签（ISO/EV/f/K/mm 等）原样保留；未命中的中文枚举降级为 `Native value {n}`；未命中的中文原因降级为一句中性英文并指向原始记录，绝不机器翻译。
- `property.label` 现由 `cameraPropertyLabel()` 统一提供英文名；`cameraPropertyUsesSelect` 仍只看原生 value/range，不受标签映射影响。
- 界面日志、失败详情、状态、提示与只读诊断全部使用英文呈现。已知原因映射为英文；未映射诊断以 `Unmapped diagnostic (raw Unicode)` 和可还原的 Unicode 转义保留细节。诊断 JSON（Camera → Advanced → Read-only diagnostics）将中文转义为 `\uXXXX`，解析后与原记录一致。错误码、`ENGINE:<CODE>`、引擎快照和 `Export Session Log` 的 `entry.message` 保持引擎原文。
- 日志 `source` 统一为引擎自身的字面量（system/preflight/operator/xray/nano/camera/reconstruction/devices）；preview adapter 不再翻译成中文，控制台标签按同一组字面量过滤。

### 11.4 弹窗尺寸与滚动面

- 扫描准备窗口（`ScanPreparationDialog.tsx`）与 Camera Advanced（`CameraSettingsDialog.tsx`）是**固定高度仪器**：header 与 footer 不动，`WorkstationDialog` 新增可选 `footer`/`footerNote` 槽；只有 `.scan-readiness-report__table`（readiness 报告）与 `.camera-advanced__pane`（参数面板）两个受界限的弹窗区域可滚动，`.modal-card__body` 仍可滚动。固定控制台栏目一律不得新增栏内滚动。
- Camera Advanced 为左侧 8 项导航 + 右侧单 pane：5 个摄影分组（9+5+10+7+3=34）+ `Other described parameters`（由实时 descriptor 编译，保证引擎新增属性不被隐藏）+ Presets + Read-only diagnostics。切换分组只改本地展示状态，不发命令、不 Apply、不丢草稿；基本栏与高级页共用同一 `model` 草稿与同一 Apply。
