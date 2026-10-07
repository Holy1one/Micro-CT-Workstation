# 后端重验证任务清单与下一智能体执行提示词

日期：2026-10-06（Asia/Shanghai）。本文件是任务交接，不是全系统实机验收完成声明。

## 一、用户当前目标

相机已重新连接。重新验证后端底层、基本流程及提供给前端的接口，修复本次验证发现的后端问题，然后按真实结果更新接口规范。前端视觉设计由用户交给其它模型，本任务只维护必要的接口、类型及功能说明。

用户允许多智能体协作。使用用户选定的模型与当前环境实际支持的模型；不依据历史会话声称正在使用某个模型，也不擅自切换用户配置。

## 二、工作区与当前状态

| 项目 | 当前事实 |
| --- | --- |
| 工作区 | `E:/LZU_Master/MicroCT` |
| 唯一活动软件仓库 | `E:/LZU_Master/MicroCT/Micro-CT-App` |
| 分支 / HEAD | `native-device-controls-offline` / `11eed6cfea8f3f5118fcd07e70319f98fc398b51` |
| 工作副本 | 大量已有未提交修改；必须保留，HEAD 不是全部当前源码身份 |
| 应用版本 / MSI 账本 | `0.9.0-rc.3` / `0.9.3`，以当前配置再次核对 |
| 相机 | Windows 已枚举 Nikon D7100，VID `04B0` / PID `0430`，USB serial `8322769` |
| 转台 | Windows 已枚举 CH340，VID `1A86` / PID `7523`，COM7 |
| 射线源 | 当前任务没有可据以执行真实出束的连接与授权；只能验证已有离线协议/安全逻辑及未连接拒绝路径 |
| 进程状态 | 交接检查时没有 ct-engine、桌面测试程序、相机控制程序、Cargo/Rust 编译进程；没有活动子智能体 |
| 本次实机进度 | 2026-10-06 相机重新连接后的 Read/Apply/LV/拍摄/恢复尚未执行，不能借历史结果标本轮 PASS |

这些设备与进程事实会变化，开始工作时必须重新读取。工作区根不是 Git 仓库；Git 操作只在 `Micro-CT-App` 内执行。

### 已完成的本轮工作

- [离线后端重验证](../tmp/tests/desktop/20261006-141700-backend-revalidation-gates/result.json)：Rust **318 passed / 7 ignored**；Node **165 passed**；typecheck、module-map PASS。未连接设备、未重建前端。
- [离线验证说明](../tmp/tests/desktop/20261006-141700-backend-revalidation-gates/summary.md)：列出命令、覆盖范围、库存审计和未验证边界。
- [接口只读清单](../tmp/tests/desktop/20261006-142000-interface-doc-audit/inventory.json)：当前 **61 个 EngineCommand 类型、17 个自定义 Tauri IPC**，静态覆盖没有缺项。静态库存不能证明全部合法正向硬件行为。
- [前端功能与接口规范](frontend-functional-spec.md) 已按本轮源码修正若干合同；仍需相机新实机结果补证。

### 中断构建的恢复点

目录：`tmp/tests/desktop/20261006-142717-connected-backend-host/`。

- `command.txt` 记录 `node node_modules/@tauri-apps/cli/tauri.js build --no-bundle`。
- `build.log` 末尾停在 `Compiling ct-workstation v0.9.0-rc.3`；没有 `result.json` 或明确构建成功结论。
- 交接检查时构建进程已不存在，因此不能继续等待一个不存在的进程，也不能把该目录当成功构建。
- 将该次记录保留为中断过程。在新的时间戳运行目录重新构建，绑定本次 host 与实际内嵌 engine 哈希。
- 若接手时发现另外的构建仍在运行，应先按 PID/工具会话确认其状态并等待；不能因观测超时重复启动。

现有 `target/release/ct-workstation.exe` SHA-256 为 `f88c9da63040926be4bd94564846b8f73baec68207fa6e11d431916f64b64de1`，内嵌构建目录引擎 SHA-256 为 `e7bf81010c0fd8a393d14c5a05559cb7762e957adabd09dd4f9bb31e18a04a61`。它们是此前测试产物的身份记录，不证明中断后的本轮构建成功或本轮相机验收完成。

