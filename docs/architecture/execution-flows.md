# 执行逻辑

## 监测通信恢复

Moxtek 的 0x43/0x80 监测对共用 10 秒等待预算，保留碎片和同一未完成回复，不重发命令。50 ms 读取片段仍接受优先 OFF/Stop，绝对连续出束上限不延长。

仅监测读取超时且设备独立确认 OFF 才进入扫描线程持有的恢复流程：保留相机收据等待、串口连接、Nano 参考及已提交投影，不触发全局取消。相机曝光结束事件携带本进程单调时间；只有会话/操作匹配且结束早于故障关束开始，当前照片才可正常提交。关束可能截断曝光时保留文件和未决 intent，显示 Paused 并拒绝直接 Resume，不猜测关联、不补记进度、不重拍。

完整事务边界先持久保存断点，再读取两组间隔 500 ms 的新 OFF/设定/联锁/温度证据，并重读冻结相机参数和 Nano 静止参考。通过后投影从下一未提交帧继续；设备层仍执行至少 2 秒关束间隔。单次恢复最多复核 3 次，三次均失败但 OFF 可确认时保持暂停、等待 Resume；同轮第 4 次通信中断起要求操作员 Resume。空场组中断后同样要求 Resume 才能继续，避免自动循环开束。协议错误、真正设备失联、联锁、过温、设定改变、关束未知及连续出束硬上限仍走硬故障处置。

## SD 卡任务断点恢复

schema 4 / profile 7 的未完成采集保持原始卡回执，不用本地文件数代替已拍投影数。恢复校验冻结参数、几何、完整参考组、连续投影索引及每条 intent/receipt 的会话、操作和对象信息；未确认曝光、未知阶段、重复或额外账本记录均阻止恢复。通过同一相机 owner 鲜读卡内元数据，按存储、名称、长度、拍摄日期唯一绑定原对象，句柄可随重连变化。缺失、变化或额外对象拒绝恢复。预检在写入射线设定前执行此校验，Restore 和 Start 再复核；不下载或删除图像、不补写回执、不猜测未确认曝光结果。

清洁断点恢复保留前暗、前空场与已有投影，仅为剩余拍摄重新检查容量，从下一张未拍投影继续。转台参考仍按现场确认、当前控制器计数和既有参考恢复规则核验；停止、失联或复位不会自动保留位置资格。曝光均完成后的导入、校验、封存和精确清卡沿原路径执行。未确认曝光须有独立对账证据，普通恢复授权不构成补造记录的证据。

## 重新打开已完成的扫描项目

`Task → Open Scan Project…` 通过现有目录选择器选中含 `manifest.json` 的原项目文件夹，提交 `open_scan_project {path}`。ct-engine 的 `scan_project.rs` 在异步操作中使用重构的封存、几何、索引和 NEF SHA-256 校验，全部通过后一次更新项目参数、投影、参考图及重构就绪状态；失败或取消保留原项目。读取不写入扫描文件，不修改设备设定、位置或安全证据。未完成任务锁和维护锁继续生效。选择目录后前端立即显示 Reconstruction 控制区，按引擎 `task_project/open_project` 操作显示验证、取消或错误状态，验证期间不显示默认 0/0 或虚构百分比。关联操作完成后才显示引擎确认的投影和参考图数量，操作者选方法并开始离线重构；未加载项目时显示明确的菜单入口。

校验进度从 manifest 的前暗场、前空场和投影字节总量产生；SHA-256 每读取 64 KiB 检查取消，每 100 ms 或文件校验完成时上报。后台只写进度观察值，由引擎同步到关联操作的 `progress`，未提交时上限 99%。已校验文件数仅在长度/hash 都符合后增加。至少 250 ms 的读取样本才估算剩余秒数；前端向上取整到 10 秒，10 秒以内显示即将完成。元数据阶段无百分比/ETA，失败和取消清除 ETA，迟到进度不能修改终态。剩余时间是读取速率估算，会随存储速度变化；最终清单复核和唯一引擎提交仍决定完成。

## 桌面启动

