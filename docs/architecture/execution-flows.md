# 执行逻辑

## 桌面启动

1. Tauri 创建窗口和共享 `EngineClient`。
2. 后台线程启动 `ct-engine` sidecar，避免阻塞 Tauri `setup`。
3. `EngineClient` 校验 sidecar 二进制并建立 stdin/stdout JSONL 通道。
4. React 首次调用 `engine_snapshot`；sidecar 未就绪时返回明确错误，不切换到预览。
5. 操作者显式连接真实设备并运行预检；启动应用本身不触发真实运动或出束。

## 浏览器预览启动

1. `createEngineAdapter` 检测不到 Tauri runtime。
2. 创建 `WorkstationAdapter` 和内存 RTS9060 transport。
3. 预览生成与生产相同形状的 `EngineSnapshot`，但不得打开串口、调用 WPD/PTP 或接触真实设备。相机、转台显示为未连接；启动不自动 HOME或预检。明确连接、Apply和准备遵循相同领域门禁；模拟单拍不创建 NEF文件或 capture登记，维护模拟报告标在 developer_preview会话和 NO HARDWARE操作中，不能用作生产安全证据。
4. 一旦处于 Tauri runtime，任何 IPC 失败都直接报错；不得回退到预览。

## 命令执行

当前新源码新增会话绑定的LV控制命令：LV从inactive→Start创建新32hex nonce；`camera_live_view_read {sessionId}`只鲜读，`camera_live_view_configure {sessionId,controls}`只接受D1A3 `cameraZoom`和D061 `focusMode`两个已广告且严格UINT8描述的控制。每次SET前重新确认当前会话/原生权限，写后读回；不能透传任意私有码。其会话与pipe/帧metadata相同，区别于稳定camera连接/capture会话；Stop后旧请求失效，迟到结果不能跨新LV会话提交。普通摄影Apply仍禁止LV，LV值不进入摄影preset或固定扫描profile。`camera_apply_properties` 的拒绝原因按 `PropertyRejection` 分码返回（`LIVE_VIEW_APPLY_FORBIDDEN`、`INVALID_CAMERA_PARAMETER_COUNT`、`CAMERA_PROPERTY_UNDESCRIBED`、`CAMERA_FOCUS_MODE_LOCKED`、`CAMERA_PROPERTY_WRITE_UNSUPPORTED`、`CAMERA_PROPERTY_UNSUPPORTED_VALUE`、`CAMERA_PROPERTY_DEPENDENCY_UNMET`、`CAMERA_PARAMETER_SESSION_UNAVAILABLE`），不再折叠为单一错误码，且都在入队前返回、不产生 operation、不置 Fault；`preset_load` 仍使用粗粒度的 `UNSUPPORTED_CAMERA_PROPERTIES`。

LV AF/step还须鲜读镜头/照片选择器和D061资格、实际操作广告及DeviceReady：lensSort=1、selector=0时模式0允许AF、0/4允许step，模式2 AF-F不允许有界motor，模式3物理MF禁止写/motor。LV未激活时`focus_step`能力恒为false，`camera_focus`按不入队拒绝处理；`camera_autofocus`在LV未激活时可依普通`focusMode` 0/1/2与镜头驱动广告执行，不隐式触发AF。未知或失败按禁用处理，不以lensSort单项推断资格。本轮源码已构建rc3 host并取得真实Tauri IPC、相机与Nano分项证据（见[后端重验证记录](../backend-revalidation-2026-10-06.md)）；rc2签名包与验收不随源码更新而改变，Multi扫描仍BLOCKED且人工策略未答。

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
3. 引擎确认关束后弹装样提示；确认后执行投影事务。每张投影均须确认已传输的 NEF 最终文件，并提交其索引、大小及哈希。
4. 末张投影及暂停按本次关联 ExposureEnded 关束；完整文件/哈希、同 ticket 的 CAPTURE_DONE、IDLE 和提交后才完成。确认警告输出关闭并封存 manifest。收尾不重复复核全部影像，不再拍后空场或后暗场。
5. 开始后的参考/装样采集提示取消、Esc 或关闭均发送 `cancel_scan_stage`。引擎负责安全结束和阶段断点；React 不自行推进阶段。恢复前仍需重新预检、必要的 HOME 与文件核对。
6. 封存后只能从底部 Reconstruction 区选择方法并打开重构预览；3D View 内不设重构入口。首次选择后引擎后台读取已校验的 NEF 和前暗场、前空场，执行可用方法并保存科学体、预览体和哈希。再次选择相同方法先核对缓存。Tauri `reconstruction_preview` 仅以方法 ID 读取引擎登记的预览缓存。