正式入口 `portable-release/micro-ct-workstation-portable.exe` 原 SHA-256：`c19b828d8f18d13d3ef2811b95b5094863aee0e494611140001bc69716c91334`。未经本轮完整验收，不以测试候选自动覆盖正式入口。

## 三、按顺序执行的任务清单

| ID | 任务 | 验收条件 |
| --- | --- | --- |
| T0 | 读取规则、确认当前源码/设备/进程与已有结果 | 明确唯一工作目录、源版本、设备身份、活动进程；保留未提交修改和失败记录 |
| T1 | 恢复并完成当前后端 host 构建 | 构建退出 0；保存命令/日志/源码身份；核对 host SHA、实际运行 engine 路径/SHA；正式入口未改 |
| T2 | 实际 Tauri → EngineClient → JSONL 链路复核 | 完整 snapshot；type/session/sequence/receipt 关联正确；未知命令、缺字段和无资源错误清晰；无模拟回退 |
| T3 | D7100 明确选择、连接、身份、参数与诊断读取 | VID/PID/serial/model 匹配；34 摄影候选及运行诊断按真实支持返回；实际值/目标/确认/错误分开 |
| T4 | 相机参数 Apply、依赖与恢复 | 选设备真实支持的有限值验证；父参数 Apply 后才刷新子能力；不支持/无效项拒绝；恢复本次鲜读原始值并核对差异 |
| T5 | Live View 启停、实时传输及专用配置 | 验证当前独立 LV nonce、D1A3/D061 描述和读回；不少于约 30 秒内存传输统计；停止/重开拒绝旧会话；禁止实时帧落盘 |
| T6 | LV 中错误请求和并发安全 | 普通摄影 Apply（含 empty/no-op）、拍摄、非法步长均正确拒绝；不因无效请求自动停止 LV、制造成功或绕开占用锁；安全 Stop 可用 |
| T7 | 独立主机 NEF 拍摄与 capture ID 预览 | 明确停止 LV；必要参数显式 Apply；获得关联事件、可靠 ExposureEnded、完整 NEF/大小/hash/登记；预览只用 ID、内存解析，不另写 JPEG |
| T8 | Nano 与双设备占用、停止、维护清理 | 当前连接/身份/状态/STOP、相机与转台的占用及维护锁正确；相机和转台清理各自确认，射线保持 UNKNOWN/never-owned；正常退出 0 |
| T9 | 扫描准备、预检、恢复/放弃、重构和日志 | 对照当前 source 和测试核基本流程；实机不满足条件时明确拒绝，参数原子提交与文件保护正确；射线/完整扫描只标实际可证明范围 |
| T10 | 依据实机结果修复与回归 | 只修复可复现的后端/IPC 根因；同步 Rust、Tauri、TS、preview 消费方和相关测试；修改后重建并验证实际测试版本 |
| T11 | 更新规范与最终验收 | 逐项填写 PASS/FAIL/BLOCKED、命令、源/二进制哈希、证据及未测边界；完整保存前端必须涵盖的功能，不改视觉设计 |

### 实机执行细节