1. Tauri 创建窗口和共享 `EngineClient`。
2. 后台线程校验并启动与应用同目录安装的 `ct-engine` sidecar，避免阻塞 Tauri `setup`；缺失或哈希不符提示修复安装，不向 C 盘缓存释放引擎。
3. `EngineClient` 校验 sidecar 二进制并建立 stdin/stdout JSONL 通道。
4. React 首次调用 `engine_snapshot`；sidecar 未就绪时返回明确错误，不切换到预览。
5. 操作者显式连接真实设备并运行预检；启动应用本身不触发真实运动或出束。

## 浏览器预览启动

1. `createEngineAdapter` 检测不到 Tauri runtime。
2. 创建 `WorkstationAdapter` 和内存 RTS9060 transport。
3. 预览生成与生产相同形状的 `EngineSnapshot`，但不得打开串口、调用 WPD/PTP 或接触真实设备。相机、转台显示为未连接；启动不自动 HOME或预检。明确连接、Apply和准备遵循相同领域门禁；模拟单拍不创建 NEF文件或 capture登记，维护模拟报告标在 developer_preview会话和 NO HARDWARE操作中，不能用作生产安全证据。
4. 一旦处于 Tauri runtime，任何 IPC 失败都直接报错；不得回退到预览。

## 命令执行

Live View 使用会话绑定控制命令：LV从inactive→Start创建新32hex nonce；`camera_live_view_read {sessionId}`只鲜读，`camera_live_view_configure {sessionId,controls}`只接受D1A3 `cameraZoom`和D061 `focusMode`两个已广告且有原生描述的控制：D1A3按当前标量整数类型1–8的宽度读写，值和描述范围保持0–255；D061仍严格UINT8。每次SET前重新确认当前会话/原生权限，写后读回；不能透传任意私有码。其会话与pipe/帧metadata相同，区别于稳定camera连接/capture会话；Stop后旧请求失效，迟到结果不能跨新LV会话提交。普通摄影Apply仍禁止LV，LV值不进入摄影preset或固定扫描profile。`camera_apply_properties` 的拒绝原因按 `PropertyRejection` 分码返回（`LIVE_VIEW_APPLY_FORBIDDEN`、`INVALID_CAMERA_PARAMETER_COUNT`、`CAMERA_PROPERTY_UNDESCRIBED`、`CAMERA_FOCUS_MODE_LOCKED`、`CAMERA_PROPERTY_WRITE_UNSUPPORTED`、`CAMERA_PROPERTY_UNSUPPORTED_VALUE`、`CAMERA_PROPERTY_DEPENDENCY_UNMET`、`CAMERA_PARAMETER_SESSION_UNAVAILABLE`），不再折叠为单一错误码，且都在入队前返回、不产生 operation、不置 Fault；`preset_load` 仍使用粗粒度的 `UNSUPPORTED_CAMERA_PROPERTIES`。

LV AF/step还须鲜读镜头/照片选择器和D061资格、实际操作广告及DeviceReady：lensSort=1、selector=0时模式0允许AF、0/4允许step，模式2 AF-F不允许有界motor，模式3物理MF禁止写/motor。LV未激活时`focus_step`能力恒为false，`camera_focus`按不入队拒绝处理；`camera_autofocus`在LV未激活时可依普通`focusMode` 0/1/2与镜头驱动广告执行，不隐式触发AF。未知或失败按禁用处理，不以lensSort单项推断资格。分项设备证据按构建记录，见[验证索引](../validation/README.md)；不能扩大为当前整机认证。

```text
operator gesture
  -> React dispatch(EngineCommand)
  -> Tauri invoke("engine_command")
  -> EngineClient request envelope
  -> ct-engine phase/safety validation
  -> device action or state transition
  -> complete EngineSnapshot
  -> React render
```

命令是意图，不是事实。例如 `send_voltage`提交电压设定命令；设备的设定读回与实测输出分别保存，设定值不能冒充实测电压。UI 不得根据按钮点击自行宣布设备已完成动作。

## 采集与离线重构