FDK 有 CPU 与真实 GPU 计算路径，首帧实测和数值比对通过且预计至少快 15% 才选 GPU；失败时全量 CPU 重算。小规模扫描可选 64³、五轮 CPU SIRT；超出计算预算时置灰。CGLS 无后端，保持不可选。体素间距是计算采样间距，不代表实测分辨率。这些数值路径的离线测试不证明 CT定量精度。D7100 独立 NEF/LV 已有指定 P0包分项实机证据，见 [当前架构的验证边界](README.md#更新维护与验证边界)；完整射线联动、新 profile 5策略实机及重构精度未由该 P0证据认证。

## 故障与关闭

当前采集准入使用profile5：本次广告且严格UINT8的D0C0鲜读OFF=0、物理适用ADL鲜读OFF=4、HDR原生OFF或仅未广告时的NEF互斥证明均必需。已广告HDR的ON/error/矛盾仍拒绝；未广告或读取失败的ADL不能算不适用。多重曝光物理适用但无可靠PTP OFF编码，真实D7100明确UNKNOWN/不可观测，因此上述扫描流程当前BLOCKED，没有人工确认覆盖。原rc1固定自动化缺口已发现，旧P0/RAM单文件/C102不构成豁免；官方与primary协议依据见[能力矩阵](native-camera-capability-matrix.md#依据索引)。D1B1是曝光表读数，不是BKT状态。

- UI 关闭或 Tauri 崩溃会导致 sidecar stdin EOF；`ct-engine` 在退出前停止扫描并强制关束。
- 全局 Stop 覆盖扫描、手动出束、HOME/MOVE、单拍和 Live View；并行请求 OFF、Nano STOP、相机取消，受理与确认分开。清理未确认时隔离会话并保持锁定，真实故障继续 fail closed；相机清理未确认时 `camera_connect` 只在显式 `acknowledgeUnconfirmedCleanup:true`、同时给出明确 `deviceId` 且该相机确实处于 isolated 且 cleanup 未确认时生效，只退役不可用的相机代理并释放相机清理锁，不清共享 fault_latched、不释放 Abandon 锁；旧会话的终止证据仍未知。连接若检测残留 Live View 会先尝试 force_live_view_off，清不掉仍拒绝并要求检查机身（自动清除成功路径尚无当前实机证据）。扫描正常收尾经过 Finishing；最后一张投影提交后保持转台在最后确认角度，确认关束和警告输出关闭、写入最终 manifest 后报告 Completed。收尾不重复读取最终 Nano STATUS，不发送额外 MOVE_ABS 或 HOME。
- 已发布 readiness/capability 未满足的相机控制不再留到 worker 内才失败：`camera_capture`、`camera_focus`、`camera_autofocus` 分别在入队前拒绝为 `CAMERA_CAPTURE_UNQUALIFIED`、`CAMERA_FOCUS_UNQUALIFIED`、`CAMERA_AUTOFOCUS_UNQUALIFIED`，不产生 operation、不置 Fault、不触发全局 Stop；静态输入校验与 Live View 拍摄禁令仍保持更高优先级。
- 每轮真实扫描在任务图像目录保存 `scan.log`，记录扫描线程事件时间、来源、级别和最终 Completed/Stopped/Fault 结果；故障退出时仍写入已发生的事件与错误，便于区分投影提交后的关束、回零和文件收尾失败。
- 真实设备故障、通信丢失、超时和无法解释的状态必须尝试关束与停止运动，并进入失效安全路径；Nano 原生 STOP 和硬件急停保护保持有效。
- 关束未确认时只能报告故障，不能报告安全完成。
- 每次应用启动后的首次生产扫描必须 HOME。普通停止或故障导致 Nano STOP、失联或参考失效时，必须重新预检并 HOME。同一有效 Nano 会话中，预检读取实时 STATUS；仅 IDLE、无待提交投影、参考有效、已 HOME、已 REARM 且目标等于当前位置时可复用此前 HOME 基准，启动前再次核验。应用关闭仍发送 Nano STOP，不为了跨进程保留参考而削弱停机。
- schema 3 Restore 只向符合当前内部 profile 5策略且具备完整冻结证据的任务追加；旧 profile 1–4及旧 schema 2只读保留并可重构，原 hash不改写。受控恢复比较实际冻结相机 profile、V/I、Exposure、maxXraySec，并验证 manifest、前暗场、前空场与投影图像哈希、几何和扫描参数，从记录的阶段或下一未提交帧继续。历史任务中已有的后参考帧仍保留在清单中，恢复时核对其记录，但不再拍摄或用于重构。故障、STOP 或参考失效后的 HOME 不删除已提交图像；跨 HOME 的物理角度配准并非文件校验可证明，应在重构验收中单独核对。
- 正常独立拍摄只使用根目录/`CameraCaptures`。相机的M、Auto ISO OFF、MF等扫描固定限制不套给已验证的独立自动模式；未知多重曝光明确诊断，不单独禁用一个完整host NEF输出。该契约不声称一次物理曝光，90C0、一C101、关联C102、LV OFF、会话/操作、完整文件/hash、超时、多对象拒绝、取消和不覆盖仍必需。参数只有明确Apply可写；预检、开始和恢复均鲜读比较，不自动停LV或写快门/RAM/burst。
- 独立照片预览只按 capture ID请求，在引擎完整登记后由 Tauri重新核对大小/hash并提取内嵌 JPEG，不生成 JPEG临时文件；无内嵌 JPEG时报告预览不可用。
- 生产图像预览按已提交帧索引请求，Tauri 只读取引擎快照登记且哈希一致的 NEF 文件，并提取其中的 JPEG 供界面显示；预览不参与投影提交。

## 更新前维护停机

1. 发行检查、验证与下载不调用设备；用户确认安装/替换后，Tauri调用 `prepare_software_update`。
2. 引擎拒绝活动任务、Live View、重构、busy或未完成持久任务锁；受理后维护占用立即锁住新设备/扫描动作，Stop和只读状态仍可用。
3. 取得本次 STOP/OFF和相机清理证据，或明确从未拥有设备会话的 never-owned分支。权威快照 `softwareUpdatePreparation` 必须匹配当前 engine session/operation，且 ready、maintenanceLocked、noActiveTransactions满足；UNKNOWN或清理未确认保持阻塞。普通 Stop会使旧 ready失效，但不能清除维护锁。匹配操作取消等待清理，不能自动放弃任务。
4. Tauri收到匹配报告后才发送 EOF、等待 owned engine严格 exit0；helper再等待原应用正常退出，随后处理已验证的对应分发类型。exit0本身不证明设备安全，也不证明 installer成功。
5. 新程序由操作员启动，建立新设备会话；不自动连接、恢复 LV、运动或开束。扫描文件、预设和冻结任务资料不作为安装资源处理。

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