1. 主智能体是唯一真实设备操作者。其它智能体只能离线审计、测试、整理文档。
2. 使用当前源码构建的受控候选及独立 `MICRO_CT_ENGINE_DATA_DIR`。测试照片保存至本次 `tmp/` 下隔离根的 `CameraCaptures`，保留原扫描文件和生产数据。
3. 拍摄前只读取并保存本次原始设置；不能用历史 ISO=1000、10s、f/5.6 等值当作当前原值。原本不支持/未知的字段恢复时仍应保持原证据状态，不能强求它们变成 confirmed。
4. 测试参数只选择本次 descriptor 支持的值，逐次 Apply/读回。快门保持原生精确编码，不猜索引、不把显示标签当写入值。
5. LV 测试只记录序号、尺寸、可用/过期/缺帧次数、会话与错误；不保存/打印 JPEG、base64/data URL、实时截图或像素数组。
6. `camera_live_view_read` / `camera_live_view_configure` 使用当前 LV nonce；专用配置只允许 `cameraZoom`、`focusMode`。普通摄影参数 Apply 在 LV 中应提前拒绝。
7. AF/镜头步进只在实际资格明确时考虑验证；不得强行写物理 MF=3、猜绝对焦点或声称相对驱动后已恢复原焦点位置。先完成读取和拒绝路径；能否执行实际驱动按现场条件和既有授权核对。
8. Live View 停止后再进行独立拍摄。RAM/NEF/单输出等条件只由明确 Apply 设置，预检不得隐式写入或自动停止 LV。
9. Nano 普通运动只沿生产方向。HOME/MOVE 必须核对现场运动空间和射线物理隔离条件；没有这些当前条件时完成连接/状态/STOP，运动项标 BLOCKED，不重复索取已经仍适用的授权。
10. 所有成功依据同一会话、操作及实际读回。操作受理不等于完成，取消请求不等于清理已确认；超时先检查同一进程/会话状态，不盲目重启。

## 四、必须保留的接口精度

- `Operation.deviceSessionId` 取入队时的实际记录。当前相机操作使用稳定相机连接 session；尚未建立相机会话的发现/连接阶段可能回退 engineSessionId。其它设备和域操作按源码使用 engineSessionId。不要统一强制相等。
- `CameraHealth.sessionId` 与 capture 的 session 是稳定连接标识；`liveView.sessionId/liveViewControls.sessionId` 是每次 Start 新建的独立 32hex nonce。
- `DeviceOperation.kind/stage` 与扫描 `scanFlow.stage/prompt.stage` 不同，没有通用 subtype 接口。
- `prepare_scan` 新任务几何要求 `measurement=measured` 和当前生产规定方向；`maxXraySec` 实际上限为 1–600 秒，其余字段和范围按当前代码/规范核对。
- `software_update_import` 的 native 返回为 `UpdateState`；取消文件选择返回当前状态，不是 null。TS facade 目前声明较宽的 nullable 类型，可据实际合同决定是否做最小类型修正。
- `set_parameters`、`timer_toggle` 的生产限制必须明确；不能据 preview 行为声称生产可用。
- legacy `connect {adapter:real_hardware}` 当前只连接 Nano；`retry_device camera` 只发现，不自动连接全部设备。
- 正常 `camera_capture` 只用唯一根下的 `CameraCaptures`；兼容 directory 参数只允许同一路径。目录查询从当前引擎根派生，不回退 Pictures、不创建目录。
- inactive LV 的像素 IPC 返回 `LV_NOT_ACTIVE`；active 会话暂无新帧时才可能返回 null。
- `multipleExposureEvidence=UNKNOWN` 仍阻止无证据的扫描；2026-10-08 起用户已批准并实现独立操作员确认 `camera_confirm_multiple_exposure_off`（会话/参数指纹绑定、可失效、不伪造机器 OFF、不清除其它锁）。不得把该确认展示成机器 OFF，也不得默认它在重连/重启后仍有效。

## 五、多智能体分工

| 角色 | 范围 |
| --- | --- |
| 主智能体 | 唯一设备 owner；构建/实际 Tauri、相机/Nano验证、故障处置、修复整合与最终结论 |
| 子智能体 A | Rust 扫描/相机/转台/射线及状态机离线检查和回归；不访问设备 |
| 子智能体 B | TS DTO、Tauri、JSONL、所有命令/字段/错误/完成等待的合同对照；不访问设备 |
| 子智能体 C | 根据主智能体提供的真实证据更新功能规范/验收资料与引用检查；不预先写 PASS |

为各子智能体指定清晰文件所有权。共享工作副本内避免同时修改同一文件或并行写 `dist`；Cargo/host构建按实际锁与依赖排序。

## 六、交付文件与完成标准

主要更新：