1. 操作员先在扫描准备窗口提交 `prepare_scan {taskId,projectionCount,maxXraySec,geometry}`：实测焦点至转轴、转轴至屏幕、有效屏幕宽度、X/Y中心偏移（mm）、水平镜像、固定逆时针方向和实测来源一并验证并原子提交。唯一保存根目录取 `Parameters.savePath`，曝光取已确认原生快门；取消窗口不写半套配置、不动作、不建任务锁。随后预检鲜读固定相机策略、射线与转台状态，必要时再明确受控 HOME；预检后无第二个几何窗口。
2. Start 后，引擎弹一次前参考提示；确认后顺序拍前暗场 10 张、只开束一次连续拍前空场 10 张，最后一张文件确认后关束。空场没有第二个确认弹窗；若单次出束上限不足以拍完整组，关束报故障，不逐张循环开束。
3. 引擎确认关束后弹装样提示；确认后执行投影事务。主机 RAM 路径确认本地 NEF 最终文件、长度/hash；默认 SD 路径持久化唯一卡回执，扫描中不传图像，见下节。
4. 末张投影及暂停按本次关联 ExposureEnded 关束；按采集目标完成文件或卡回执、匹配 ticket 的 CAPTURE_DONE、IDLE 和提交，确认警告输出关闭；SD 继续自动导入、封存与精确清卡，全部确认后完成流程。采集收尾不重复读取整批图像；SD 导入与清卡仍执行各自文件核验。不再拍后空场或后暗场。
5. 开始后的参考/装样采集提示取消、Esc 或关闭均发送 `cancel_scan_stage`。引擎负责安全结束和阶段断点；React 不自行推进阶段。恢复前仍需重新预检、必要的 HOME 与文件核对。
   已核对的 `projections` 断点直接进入下一张未提交投影，复用已完成的前暗场和前空场，不重新弹参考或装样提示。启动快照与扫描线程使用同一阶段。历史恢复错误写成 `placeSample` 且已有投影的清单，在冻结参数和全部文件校验通过后仅在内存归一为 `projections`；没有已提交投影的真实装样断点仍需确认，未完成的参考组只补拍缺失帧。
6. 封存后只能从底部 Reconstruction 区选择方法并打开重构预览；3D View 内不设重构入口。首次选择后引擎后台读取已校验的 NEF 和前暗场、前空场，执行可用方法并保存科学体、预览体和哈希。再次选择相同方法先核对缓存。Tauri `reconstruction_preview` 仅以方法 ID 读取引擎登记的预览缓存。

FDK 有 CPU 与真实 GPU 计算路径，首帧实测和数值比对通过且预计至少快 15% 才选 GPU；失败时全量 CPU 重算。小规模扫描可选 64³、五轮 CPU SIRT；超出计算预算时置灰。CGLS 无后端，保持不可选。体素间距是计算采样间距，不代表实测分辨率。这些数值路径的离线测试不证明 CT定量精度。当前安装、设备分项及重构限制见[验证索引](../validation/README.md)，旧包证据不认证当前整机。

## D7100 存卡采集（schema 4 / profile 7）

新会话草稿默认 SD，明确 Apply 并读回 `recordingMedia=0` 后暗场、空场、投影统一存卡。继承的 RAM 值不授权下载，预检和启动在暗场前拒绝（`CAMERA_SCAN_SD_REQUIRED`）；只有在当前相机会话明确选择并 Apply `recordingMedia=1` 才允许新 RAM 任务。Read、其他参数 Apply、重连和引擎重启不授予传输许可。已登记的旧 RAM 任务按原冻结 profile 恢复和校验，不改写旧数据或隐式转换目标；改用 SD 需完成清理并明确放弃旧任务，再创建新任务。