- `docs/frontend-functional-spec.md`：前端需要涵盖的基本功能、命令、字段、会话、限制、失败与未知状态语义。
- `docs/backend-interface-acceptance-2026-10-05.md`：保留历史，追加本轮范围或指向新记录。
- 可在现有 `docs/` 模块新增 `backend-revalidation-2026-10-06.md`：完整本轮验收。

过程证据统一保存在 `tmp/tests/<module>/<YYYYMMDD-HHmmss-topic>/`，包含 command、result/summary、必要日志、当前源码及二进制身份。保留失败，修复后另存通过结果。

软件离线门禁和设备实机结论分别陈述。无真实射线证据、缺少多重曝光规则或其它必要条件时，不能宣布“所有后端系统实机正常/完整扫描可用”。把可执行部分完成，再准确列出具体 BLOCKED 条件。

## 七、可直接复制给下一智能体的提示词

```text
你接手 Micro-CT 后端验证任务。工作区 E:/LZU_Master/MicroCT，软件仓库 E:/LZU_Master/MicroCT/Micro-CT-App。

用户目标：相机已重新连接，重新验证后端底层、基本流程以及提供给前端的接口；修复本次发现的问题，再据真实结果更新接口规范。前端视觉设计交给其它模型，本轮不调整 UI 外观。允许多智能体协作，真实设备只能由主智能体统一操作。

第一步读取本文件 docs/backend-next-agent-task-list-2026-10-06.md，以及根/仓库/对应模块 AGENTS.md、docs/architecture/README.md、module-map/README.md、docs/architecture/directory-map.md 和任务涉及的 modules/edges/directory-policy/module-graph。再读 docs/frontend-functional-spec.md、docs/backend-interface-acceptance-2026-10-05.md 与当前 Rust/Tauri/TS源码。

保留当前 native-device-controls-offline 分支中的大量未提交修改。只用这一个工作目录，不创建 worktree，不读取 Fritzing-archive-old，不重置用户改动。ct-engine 是唯一设备、任务、安全与扫描状态所有者；Tauri 转发，前端显示与提交意图。

先重新核对 D7100 VID04B0/PID0430/serial8322769、Nano CH340 COM7 和当前进程。2026-10-06 离线门禁已通过：Rust318/7ignored、Node165、typecheck/module-map；证据在 tmp/tests/desktop/20261006-141700-backend-revalidation-gates。静态库存61命令/17自定义IPC不等于全部正向硬件认证。

中断构建目录 tmp/tests/desktop/20261006-142717-connected-backend-host 没有成功result，日志停在桌面编译。交接时已无构建/设备进程。核实最新状态后在新run目录完成当前 --no-bundle host构建；不要把旧f88c host/E7bf engine或历史rc2实机证据当本轮成功。真正内嵌engine来自 src-tauri/build.rs 的独立 embedded-engine 构建，核实际运行路径与哈希，不猜外侧target/release的哈希。

按本文件T0–T11执行：实际Tauri/JSONL；D7100明确连接/身份/Read；支持参数Apply/依赖/恢复；约30秒LV仅内存传输；当前LV专用Read/configure、新nonce及错误/迟到拒绝；停LV后独立RAM NEF拍摄、关联C101/C102/ExposureEnded/完整文件hash与captureID预览；Nano状态/STOP/占用/维护清理；扫描准备与预检拒绝、恢复/放弃、重构及日志流程对照。所有动作以真实receipt/读回为完成依据。

每次参数修改前保存本次鲜读原值，结束时恢复并核对。测试NEF保存在本次tmp隔离根/CameraCaptures；原扫描数据只读。实时帧不得保存JPEG、截图、dataURL/base64或像素数组，统计只包含seq/dims/freshness/errors。Live中禁止普通Apply与拍摄，不自动停LV再拍。没有当前射线连接和出束授权，禁止写射线参数、开束或模拟实机成功；HOME/MOVE仍核对规定方向和已有现场门禁。禁止上传固件。

先根据源码纠正文档精度：相机Operation.deviceSessionId可能为稳定camera session，连接发现阶段/其它设备可为engine session；LV nonce独立；prepare_scan必须measured/规定方向/maxXraySec1–600；offline update picker取消返回UpdateState；生产set_parameters/timer_toggle有限制；legacy connect只Nano、camera retry只发现；正常capture目录来自唯一savePath。多重曝光UNKNOWN仍阻扫描，用户未选择人工覆盖，不擅自放宽。

只修可复现的后端/接口根因，修改DTO或语义时同步全部生产消费方和相关测试。发生失败保存原记录，修复后另存验证。发现超时先检查同一handle/进程与安全状态，不因观测超时盲目重启。

最后按当前证据更新 frontend-functional-spec.md 和后端验收资料：每项PASS/FAIL/BLOCKED、准确命令、源码/host/实际engine哈希、设备身份、文件hash、恢复、退出及未测边界。写清新前端必须保留的功能和接口，不提供新的视觉设计。完成能执行的全部工作，无法验证的部分给具体缺失条件，不把离线通过叫硬件认证，不自动发布、安装升级或覆盖正式入口。
```

## 八、2026-10-06 后续轮次进展（由接手智能体追加，不修改上文）

本文件上文的设备/进程/构建状态与"已完成的本轮工作"是 2026-10-06 首次交接时的事实。**后续轮次（1–21 轮）的完整验收、缺陷清单、修复与证据索引以 [后端重验证记录 2026-10-06](backend-revalidation-2026-10-06.md) 为准**（§5b–§5ab），前端接口规范见 [frontend-functional-spec.md](frontend-functional-spec.md)。

### 当前交付候选与门禁（本节随轮次更新）

| 项 | 当前值 |
| --- | --- |
| 交付候选 host | `56EC7FABF9770334302CF270B5504891CFCB12C1199621D2950D797A6F03ECA8` |
| 内嵌引擎 | `6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7` |
| 正式入口 | 未改动，`C19B828D8F18D13D3EF2811B95B5094863AEE0E494611140001BC69716C91334` |
| 离线门禁 | 既有 `cargo test --workspace` **328 passed / 0 failed / 7 ignored**（Rust 本轮未改、未重跑）；第 19 轮重跑 Node **166/166**、typecheck 0、module-map PASS（1869 files / 16 modules / 25 boundaries），均仅离线验证 |
| 分支 / HEAD | `native-device-controls-offline` / `11eed6cfea8f3f5118fcd07e70319f98fc398b51`（本轮开始时工作副本 105 处未提交修改；生产源码原样保留，文档按本轮证据更新） |

### T0–T11 状态（截至第 21 轮）

| ID | 状态 | 说明 |
| --- | --- | --- |
| T0–T2 | **PASS** | 状态/设备/进程每轮重读；Tauri → EngineClient → JSONL 链路、回执关联、未知命令与错误码均在实机复核 |
| T3 | **PASS** | D7100 身份匹配、34 项摄影参数 + 运行诊断、actual/target/confirmed/error 分离 |
| T4 | **PASS** | 支持值 Apply/串行读回；`iso 1000→200→1000` 且与鲜读基线 34 项差异为 0；不支持/无效项按 9 个拒绝码分类且不再毁会话 |
| T5 | **PASS** | 独立 LV nonce、D1A3/D061 读回、**32 秒仅内存传输 129 帧 / 0 重复 / 序列单调 / 0 错误**、停止后取帧 `LV_NOT_ACTIVE`、旧会话 `STALE_LIVE_VIEW_SESSION` |
| T6 | **PASS** | LV 期间：普通 Apply（含 empty）→ `LIVE_VIEW_APPLY_FORBIDDEN`、拍摄 → `LIVE_VIEW_CAPTURE_FORBIDDEN`、非法步长 → `INVALID_CAMERA_FOCUS_STEP`、AF → `CAMERA_AUTOFOCUS_UNQUALIFIED`；**全部拒绝后 LV 仍活动、nonce 不变、无 fault**；Stop 可用（617 ms） |
| T7 | **PASS** | §5z：driveMode=1；独立 1/5/15/30 s 主机 NEF 共五张，可靠 C101/C102/ExposureEnded、文件/hash/登记、capture-ID 内存预览；34 项恢复 diff=0，三次退出 0/0。使用者确认扫描 imageCount 保持 0 |
| T8 | **PASS** | Nano 连接/身份/状态/STOP、双设备占用与维护锁、相机与转台清理各自确认、射线保持 UNKNOWN、正常退出 0 |
| T9 | **PASS** | `set_capture_root`→`prepare_scan` 原子提交且**未创建任务目录**；`preflight`→`XRAY_NOT_CONNECTED`、`start_scan`→`PREFLIGHT_REQUIRED`、`restore_previous`→`PREFLIGHT_REQUIRED`、`start_reconstruction`→`INVALID_RECONSTRUCTION_METHOD`、`abandon_scan`→`NO_ACTIVE_TASK`；taskLock 正确 |
| T10 | **PASS（F-12 根因 BLOCKED）** | F-01…F-10 已修复/回归；F-12 原失败保留，本轮同参数 30 s 与新会话首张 30 s 均成功，未复现且未标已修复 |
| T11 | **PASS（有 BLOCKED 项）** | 本文件 + 重验证记录 + 前端规范均已按实际证据更新；未测边界与 BLOCKED 条件逐条写明 |