1. 预检按 `capture_profile::scan_camera_issues_for(health, CaptureDestination::CameraCard)` 校验：相机已连接、`capabilities.cardCapture` 已协商、Live View OFF、`recordingMedia=0` 鲜读确认。卡 profile 由 `CaptureProfile::from_card_readback` 生成（schema 7，含 `CardCapturePolicy`：single/overflow、150 ms 末下降沿契约、控制器固件 version/build/源哈希）。
2. 启动前在相机 owner 线程内枚举存储：`GetStorageIDs` → 逐 id `GetStorageInfo` → `bind_storage_ids` → `plan_camera_task_cards`（整任务 = 10 暗 + 10 空 + N 投影，按可用剩余字节和用户指定的 32 MiB/张规划预算校验；机身正数剩余张数保留为估算诊断，0 张卡不贡献容量，未知仍拒绝；预算不是文件大小上限，实际存储错误继续故障收尾）。D7100 机身存卡与 USB 写入权限分别判定；容量未知、容量不足或无法证明所需 Overflow 时，在**任何设备动作之前**以 `CARD_*` 稳定码拒绝；单卡足够即通过，缺第二张卡不当作容量。
3. 关闭早释放时每张投影：MOVE → 匹配 ticket 的 `READY_TO_CAPTURE` → 先持久化拍摄意图 → 相机一次标准 `InitiateCapture`（`0x100E`，参数 `[0,0]`，由机身选择卡／当前格式）→ 恰好一个 `ObjectAdded`（`0x4002`）对象与一个 `CaptureComplete`（`0x400D`）→ 严格 `GetObjectInfo` 解析并把 receipt 持久化 → 同 ticket 的 `CAPTURE_DONE` → IDLE → 下一位置。`StoreFull`（`0x400A`）只作为 Overflow 换槽过渡；`StoreRemoved`（`0x4005`）、RAM 事件、重复或多余对象、未知事件、会话变更都拒绝该张。扫描循环**不请求任何图像载荷**（无 `GetObject`/`GetPartialObject`/缩略图），不落本地文件、不产生哈希。
4. 参考帧（10 暗 + 10 空）走同一存卡事务，并同样计入 `captureRecords`。
5. 安全收尾后写 schema-4 清单（`captureDestination=cameraCard`、`acquisitionStatus=acquired`、`dataStatus=pendingImport`、`frames` 为空），并登记 `acquired_pending_import`：任务离开 active 槽（释放扫描占用与手动锁）但目录保留不可复用，且**不可重构**。
6. 完整扫描流程安全收尾后自动执行 USB 导入，下载前再次确认射线 OFF；复用现有导入逐对象下载、校验并封存，把每张 `captureRecords` 升级为 `verified` 并附本地文件身份，清单置 `dataStatus=verified`、`completed=true`，清卡全部确认后登记才转为 completed；重构与预览只接受已验证封存的数据状态（`scan::schema_four_completion_consistent`）。

占用与恢复：`acquired_pending_import` 与扫描硬件占用分离；导入未完整校验前绝不删除或重拍。所有本地文件验证并封存后，再次校验全部长度/hash，以独立 `card-cleanup.json` 记录逐对象删除意图和确认；同一相机 owner 用本次回执重新绑定的唯一对象，鲜读身份及 ObjectInfo 后仅发精确句柄 `DeleteObject [handle,0]`，随后重新枚举确认消失。已有卡文件保留，不格式化、不删除文件夹。全部清卡确认后才登记 completed。清卡失败或 Stop 保留已封存的 verified 清单、本地文件及 pending registry，现有导入入口跨重启重试剩余删除，已删对象仅凭持久删除意图和鲜读缺失证据核销，不重新下载或拍摄。未封存的导入失败保留原始卡对象及已验证文件。

清卡日志与任务登记共用持久元数据写入器：先以唯一临时文件写入并 sync，再原子替换。在 Windows 上，替换遇到错误 5/32/33 时以 20 ms 间隔重试同一替换，最多 1 秒；可取消的登记写入每轮检查取消。超时、永久权限错误及其他错误保留旧记录并返回具体失败阶段和路径。写入器不删除旧记录、不修改 ACL，未持久提交删除意图不得继续发送下一条 DeleteObject。