### 已修复缺陷（完整证据见重验证记录）

F-01（LV 期间 Stop 卡死/退出挂起）、F-02（Apply 拒绝码折叠）、F-03/F-04（拍摄/对焦门禁先于设备操作且不锁故障）、F-06（发现回执 kind）、F-07（机身残留 LV 的软件清除）、F-08（重复 connect 锁故障丢会话，含空白 `deviceId` 绕过）、F-09（LV 控制事务阻塞 Stop）、F-10（设备拒绝请求值却毁会话）。另修：`rejected` 状态的前端契约（快照校验白名单 + 操作等待器终态 + 判别测试）、值拒绝分类收紧为仅有实机证据的 `PTP 0x201c`。

### 第四节「必须保留的接口精度」的措辞更正（第 14 轮核对；上文第四节保持交接原样）

第 14 轮由独立只读审计逐条核对（`tmp/tests/desktop/20261007-142000-interface-precision/`，typecheck/module-map/Node 165 全绿）：**8 条仍成立，2 条需更正**。以下措辞以本节为准。

| 原第几条 | 更正 |
| --- | --- |
| 1 | 回退规则本身成立，但**漏写一处例外**：活动相机会话期间的 `camera_live_view_stop` 以设备标签 `camera_cleanup` 入队（`device_controls.rs:465-472`），其 `deviceSessionId` 回退为 engineSessionId。按 `deviceSessionId` 分组的消费方必须容忍。 |
| 2 | **已澄清**：`liveView.sessionId` 与 `liveViewControls.sessionId` 是**同一个** per-Start nonce 且必须相等（`nikon.rs:813`、`properties.rs:15`），不是两个独立 nonce；它们只独立于稳定连接会话。 |
| 3 | 成立；**补充** `Operation.status` 词表：`queued/running/cancelling/completed/cancelled/failed/cleanup_unconfirmed/rejected`（`rejected` 见 `device_controls.rs:187`：设备拒绝请求值，不 fault/不 Stop/不隔离，但失效 preflight）。`scanFlow.stage` 闭集已含 `persisting_completed`。 |
| 4 | 成立；`maxXraySec` 1–600 在 `device_controls.rs:51`、`lib.rs:1014/1161/1220` 多处校验；**其余字段**：`projection_count` 1–3600、`angleStepDeg` 须与 360/count 一致、`taskId` 有保留名约束、`savePath` 必须绝对路径（`lib.rs:131-157`）。 |
| 5 | **已过期**：TS facade 的最小类型修正**已完成**——`src/platform/updater.ts:28` 现为 `Promise<UpdateState>`（不再 nullable）。原句不能说"仍声明较宽 nullable 类型"。native 语义不变（`main.rs:50-62`，取消走 `service.status()`）。 |
| 6 | 成立且更强：`set_parameters`→`STAGE_1_NANO_ONLY`、`timer_toggle`→`XRAY_STAGE_LOCKED`、`timerControlsEnabled=self.preview`，且二者不在 `device_controls.rs:216` 白名单，无法绕过。 |
| 7 | 成立；**补充**：无 deviceId 时 kind=`camera_discover`（`device_controls.rs:432`），发现到 0 台或 ≥2 台一律 completed、不报错、不锁故障、不隐式选设备。 |
| 8 | 成立；**补充**新码 `CAMERA_CAPTURE_ROOT_REQUIRED`，以及单测 `main.rs:894-905`（"unknown roots cannot fall back to Pictures"、不创建目录）。`FOLDERID_Pictures` 只服务选图默认值，与捕获目录查询是两条路径。 |
| 9 | 成立：inactive → `LV_NOT_ACTIVE`（`device_controls.rs:233`），渲染层实收 `ENGINE:LV_NOT_ACTIVE`；仅 active 且会话一致但无新帧才 `null`。 |
| 10 | 成立且**源码强于文档**：非 test 构建的采集 profile 恒为未知（`capture_profile.rs:119-123`）、"已证明未激活"在非 test 恒假（`types.rs:295-302`，唯一 true 出口被 `cfg(test)` 锁死），**不存在任何人工覆盖入口**——只能改源码。前端不得提供"确认即可继续"的开关。 |

规范侧已同步两处最尖锐的表述：`frontend-functional-spec.md` 的会话字段段（nonce 相等 + `camera_cleanup` 例外）与 preflight 段（多重曝光无覆盖入口、`ready` 不可能为真）。

### 仍需人工决定或仍 BLOCKED

1. **F-12 根因**：T7 已 PASS；原 WPD 0x8007001F/cleanup_unconfirmed 未复现，同参数与新会话 30 s 均成功。需具体失败阶段/驱动 trace 才能归因；不得按长曝光必然失败处理，原 run 保留。
2. **射线参数/开束、HOME/MOVE、真实扫描**：无当前连接与出束授权；`multipleExposureEvidence=UNKNOWN` 按设计阻止扫描，用户未批准人工覆盖。
3. **D-3（记录项）**：LV 控制超时会按 fail-closed 退役会话（`call()` 到点即 `invalidate_epoch`）；实测正常事务 66 ms 对 2 s 上限约 30× 余量，暂不改动。

### 第 19 轮当前结论

T7 已完成，证据与文件 SHA 见重验证 §5z。F-12 本轮未复现，根因未确证；全部恢复并正常退出。两栏保持使用者确认的 tooltip/Advanced，留档见 [重设计确认](shots/2026-10-07-panel-redesign/README.md)。当前未动正式入口。软件更新维护锁仅有离线证据；旧 PID 52332/40964 环境残留仍需注销/重启清除，本轮未执行。

### 第 20 轮补证

仅只读审计：原失败时间窗存在 WPD-MTPClassDriver event1002（State=4、hr=0），早于引擎 Stop 约5ms；原字符串筛选漏掉此事件。事件无设备ID/PTP失败阶段，不能证明物理拔线。源码允许失联/未确认 health 先自动Stop、迟到错误回执随后变cancelled，因此 cancelled 不能独立判定用户Stop。完整证据见重验证 §5aa；F-12根因仍BLOCKED，T7/UI确认不变。

### 第 21 轮相机重启后复测

使用者重启相机后，新会话的1s/30s独立NEF均PASS，可靠C101/C102/ExposureEnded、文件/hash/登记和内存预览成立；34项恢复diff=0，app/engine0/0，9400释放。当前可拍，重启因果与F-12精确根因未确证，原失败不标已修复。证据见重验证§5ab。生产源码和正式入口保持原样，T7/UI确认保持完成。

后续只读对照：原失败前与重启后30s前，34项摄影证据、DeviceInfo/厂商目录hash和operation/effectiveProperties相同；runtime仅batteryLevel100→60、曝光表原值54→58不同，其余8项相同。保留未知语义，不能据差值归因；结果在重启run的runtime-comparison.json。