L1 早释放默认开启，Scan Preparation 的 earlyRelease 布尔选项允许关闭，扫描开始时固定并传入每张 CardCapturePlan。每张独立识别本 session/operation 的 D1A4 bit15 1→0，首次清零发 EarlyExposureEnded。非末张经共用 CAPTURE_DONE/IDLE helper 释放并用原 move_to_projection_monitored 预取下一位点，本张 receipt 持久化后才消费 ticket 并触发下一张，始终一张拍摄在飞。缺少早信号的该帧自动走存卡回执后释放，下一帧重新识别，不阻塞等早事件、不沿用前帧证据。重复事件无额外动作，末张无预取；失败保留未决 intent，不补拍、不置 acquired。Pause/Stop/到限/冷却复用既有监测逻辑。调度选项不改变摄影 profile、回执或 manifest schema；生产扫描不依赖环境开关。完成全部回执并安全关束后自动登记待导入，再次确认 OFF 后启动现有 USB 导入，逐文件校验封存后只清理本次卡对象，确认清卡才完成流程；下载失败保留卡原件，清卡失败或 Stop 保留本地封存及剩余卡对象供重试。当前不实现 L2 多张在飞；契约与证据见 [ADR 0004](../decisions/0004-camera-sd-card-acquisition.md)。

存卡事务在同一原生 owner／会话内复用已验证的描述；每张触发前与 receipt 返回前分别鲜读全部冻结参数、卡目标和 Live View OFF。默认 D1A4／D1B4／D1F1 三类常见显示通知只记录事件并将旧显示值标为待刷新，不在拍摄中重复查询；仍要求本会话广告的只读标量描述。D1B2 通知保留即时存储目标和诊断读回，其余目录通知仍鲜读对应属性。边界的存储目标已由冻结值鲜读证明，诊断确认不重复读取同一值；非空事件批次立即继续排空，只有空查询等 10 ms。摄影参数变化、DeviceInfoChanged 或未知事件拒绝当前操作。`lastCardCaptureTiming` 是单调时钟和真实 PTP 次数的只读诊断，不能当作曝光结束／存卡凭据。普通移动的 150 ms 仍由固件最后下降沿计起；2.3.0 加减速候选与 2.2.0 分别冻结精确固件来源，旧设备不冒充更快曲线。同一 ticket 的运动证据只读一次：串口 owner 在 `READY_TO_CAPTURE` 后读一份 `CAPTURE_HOLD` STATUS，`execute_plan` 用该份证据核对计划／ticket 后以 `MoveOutcome` 上返，协调器据此核对本帧名义位点并立即触发，不再重复 STATUS；冷却等真实等待之后仍重新鲜读。严格串行历史基线见 [存卡时序记录](../validation/camera-card-serial-multiexposure-2026-10-12.md)，不代表默认早释放后的全任务速度。

## 故障与关闭

- 首次 Moxtek 发现失败且从未打开串口、无既有射线会话或故障证据时，连接回执为 rejected，保留具体发现错误并提示检查 Windows USB 识别；退役未使用的代理，不触发全局 Stop 或 Nano 蜂鸣器。输出仍为 UNKNOWN、禁止开束，扫描预检失效。曾拥有射线会话、已有端口证据、真实通信/协议故障及关束未确认仍走原失效安全清理与警报；独立 Stop 的 UNKNOWN 警报规则保持有效。Moxtek owner 继续独立监测，领域观察器只在错误或关束证据变化时记录错误，避免每个 tick 刷满日志并丢失首次失败原因。

当前采集准入共用 RAM profile 6 / SD profile 7 的固定摄影策略：本次广告且严格 UINT8 的 D0C0 鲜读 OFF=0、ADL 鲜读 OFF=4、HDR 原生 OFF 或仅未广告时的 NEF 互斥证明均必需。已广告 HDR 的 ON/error/矛盾读回仍拒绝；未广告或读取失败的 ADL 不算不适用。长曝光降噪必须读回 OFF=0。参数通过明确 Apply 写入；预检只鲜读和验证。D1B1 是曝光表读数，不是 BKT 状态。

D7100摄影控件的模式/RAW/WB/HDR/包围/闪光依赖按[手册功能矩阵](native-camera-capability-matrix.md#手册功能关系与机身检查)裁决。Apply先验证全部目标互斥，显式HDR/包围OFF先执行，写后重读描述符并逐项重新校验。机身多重曝光不参与当前固定profile或运行时健康；Scan Preparation只显示预检前机身菜单OFF及指示器检查，不创建人工确认记录，不发送相机命令，也不改变readiness。一个完整主机NEF不证明一次物理曝光。

- UI 关闭或 Tauri 崩溃会导致 sidecar stdin EOF；`ct-engine` 在退出前停止扫描并强制关束。
- 全局 Stop 覆盖扫描、手动出束、HOME/MOVE、单拍和 Live View；并行请求 OFF、Nano STOP、相机取消，受理与确认分开。清理未确认时隔离会话并保持锁定，真实故障继续 fail closed；相机清理未确认时 `camera_connect` 只在显式 `acknowledgeUnconfirmedCleanup:true`、同时给出明确 `deviceId` 且该相机确实处于 isolated 且 cleanup 未确认时生效，只退役不可用的相机代理并释放相机清理锁，不清共享 fault_latched、不释放 Abandon 锁；旧会话的终止证据仍未知。连接若检测残留 Live View 会先尝试 force_live_view_off，清不掉仍拒绝并要求检查机身（自动清除成功路径尚无当前实机证据）。扫描正常收尾经过 Finishing；最后一张投影提交后保持转台在最后确认角度，确认关束和警告输出关闭、写入最终采集 manifest 后进入后续数据收尾；主机路径完成后报告 Completed，SD 路径须继续导入、封存和精确清卡。收尾不重复读取最终 Nano STATUS，不发送额外 MOVE_ABS 或 HOME。
- 已发布 readiness/capability 未满足的相机控制不再留到 worker 内才失败：`camera_capture`、`camera_focus`、`camera_autofocus` 分别在入队前拒绝为 `CAMERA_CAPTURE_UNQUALIFIED`、`CAMERA_FOCUS_UNQUALIFIED`、`CAMERA_AUTOFOCUS_UNQUALIFIED`，不产生 operation、不置 Fault、不触发全局 Stop；静态输入校验与 Live View 拍摄禁令仍保持更高优先级。
- 每轮真实扫描在任务图像目录保存 `scan.log`，记录扫描线程事件时间、来源、级别和最终 Completed/Stopped/Fault 结果；故障退出时仍写入已发生的事件与错误，便于区分投影提交后的关束、回零和文件收尾失败。
- 真实设备故障、通信丢失、超时和无法解释的状态必须尝试关束与停止运动，并进入失效安全路径；Nano 原生 STOP 和硬件急停保护保持有效。

- 普通 Stop/故障清理保留健康设备连接：Moxtek OFF 和 Nano STOP 不释放串口；相机仅在 Stop 受理时确认空闲、无原生调用/拍摄/LV/故障，且 owner 鲜读空闲事件通过时保留同一 WPD 会话。Stop 与原生 I/O 入场共用代次门，旧排队工作不得在空闲判断后执行。正在调用、待确认曝光或清理失败的相机继续隔离并关闭；显式 Disconnect、应用退出与更新维护仍关闭会话。保留连接不恢复预检、运动基准、任务资格或取消的照片回执。转台面板连接标签独立显示 CONNECTED/DISCONNECTED，运动状态放在标签提示中，位置/基准缺证据继续显示 UNKNOWN。
- 关束未确认时只能报告故障，不能报告安全完成。
- 新任务首次生产扫描仍需建立 HOME 基准。恢复任务不因软件重启单独强制 HOME：操作员通过 `confirmTurntableUnchanged` 确认台面/样品未动且控制器未失去连续位置证据，预检读取当前 STATUS 并核对冻结采集。已有有效、静止、无待提交投影的参考直接复用；被 STOP 作废的参考只在全部冻结文件验证通过、当前计数与末张/下一张投影相位匹配、设备广告 `RESTORE_REFERENCE` 时才恢复资格。额外 `confirmReferenceAtHome` 仅适用于操作员确认仍停在最近一次已完成 HOME 的精确原点，且实时计数为0、Hall为active；不能用 Hall 单项推断已回零。恢复命令不改位置计数、不发运动脉冲，也不自动开束。FAULT、位置不一致、未确认及旧固件不支持时保持阻塞，不自动改成 HOME。应用关闭仍执行 STOP；只有操作员重新选择受控 HOME 才运行回零。
- schema 3 Restore 只向符合当前内部 profile 6策略且具备完整冻结证据的任务追加；旧 profile 1–5及旧 schema 2只读保留并可重构，原 hash不改写。受控恢复比较实际冻结相机 profile、V/I、Exposure、maxXraySec，并验证 manifest、前暗场、前空场与投影图像哈希、几何和扫描参数，从记录的阶段或下一未提交帧继续。历史任务中已有的后参考帧仍保留在清单中，恢复时核对其记录，但不再拍摄或用于重构。故障、STOP 或参考失效后的 HOME 不删除已提交图像；跨 HOME 的物理角度配准并非文件校验可证明，应在重构验收中单独核对。
- 独立单拍按已确认保存目标选择路径：SD=0 使用标准卡事务及固定卡参数策略，先只读枚举并规划一张剩余容量，唯一对象/完成事件与 receipt 确认后仅报告存卡结果，不需要本地照片根目录、不下载图像或预览。原生 D7100 机身拍摄准入保留 USB ReadOnly 原始证据，检查可移除非 ROM 介质、剩余字节/张数；未知型号/能力、容量不足及真实存储错误仍拒绝。通用 USB 写入规划的 CARD_READ_ONLY 拒绝保持。RAM=1 须当前会话明确 Apply 授权，否则入队前返回 CAMERA_HOST_TRANSFER_OPT_IN_REQUIRED；获准后保留根目录/`CameraCaptures` 的原独立摄影策略。相机的M、Auto ISO OFF、MF等扫描固定限制不套给已验证的 RAM 独立自动模式。RAM 的90C0、一个与本操作关联的C101对象（C102完成可选，可早到、迟到或不到）、LV OFF、会话/操作、完整文件/hash、秒级对象窗口、多对象拒绝、取消和不覆盖仍必需。参数只有明确Apply可写；预检、开始和恢复均鲜读比较，不自动停LV或写快门/保存目标/burst。
- 独立照片预览只按 capture ID请求，在引擎完整登记后由 Tauri重新核对大小/hash并提取内嵌 JPEG，不生成 JPEG临时文件；无内嵌 JPEG时报告预览不可用。
- 生产图像预览按已提交帧索引请求，Tauri 只读取引擎快照登记且哈希一致的 NEF 文件，并提取其中的 JPEG 供界面显示；预览不参与投影提交。

## 操作员 Stop 的任务收尾

- Pause 在完整事务边界确认关束后保留采集线程和任务锁，Resume 继续当前任务。操作员 Stop 对正在开始、采集、暂停或中断保留的任务设置结束意图，先保持 Stopping 与手动占用。
- 设备清理和扫描线程返回可以独立先后完成；引擎只有两者完成且当前 OFF/相机清理证据有效才提交 abandoned 登记。登记写入完成后清空当前任务选择，释放占用；全部采集文件保留。已封存任务保留 completed 状态，不改写为 abandoned。
- 重复 Stop 可以撤销普通工作，但不能拆开 complete/abandon 的原子登记写入与引擎提交。开始登记尚未写盘时，终态写入只允许匹配冻结的前一份登记；其他来源修改登记时拒绝覆盖并维持故障锁。关闭应用、断开及故障清理没有操作员结束意图，保留恢复资格。
- 已结束任务的目录仍存在时，restore/start 返回 SCAN_OUTPUT_RETAINED，界面提示选择新任务 ID/目录；确认整个任务目录被删除后可撤销旧登记，同一 ID 只可新建采集，不能恢复已删除的采集。停止成功后的新任务仍需准备、预检与必要的 HOME，不复用旧准入结果。

## 删除目录与关闭失败

引擎在空闲且设备清理已确认时后台探测登记目录，确认保存根可读、目标目录明确不存在后原子撤销登记。现存数据、读错误、掉线根目录、失效链接和活动 worker 保持占用；Stop 与故障门禁继续独立生效。Start 对已删除的结束目录可以立即重新登记，冲突拒绝不能留下未开始任务的幽灵锁。后台删除核对的写入与 complete/abandon 一样，在重复 Stop 时保留原子写入和引擎提交的对应关系。

普通窗口关闭发送 owned engine EOF，等待有界清理。引擎已退出时无论返回码是否成功都结束桌面进程，并保留非零返回码和失败日志；引擎仍运行且清理超时则显示英文 Shutdown needs attention，不默默拦截。更新安装仍要求引擎 exit0 和关联的维护报告，异常退出不能授权安装。

## 设定值反馈

SEND V/I 在生产后台记录请求日志，写入后鲜读确认设备接受的电压/电流和 OFF 状态，再记录 PASS；12 W 限制导致另一通道调整时记录 WARN。UI 的 SET 是输入草稿，灰色 Applied 来自引擎 setKv/setUa 及对应 confirmed 标志，Measured 来自输出遥测；未确认、离线或陈旧值显示未知，不用草稿推测设备设定。

## 更新前维护停机

1. 发行检查、验证与下载不调用设备；用户确认安装/替换后，Tauri调用 `prepare_software_update`。
2. 引擎拒绝活动任务、Live View、重构、busy或未完成持久任务锁；受理后维护占用立即锁住新设备/扫描动作，Stop和只读状态仍可用。
3. 取得本次 STOP/OFF和相机清理证据，或明确从未拥有设备会话的 never-owned分支。权威快照 `softwareUpdatePreparation` 必须匹配当前 engine session/operation，且 ready、maintenanceLocked、noActiveTransactions满足；UNKNOWN或清理未确认保持阻塞。普通 Stop会使旧 ready失效，但不能清除维护锁。匹配操作取消等待清理，不能自动放弃任务。
4. Tauri收到匹配报告后才发送 EOF、等待 owned engine严格 exit0；helper再等待原应用正常退出，随后处理已验证的对应分发类型。exit0本身不证明设备安全，也不证明 installer成功。
5. 新程序按本次安装/检查授权启动，建立新设备会话；不自动连接、恢复 LV、运动或开束。扫描文件、预设和冻结任务资料不作为安装资源处理。

## 状态所有权

| 状态 | 唯一所有者 | 消费方 |
|---|---|---|
| 设备连接与健康 | `ct-engine` | Tauri 转发、React 展示 |
| 安全门控与 X 射线实测 | `ct-engine` | UI、3D 场景、日志 |
| 更新维护占用与设备清理报告 | `ct-engine` | Tauri更新器、受控helper意图 |
| 扫描 phase 与进度 | `ct-engine` | UI、manifest |
| 采集阶段、操作员确认与重构任务 | `ct-engine` | UI、Tauri 预览读取 |
| 原生路径与对话框 | Tauri | React |
| 主题、面板和相机视角 | React / scene | 用户界面 |
| 浏览器预览状态 | `WorkstationAdapter` | 非 Tauri 开发预览 |

## 暂停与恢复检查

暗场、空场、投影在当前事务完整提交后统一进入暂停：确认关束、保存断点并保留设备 owner。Resume 只受理恢复请求，界面继续显示 Paused，扫描线程重新比较冻结相机配置和当前静止转台参考，验证通过才推进；活动扫描不依赖可能被相机读回变化清除的旧 preflight 标记。

相机属性读取收到明确 DeviceBusy 时，仅重试只读查询；PTP 三次忙回复保留该类型，不伪装成传输超时并关闭会话。恢复最多三轮只读检查，仍忙则保持暂停等待 Resume；参数读回不匹配同样保留暂停，纠正后重新检查。原生 I/O 未决、会话改变、转台参考丢失、联锁/过温及关束未确认保留故障保护，拍摄和运动命令不因此重发。
