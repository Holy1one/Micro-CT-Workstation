# D7100 SD 卡采集迁移与扫描提速规划

状态：用户已确定主要边界，可交接实施；设备协议和性能仍待实机验收。日期：2026-10-10。交接对象：后续执行智能体。

目标是在扫描曝光循环中只把 NEF 保存到 D7100 的 SD 卡，PC 保留控制、设备监测、投影角度和文件对象对应记录；安全收尾后通过 USB 批量导入，校验通过再重构。首期完整范围是单卡/双卡 Overflow 存卡采集、USB 导入、1000 秒扫描出束策略、所有普通移动末脉冲结束后统一等待 150 ms，以及实测提速。交接时可直接复制第 11 节精简提示词，协议和验收细节见全文。

**已确定的拍摄节奏：运动脉冲结束 → 等待 150 ms → READY → 曝光 → 本张确认 → 下一位置。** 不同角度与一圈投影数量均采用该规则。150 ms 在固件执行，PC 依据匹配事件推进。当前运动曲线下，360 张、1 秒曝光的纯运动/曝光估算约 16.0 分钟；还需相机事务等开销。连续出束上限按用户向厂家咨询后的要求设为 1000 秒，保留到限关束与 300 秒休息。12 分钟继续作为后续目标。

本规划没有改变生产源码、固件或设备设定；没有连接设备、打开串口、拍摄、运动或出束。SD 模式尚无本次实机证据。

## 1 用户已确认的边界与实施假设

以下用户回答是本规划的约束，执行方直接采用，不重新要求选择导入方式、上限或双卡用途。

| 边界 | 用户确认 | 对实施的影响 |
| --- | --- | --- |
| 后续数据导入 | 扫描结束后通过 USB 统一导入电脑 | 原生 camera owner 增加批量对象下载；完整导入、校验和重构就绪属于首期交付 |
| 连续出束上限 | 按 1000 秒；用户表示已向厂家咨询 | 统一扫描入口、冻结配置和独立截止保护；依据记为用户转述厂家答复，整机联动仍须验收 |
| 相机 | D7100 是唯一扫描相机 | 首期不扩展其它型号或猜测 Nikon 私有操作 |
| 卡容量和数量 | 64 GB，一般双卡，有时单卡 | 首期同时支持单卡与双卡；逐存储鲜读剩余容量，不推定双卡合计一定为 128 GB |
| 双卡用途 | Overflow：第一张满后续存第二张 | 每次曝光对应一个主 NEF，允许经验证的换槽续存；首期不实现 Backup 或 RAW/JPEG 分卡 |
| 转台等待 | 所有普通移动完成脉冲输出后统一等待 150 ms | 与角度和投影数量无关；固件到期发送 READY，PC 不叠加固定等待；首期纳入固件修改 |
| 12 分钟口径 | 仅核心 360 张投影，不包含其它阶段 | 前暗/空场、装样、USB 导入和重构分别计时；核心段内真实发生的休息和收尾计入 |
| 实施优先级 | 先完成存卡、150 ms 等待并实测提速，12 分钟作为后续目标 | 首期不以 720 秒为门槛；加减速/巡航曲线进一步优化在实测后决定 |

以下为保持现有系统一致性的实施假设：

- 保持当前已确认的位深、压缩、裁切、ISO、光圈和科学采集策略，继续 NEF-only；不以降品质、改 JPEG 或减参考帧兑现提速。
- 独立单拍跟随明确 Apply 的保存目标，SD 单拍与扫描计数隔离；不允许扫描选择 SD 后单拍静默改回 RAM。
- 首期新 SD 扫描默认 `maxXraySec=1000`，允许设置较小值；旧任务继续使用原冻结值，独立手动出束默认期限保持现有 600 秒。
- 安全收尾后自动启动 USB 导入，提供取消、续导与明确进度；取消并清理导入后可开始下一任务，旧任务保留为待导入。

实施前从设备和现场记录核对：相机固件、卡型号及各槽实际容量/可写状态、Overflow 菜单与协议证据、实际 Nano 固件是否对应候选源码、脉冲比例和方向、负载稳定、相机供电，以及射线源运行功率和散热。源码中的固件常量不证明控制器当前已刷入该版本。这些属于实施验收采证内容；无法证明时报告具体缺口，不补造身份、参数或设备成功。

## 2 已阅读的系统范围和当前基线

唯一活动软件仓库为 `E:/LZU_Master/MicroCT/Micro-CT-App`。工作区根不是同一 Git 仓库；不创建第二工作副本或 worktree。

本次读取时分支为 `native-device-controls-offline`，HEAD 为 `b1a796a6c78d440e6ca3d122807ea2cd08ceb0ab`；`package.json` 为 0.9.27。已有大量未提交改动，包括扫描、任务读取、叠层重构、前端及发布资料。本规划依据当前工作文件；执行智能体须先重新检查状态，保留已有改动。部分说明页仍写 0.9.12 或早期候选状态，不能据其判断当前版本或认证范围。

梳理范围包括：根及各活动模块规则、架构/目录/模块与强依赖图谱、命令和快照、原生相机与取消链、Moxtek worker 和监测、Nano 协议及固件运动、扫描参考和投影事务、清单和恢复、持久任务锁、项目打开、单色/叠层重构与缓存、图像预览、前端准备和错误呈现、离线预览、测试入口、安装和更新维护。

硬件目录只读取规则，未检查 PCB；该迁移沿现有 USB 控制链设计，不提出接线或电路修改。`Related_files` 只作为厂商资料和有许可协议参考。永久封存区没有被读取或作为依据。

### 2.1 生产职责

```text
Operator → React UI → Tauri IPC → EngineClient JSONL v2 → ct-engine
                                                        ├ Camera COM owner / WPD / PTP
                                                        ├ Moxtek serial owner / watchdog
                                                        └ Nano serial owner / confirmed motion

扫描顺序、任务占用、安全裁决、采集账本和数据就绪：ct-engine
参数草稿、状态展示、明确操作请求：React
目录选择、受限媒体读取、进程生命周期、安装更新：Tauri
NEF 校正、FDK/SIRT、叠层颜色与材料处理：引擎离线重构
```

相机、射线和转台设备模块互不调用。扫描线程编排长事务；主循环和设备 owner 持续监测，Stop/OFF 不等待图片传输或相机 COM。浏览器预览不连接硬件，生产 IPC 失败不转模拟成功。

### 2.2 当前采集与数据链

1. 明确 Apply 写入并读回相机参数；扫描准备提交任务、投影数、几何和 `maxXraySec`；预检鲜读比较，不隐式修复参数。
2. 必要时受控 HOME；Start 后拍 10 张前暗场，连续开束拍 10 张前空场并关束，装样确认后开始投影。
3. 每个投影通过负方向规划到角度，取得 `READY_TO_CAPTURE`、匹配运动 ticket、设备位置和 `CAPTURE_HOLD`。
4. 相机当前固定 `recordingMedia=1`；`0x90C0` RAM 捕获；关联 `0xC101` 对象与 `0xC102` 完成事件。
5. GetObjectInfo → GetObject 下载 → NEF 结构与长度检查 → 写文件、SHA-256、flush、原子不覆盖提交。
6. 扫描提交再次读取文件计算哈希，发送同 ticket 的 `CAPTURE_DONE`、确认 IDLE、写 manifest，再推进计数。
7. 最后投影/暂停在关联 ExposureEnded 后关束；完整文件和事务提交完成后才报告终态。完成后不再拍后参考，不额外 HOME 或转完最后 1°。
8. 恢复核对冻结参数、设备、几何、参考帧、每个本地文件及哈希。打开已完成项目与重构均要求本地完整数据。

360 张投影的计划角为 0°、-1°…-359°，没有重复的 360° 端点。1° 对应约 266/267 脉冲，沿现有规划分配取整误差；不能简单每次固定发 267 脉冲。Nano 的确认值是设备脉冲计数证据，不是负载端编码器的实测角度。

### 2.3 关键源码定位

| 内容 | 文件和入口 |
| --- | --- |
| RAM 采集、事件与本地文件 | `backend/ct-engine/src/devices/camera/capture.rs`：`capture_with_policy`、`confirm_frozen_capture`、`download_current_ram` |
| COM owner、消息、取消与清理 | `devices/camera/actor.rs`：`Request::Capture`、`cleanup`、capture API |
| 能力、参数、RAM 固定要求 | `devices/camera/nikon.rs`：`negotiated_capabilities`、`read_properties`、`profile_readback`；`properties.rs` |
| 对象/事件解析和传输 | `devices/camera/ptp.rs`、`types.rs`、`wpd.rs` |
| profile 版本、科学参数和 hash | `backend/ct-engine/src/capture_profile.rs`，当前新扫描 profile 6 |
| 参考、投影、恢复和 manifest | `backend/ct-engine/src/scan.rs`：`run_scan_inner`、`capture_reference_group`、`commit_projection_monitored`、`load_resume`、`persist` |
| 领域准入和完整快照 | `backend/ct-engine/src/device_controls.rs`、`lib.rs`、`operations.rs` |
| 跨重启占用与终态登记 | `backend/ct-engine/src/task_registry.rs` |
| 项目打开、完整性和重构 | `scan_project.rs`、`reconstruction.rs`、`reconstruction_preprocess.rs`、`reconstruction_channels.rs`、`reconstruction_spectral.rs`、`material_decomposition.rs` |
| 连续出束、2 秒再次开束、监测 | `devices/xray/worker.rs`、`moxtek.rs` |
| 转台 ticket 和方向规划 | `devices/turntable/nano.rs`、`planner.rs`；`firmware/rts9060_nano/rts9060_nano.ino` |
| 类型、快照消费和离线预览 | `frontend/src/engine/*`、`App.tsx`、`frontend/preview/workstationAdapter.ts` |
| 保存目标、准备和单拍呈现 | `CameraPanel.tsx`、`CameraSettingsDialog.tsx`、`camera-draft.ts`、`ScanPreparationDialog.tsx`、`scan-input.ts`、`WorkstationMedia.tsx`、`menuActions.ts` |
| 原生选择目录和媒体读取 | `frontend/src/platform/*`、`backend/src-tauri/src/main.rs`、`engine_client.rs`、`camera_forwarder.rs` |
| 所有权与门禁 | `docs/architecture/modules.json`、`edges.json`、`directory-policy.json`；`package.json` |

## 3 时间目标与真实瓶颈

### 3.1 已选定的 150 ms 策略与时间推算

当前源码：96,000 pulses/rev，起步周期 12,000 μs，巡航周期 2,000 μs，每步调整 100 μs，前后加减速最多各 100 脉冲，`SETTLE_MS=500`。现有 1° 约 2.034 秒。用户已确定首期将普通移动等待改为 150 ms，其余运动曲线保持当前值。

时间起点是最后一个 STEP 脉冲的下降沿，确认 STEP 已回低且目标脉冲数已完成；不能在最后高电平尚未结束时开始等待。现有 `finishPhase` 在下一脉冲周期到期后进入 SETTLING，执行方须核对并消除这段隐含尾周期，使新策略从真正的末脉冲结束起计时。使用回绕安全的单调时钟和 ISR 时间戳读取，保证至少满 150 ms 后发送 READY；零位移 MOVE 从接受命令时等待同样 150 ms，HOME 内部阶段保持原语义。

按当前脉冲曲线、150 ms 从最后下降沿起算的理想估计如下，未计调度与通信误差：

| 单次转动角度 | 代表脉冲数 | 运动开始至 READY |
| --- | --- | --- |
| 0.5° | 133 | 约 1.292 秒 |
| 1° | 267 | 约 1.672 秒 |
| 2° | 533 | 约 2.204 秒 |
| 5° | 1,333 | 约 3.804 秒 |
| 10° | 2,667 | 约 6.472 秒 |

对不少于 200 脉冲的普通移动，近似 `T_ready = 0.002 × 脉冲数 + 1.138 秒`；小角度按实际逐脉冲曲线累计。此式仅用于估计/超时预算，不触发曝光。不同投影数使用现有绝对角度与脉冲取整规划，逐张等待真实 READY，不写死“每张 1°”或每次固定 267 脉冲。

360 张、1 秒曝光时，采用 150 ms 后纯运动/曝光约 16.0 分钟；原 500 ms 对照约 18.2 分钟。减少等待预计节省约两分钟，但相机查询、写卡确认、账本和射线预算仍可能使整段超过 1000 秒。150 ms 是用户选定的稳定预留策略，READY 提供的是脉冲完成及等待到期证据；负载端是否足够稳定由图像和机械验收证明，不宣称已增加编码器实测。

总时间按 `曝光 + 各次运动及稳定 + 相机准入/事件/写卡等待 + 控制与持久化 + 休息 + 收尾` 分解。核心段统一从投影阶段开始至最后投影安全收尾完成计时，包含该段开束确认、实际休息和末张关束/事务提交；另记录首张 trigger 到末张对象确认的净采集时间。扫描工作流另有 20 张前参考、两次现场提示和装样；360 投影时间不能当整个实验时间。首期报告实际收益，超过 720 秒不因此判定存卡迁移失败。

### 3.2 必须测量的阶段

PC 使用同一单调时钟记录每张：运动请求、READY 接收、CAPTURE_HOLD 确认、相机鲜读、事件队列清空、trigger 受理、完成事件、SD 对象可确认、DeviceReady、CAPTURE_DONE/IDLE、账本持久化、下一次 trigger。另以固件时钟/台架测量记录末 STEP 下降沿至 READY 的间隔；不直接相减未同步的 PC 与 Nano 时间戳。RAM 对照另记录 ObjectInfo、下载、NEF 检查、首次 hash/flush、扫描第二次 hash。

报告 1° 运动和完整周期的均值、P50/P95/P99、最大值及整组耗时；记录真实卡、参数、负载、固件和构建哈希。对比 30 张预试与完整 360 张的持续表现。单拍速度、卡包装标称读取速度或少量照片缓存速度不能代表全程。

当前每次 `read_properties` 遍历摄影属性，读取 descriptor 和实际值；采集前后及某些通知会多次调用。USB 传输是要移出的开销，重复全量参数查询、卡写入和运动也可能占主导。先测量后决定优化，不能承诺移除下载就达到目标。

### 3.3 SD 容量与写入预算

容量覆盖 360 投影 + 10 暗场 + 10 空场，还应留试拍和容量裕量。NEF 大小随设置和场景变化，预检使用当前品质的保守上界，未知容量/可写状态不得当充足。

Nikon 手册的 DX 14-bit lossless 示例约 28.5 MB/张，380 张约 10.83 GB；这是估算，不是本机文件尺寸。2 秒一张要求长期平均写入至少约 14.25 MB/s，实际需要裕量，且串行等待卡确认时受单张延迟制约。机内缓冲不能长期弥补写入不足。[Nikon D7100 手册，印刷页 58、348](https://cdn-10.nikon-cdn.com/pdf/manuals/dslr/D7100_EN.pdf)

首期支持单卡与双卡 Overflow，NEF-only。双卡预检分别枚举各 StorageID、可写状态、剩余容量与目录基线；只有证实 Overflow 续存语义后才按可用容量合计规划，实际每张 receipt 保存其所在存储。单卡按唯一已验证的可写存储预算，不能因为没有第二卡拒绝全部单卡采集。

Nikon 手册将 Overflow、Backup 和 RAW slot 1/JPEG slot 2 区分为不同策略；本次选择 Overflow，不能把 Backup 的两份对象当作两张投影，也不能在发现其它模式时隐式沿用 Overflow 合同。[Nikon 手册，印刷页 69](https://cdn-10.nikon-cdn.com/pdf/manuals/dslr/D7100_EN.pdf)

满卡续存是预期策略变化，实际事件、对象存储和设备就绪须证明本张已保存及续存目标有效；不能只看到原槽满就宣告成功。总容量不足、续存失败、卡移除、非预期目标或未知写入结果仍关束并保留断点。不自动格式化卡、不删除照片、不改变位深和压缩来腾空间。

## 4 SD 捕获的协议与证据

### 4.1 候选技术路线

继续使用进程内 Rust WPD/PTP 和单 COM owner，不增加桥进程、驱动替换或旧相机软件回退。必要协议事实从已核验的 MIT 来源和标准取得。

现有同源 Nikon 参考区分 `0x90C0` SDRAM 与标准 InitiateCapture 的存卡路径；参考中 `0x9207` 出现在 Live View 分支。它证明存在不同路径，不能直接证明当前 D7100 的 LV OFF 存卡参数或取消语义。[MIT NikonBase 源码](https://raw.githubusercontent.com/dukus/digiCamControl/master/CameraControl.Devices/Nikon/NikonBase.cs)

执行方先核对 D7100 本次 DeviceInfo 广告：标准 InitiateCapture `0x100E`、GET_EVENT、DeviceReady、GetObjectInfo、GetStorageIDs `0x1004`、GetStorageInfo `0x1005`、GetObjectHandles `0x1007` 及相关事件。捕获 opcode、参数和目标 StorageID 只使用已证实契约；不向未知型号试发 Nikon 私码。不把 `0x90C0` 只改参数后声称存卡。

`recordingMedia=0` 作为 SD 目标，严格检查原生 descriptor、类型、允许值及鲜读；明确 Apply 写入，预检/开始/恢复只验证。目标选择与实际介质必须一致，不隐式从 SD 切回 RAM，或从 RAM 回退到 SD。现有 host_capture 能力不代表 card_capture 能力，分开协商。

双卡策略验证同时覆盖实际槽角色与跨槽续存。现有 `activeSlot` 是 raw diagnostic，不代表已核验的 0/1 槽选择契约；D1B2 等不透明值也不能用来猜测 Overflow、卡忙或保存完成。执行方须取得本机可核验的菜单/属性及对象事件证据，明确哪些可以原生读回、哪些只能操作员设置后确认。未证明的写入接口不开放，无法可靠确认 Overflow 时报告该边界并保留单卡能力，不能把双卡交付标为 PASS。

### 4.2 三类事实分开表达

| 事实 | 证据 | 用途 |
| --- | --- | --- |
| 本次曝光/捕获已结束 | SD 路径经验证的关联完成事件；标准 `0x400D` 是候选，需要证明本机时序及意义 | 正常末张/暂停可按该证据关束；计时仅用于超时 |
| 本次 SD 对象已可确认 | 唯一关联 `0x4002`、完整 ObjectInfo、符合单卡/Overflow 策略的实际 storage/目录/NEF 格式/大小、DeviceReady 与清晰关联边界 | 提交采集记录、释放同 ticket 的 CAPTURE_HOLD；不宣称已校验 NEF 内容 |
| 电脑原始数据完整 | 导入后完整长度、NEF/TIFF/CFA 与身份检查、SHA-256、flush、原子不覆盖提交 | 图像可预览、数据封存、重构可用 |

不把 ACK、对象出现、DeviceReady、预期曝光时长、指示灯推断、下载结束互相替代。当前 `C102` 只为 RAM 路径提供证据，不能拿它填 SD 的完成字段。若只能得到较晚的可靠完成事件，首期宁可在其后关束/推进；未证明的新“提前曝光结束”不进入生产。

SD ObjectInfo/DeviceReady 不提供主机内容校验，也未必等于存储介质的 fsync 保证。验收要核对事件与实际曝光、卡对象和访问灯/保存行为的关系；界面使用“Camera object confirmed”等准确语义。取卡仍按机身访问灯熄灭与厂商操作要求完成。[Nikon 手册，印刷页 58](https://cdn-10.nikon-cdn.com/pdf/manuals/dslr/D7100_EN.pdf)

### 4.3 关联和取消

仅一个 selected session、一个 in-flight capture。预检及会话/存储变化时建立所有参与存储的既有对象基线，逐张维护已知集合，不默认每张全量枚举整卡。触发前处理能力变化、鲜读固定参数、LV OFF、DeviceReady 与旧事件队列；旧照片不能成为新曝光证据。触发后允许相关事件不同顺序，但完整证据前不提交。事件缺失时不能靠新文件数量猜测曝光结束或顺序；额外对象、重复/迟到完成、错误存储、StoreRemoved、未知事件或参数漂移均拒绝该次提交并安全停止。

Overflow 的第一卡容量耗尽、第二卡接续与相关 StoreFull 通知必须有专门状态分支：经实机验证的正常续存可提交到第二卡，通知本身不证明本张成功；未开始曝光前无法确认有可用目标则不触发，触发后结果不明则安全停止并对账。不得捕获失败后盲目换卡重拍。首期每张仍只接受一个 NEF 对象，Backup 的双对象与外部快门新增对象都不进入正常计数。

物理快门和其它控制软件产生的照片会破坏关联；扫描期间要求相机控制独占，检测多对象或额外捕获后拒绝。不得只靠照片编号顺序或时间排序配角度。

Stop/OFF/EOF：优先并行关束、Nano STOP 和相机取消；关束不等待写卡。相机清理需要新的介质感知 pending capture：分别记录曝光结束、对象保存/就绪和接口关闭结果。当前 cleanup 消费 C102、使用曝光+2 秒等 RAM 约束，不能原样复用为 SD 写卡结束。新的有限等待期限由 SD 实测决定，超时仍是 UNKNOWN/isolated；取消不删除已写卡对象，迟到对象保留为待核对，不能提交给下一帧。

读取卡文件的小量元数据可经 USB 返回；扫描期间禁止 GetObject/GetPartialObject/GetThumb 图像载荷、嵌入预览下载和后台自动图片传输。控制与健康数据继续回传。

## 5 新的采集与数据事务

### 5.1 一张投影的首期顺序

```text
PC 协调器 → Nano：MOVE(ticket，目标位置)
Nano：按实际角度输出脉冲 → 最后 STEP 回低 → 等待 150 ms
Nano：进入 CAPTURE_HOLD → PC：READY_TO_CAPTURE(ticket，脉冲位置)
PC：核对 ticket/位置/状态及设备准入，持久保存拍摄意图
PC → D7100：本次 SD 曝光(captureId)
D7100 → PC：关联曝光结束 + 本张卡对象确认/就绪
PC：持久保存 receipt → Nano：CAPTURE_DONE(同 ticket)
Nano → PC：确认 IDLE
PC：提交本张记录 → 下一位置；末张进入安全收尾与 USB 导入
```

PC 不在 READY 后追加 150/500 ms 或按角度计算的 sleep。ACK、UI 动画到位、超时到期都不能触发拍摄；只有匹配本次 ticket 且位置/状态验证通过的 READY 才允许进入相机事务。普通移动等待统一为 150 ms，不因投影数量或角度自动改为 100/200/500 ms。

拍摄意图记录 acquisitionId、frame/capture/operation ID、组、索引、计划角、确认脉冲位置、固定 profile、会话及尝试状态。相机只触发一次，响应未知时不自动重拍。正常末张/暂停按可靠曝光结束证据关束；异常立即关束。任一步失败保留意图和已知 receipt，进入恢复核对，不能猜测重拍。相机和转台的消息始终由 PC 协调器关联，设备模块互不调用。

首期逐张确认对象后才进入下一投影，沿用一次一张的清晰关联。将写卡与运动重叠、允许多张未确认或后台流水化是后续优化，只有事件和缓冲/关联契约验证后另行实施。

PC 仍需现有保存根和任务目录，存放小量配置、账本与日志；“图片在 SD”不意味着删除保存路径。首期建议 `<saveRoot>/<taskId>/` 使用 `manifest.json`、`acquisition-journal.jsonl`；导入后沿用 `frames/` 和现有参考目录布局。原始卡文件名、源路径和槽信息永久保留在 provenance 中。

### 5.2 数据类型和版本

建议新增 SD profile 7 与 manifest schema 4；执行前确认版本没有被其它改动占用。旧 canonical hash 原样保留。

`CaptureResult` 使用显式变体：

- HostFile：当前 CapturedImage 的完整文件证据。
- CameraCardObject：相机对象 receipt，**没有**本地 path 和本地 SHA-256。

建议 card receipt 包含：camera identity、cameraSessionId、operationId、captureId、acquisitionId、storageId、本次可观测的槽/存储信息、handle、父目录/相对源路径、原始 filename、format、expectedBytes、可获取的原生拍摄时间、关联事件摘要、曝光结束证据、就绪/对象确认状态、计划角与确认脉冲位置。每张保存实际存储，不用项目级单一 StorageID 覆盖 Overflow 的前后两段；跨槽同名照片不能覆盖或合并。

现有 `ptp.rs::ObjectInfo` 丢弃 storage、父目录和时间，仅保留格式/大小/文件名，且有 RAM 专用容错；必须为卡对象增加独立严格解析和目录定位，不能把 RAM 的兼容放宽默认施加到 SD。

schema 4 分别保存：

- `acquisitionStatus`：prepared/running/paused/acquired/stopped/fault。
- `dataStatus`：pending_import/importing/verified/import_failed。
- `captureDestination`、冻结 card policy（single/overflow）、本次可验证的存储身份/槽证据、profile、设备与几何；新任务记录 150 ms 运动策略及固件版本/来源 hash，运行期实际存储另按张记录。
- `captureRecords`：全部参考和投影的 receipt、意图和提交状态。
- `frames/references`：只有已导入并验证的本地文件记录才进入可重构输入。
- `completed`：只在采集安全收尾与全部数据验证封存均完成时为 true。

相机对象信息不是图像 hash。不得用空 hash、全零 hash、源路径、`imageCount=360` 或虚构 NEF 填充现有文件事务。

### 5.3 任务锁与用户状态

新增“Acquisition complete · import required”。采集/设备/收尾终态和数据终态各自明确：

```text
Prepared → Front references → Projections → Safe finishing
                                              ↓
                                  Acquired / Pending import
                                              ↓
                                         Importing
                                              ↓
                                   Verified / Completed
                                              ↓
                                      Reconstruction
```

运行、暂停、未安全结束、清理未知及恢复中继续持有现有硬件任务锁。设备 OFF、当前运动释放、相机采集清理和采集账本全部确认后，任务可转为 retained `acquired_pending_import`，释放扫描硬件占用；数据依然不能重构。随后自动启动的 USB 导入另持有相机传输占用，单拍、LV、相机参数修改和新扫描不能与其争用 COM owner。取消并完成传输清理后可开始新任务，旧任务继续待导入；不把安全停止或状态读取锁住。登记文件目前只有 unfinished/completed/abandoned，必须增加版本化状态和重启加载规则，不能通过 `mark_completed` 跳过本地文件校验来解锁。

未知/损坏登记继续锁定。未完成任务的旧跨重启门禁不降低。待导入记录保存来源约束，导入成功再独立封存为 completed；多个待导入任务的目录不可覆盖或复用。明确放弃和 Stop 保留所有卡对象、导入文件及记录，不能标记为 completed。

UI 分别显示采集、USB 导入、校验计数和时间；自动导入时显示“Acquisition complete · transferring via USB”，取消/失败后显示“Acquisition complete · import required”。推荐新增 acquiredProjectionCount/importedProjectionCount/verifiedProjectionCount；现有 `imageCount` 和本地文件预览必须明确保持文件含义或全面迁移所有消费方，不能单独改后端计数。现有闭集 Phase、scanFlow.stage、前端快照校验、progress、dock、菜单、恢复和更新维护均需同步。

SD 模式采集中不显示新照片缩略图，提示“Images are on the camera SD card”。导入后恢复注册图片预览。Live View 仍是独立功能，扫描拍摄时继续要求显式关闭，不用 LV 或缩略图下载制造隐藏传输。

## 6 扫描后导入与重构

### 6.1 已选定的 USB 批量导入

1. 末张可靠结束、OFF 确认及采集安全收尾后，持久保存 acquired_pending_import，再自动启动本任务 USB 导入。使用同一 camera owner，不打开第二 COM 会话；UI 只请求导入任务 ID 和显示进度，不能提交任意 handle/图片列表作为真相。
2. 鲜读相机身份、全部参与存储和 ObjectInfo。有效原会话仍需核对 receipt；重连后重新枚举并按机身、存储证据、目录/名称、大小及可读的拍摄元数据唯一绑定，不能沿用旧 handle 或把旧 StorageID 当永久卡身份。Overflow 的前后两卡按各张 receipt 找回，不按文件名或时间排序分配角度。
3. 在已验证的 GetObject 数据传输合同下逐对象下载到任务私有临时文件。使用传输分块/进度和有界取消，累计长度、流式 SHA-256，检查传输结束响应和前后对象元数据；验证完整 NEF、D7100 身份、原生 CFA，以及与冻结设置相符的可解析参数。不得无界分配内存或在 worker 卡住时阻塞独立 OFF。
4. flush 后原子不覆盖提交到既有 frames/参考目录，持久写该帧 import receipt，再更新已导入/验证计数。重复导入只复用 hash 相同且重新验证的目标；冲突不覆盖。采集序号决定目标名称，原始卡名称和源位置保留为 provenance。
5. 全部 10 暗场、10 空场和 N 投影的一对一对应、索引、角度、长度/内容验证通过后，原子封存 completed/dataStatus=verified，释放传输占用并开放重构。原卡文件保持不变，不为腾容量自动删除。

文件名、长度和时间联合不是密码学级原图身份证。本方案首期来源保证属于受控会话及元数据关联，首次内容 hash 在导入时产生，不能声称采集时就拥有相同 hash。卡更换、跨槽重名或重连后无法证明唯一来源时，导入失败并保留待导入状态，要求来源核对；绝不补造映射。若以后要求采集时的密码学内容身份，需另找相机端摘要能力或重新界定“完全不传图”的边界。

下载能力需要单独核验 DeviceInfo 与当前 WPD 数据传输接口；不把 RAM 下载代码中的对象识别、容量假设和兼容容错直接套在 SD 文件上。GetPartialObject 只在实际广告和契约已证明时使用，USB 导入不以支持该操作作为未经验证的前提。

### 6.2 取消、续导和占用

导入是后台可取消操作。取消/掉线/掉电保留已验证文件、采集记录与导入账本；重启后按任务 ID 续导。首期以完整文件为恢复单位，未完成文件的私有临时数据可重新下载，不能把部分文件计作完成。提交文件后、写 receipt 前崩溃时，根据确定目标与完整校验恢复，不能覆盖冲突文件。

导入期间保持状态读取和安全停止可用，不执行出束、HOME 或 MOVE 来“恢复导入”。同一相机不能同时服务导入和下一轮扫描；取消并完成 camera owner 清理后才解除传输占用。多份待导入任务保留独立来源与目录，不借新的拍摄事件补旧任务数据。

进度及 ETA 来自实际字节/文件处理；路径逃逸、软链接/重解析点、目标冲突和来源变动均拒绝。USB 失败保留错误与续导入口，不静默转为逐张回传或改用其它导入方式。整组导入和校验时间单独报告，同时报告采集提速与数据可重构的总等待；扫描期间始终零图像载荷。

### 6.3 重构与既有项目

重构读取 schema 4 时要求完整导入和封存；当前 `reconstruction.rs` 仅接受 2/3，`scan_project.rs` 先检查 completed 并逐文件 hash，都需要有针对性的扩展。`open_scan_project` 对待导入项目返回明确入口，不当作损坏或完整项目。

导入文件继续使用现有 NEF 线性 CFA、暗/空场校正、几何、单色/叠层颜色和材料流程；存储迁移不改变科学算法和校准。重构缓存只在最终数据封存后生成，绑定最终 manifest 和文件 hash，不复用待导入清单的缓存。

旧 schema 2 和 profiles 1–5 保持读取、重构和合法缓存能力；不续扫、不改写。当前 schema 3/profile 6 的 RAM 任务应保留原有受控恢复路径，不能在中途转换为 SD，或仅因引入新 profile 就破坏既有合法恢复。新 SD 任务用新版本和明确的 SD 策略。

## 7 参考、暂停、恢复与故障

前暗/空场也按同一 single/overflow 策略存卡并纳入 receipt，整个任务可以跨两卡。流程仍为 10 暗 → 一次连续开束 10 空 → 关束确认 → 装样 → 投影；不增加后参考。前空场无法在单个出束上限内安全完成时关束报错，不自动循环开束补足。

Pause：本张可靠曝光结束后关束，完成卡对象/同 ticket/持久事务后进入 paused；停止发下一张。没有对象确认的结果留待核对，不能显示完整暂停断点。

Resume：新的预检、冻结 profile/品质/卡策略、已采集对象清单、当前位置或受控 HOME 均需核对。首次版本只自动恢复连续且无歧义的 committed 前缀；存在 pending trigger、receipt 未提交、缺对象、多对象、卡更换或来源不明时阻止自动继续。拍照响应超时不意味着没拍，不能直接重复该索引。

PC 崩溃后可能“卡已写、PC 未提交”。预写意图和 receipt 必须支持识别这段窗口：先在 OFF 状态完成重连和对象核对，再决定是否经明确对账接受该记录或终止该任务；没有可恢复曝光/角度证据时不自动吸收孤儿照片。原卡数据与日志保留，任何人工对账另记来源和结论。

正常最后张关束无需等待整组导入；必须取得本次 SD 可靠结束事件。安全收尾还需相机/运动/警告状态和采集账本确认，才报告 acquired_pending_import。关束 UNKNOWN 时永不报告安全采集完成。

## 8 已确定的 1000 秒扫描出束策略

目前 `lib.rs`、`device_controls.rs`、`scan.rs` 的新任务/预检/恢复/清单检查为 1–600 秒；`frontend/src/scan-input.ts` 为最多 10 分钟，准备窗口和 preview 也有对应逻辑。`xray/worker.rs::beam_on` 的手动默认期限为 600 秒；扫描通过 deadline 路径传入冻结上限，二者不能混为一个入口。

用户已明确“直接按照 1000 秒来，我问过厂家了”。本规划按该答复采用 1000 秒，不再将新增厂家确认作为离线实施的前置条件。来源记录为用户转述厂家答复；没有把它改写成已取得书面厂家资料或本系统硬件验收完成。

现有 300 秒休息是项目策略：`scan.rs` 源码明确注明不是厂家指定的五分钟冷却。公开的 70kV MagPro Rev B 手册给出散热/温度条件与至少两秒再次开束间隔，未给出当前代码的 600 秒连续上限；本次 1000 秒依据来自用户答复。[Moxtek 70kV 12W MagPro 操作手册，页 3、6](https://moxtek.com/wp-content/uploads/xray-product/TUB-MAN-1007-70kV-MAGNUM-Rev-B.pdf)

首期实施包含以下改动，并单独验收：

1. 在领域定义统一的扫描允许上限 1000 秒，建议常量 `SCAN_MAX_XRAY_SEC`；新 SD 扫描默认 1000 秒，旧任务保留冻结值，独立手动 `beam_on` 保持 600 秒默认期限。
2. 修改所有领域、DTO、UI 换算、preview、preparation checkpoint、resume、completed manifest 和测试的相关限制；不能全仓替换数字 600。1000 秒准确显示为 16 分 40 秒，DTO 使用整数秒；不能四舍五入成 17 分钟或把 16.67 分钟直接换成 1000.2 秒。建议输入秒或分/秒两栏，现有分钟字段若保留必须验证精确换算。
3. 冻结本任务上限及策略版本；旧清单保留原上限，恢复不以新默认替换。
4. 保留 Moxtek owner 的独立单调 deadline、持续联锁/温度/通信监测、UNKNOWN/OFF 门禁和确认 OFF 后两秒再次开束；camera/engine/UI 卡顿仍须截止关束。
5. 保留正常连续出束和到上限后的 300 秒休息。预算不足或故障时允许休息/失败，不为 12 分钟宣传目标越过上限。
6. 源码现有曝光/运动预算含较保守的 30 秒余量。只有实际时序与异常测试支持时再调整；不能为了避免休息直接删除余量。
7. 经明确设备授权在实际功率、散热条件下验收：1000 秒截止、温度全过程、OFF 证据、失联/联锁故障；记录具体包与设备。未做整机验收的候选标记为仅离线通过，不能标已硬件认证。

1000 秒不是“不休息”的保证。150 ms 候选的纯运动/曝光约 960 秒，叠加相机事务和现有安全余量仍可能需要两段出束。运行时按实测耗时、未开始投影预算和硬截止裁决，不按预计 720 秒跳过休息，不在截止后延长出束等待最后图片。恢复下一组仍须重新满足开束准入条件。

## 9 性能优化的范围

首期完成存卡、USB 导入和末脉冲后 150 ms READY 握手，保留元数据事务与科学门禁。性能报告区分三项收益：移除逐张下载、500 ms 改为 150 ms、600 秒改为 1000 秒带来的休息变化。RAM/SD 传输对照使用相同运动策略、曝光、负载、参考帧和上限；500/150 ms 等待对照保持其余曲线一致，另报告原配置到候选整体体验差异。

第二轮若参数查询占比显著，可消除紧邻重复读取并按有效会话缓存 descriptor；明确 Apply、能力变化、模式/依赖变化和相关通知使缓存失效。关键实际值在拍摄前仍鲜读；参数漂移、未知通知和取消回归必须继续拒绝。现有 D1B2 等不透明诊断值不改解释为“安全/写卡完成”。任何改变冻结参数检查时点的优化都需证据，不能只用定时器替代查询。

后续向 12 分钟目标推进时，针对小位移实测候选加减速和巡航预算，继续采用已确定的 150 ms 等待。建议预算分配起点：运动及等待不超过约 0.6–0.7 秒，相机事件/元数据/释放/持久化的非曝光开销约 0.3–0.4 秒；这是待测设计预算。1 秒非曝光预算必须包含全部串行步骤。首期只落实 150 ms 及正确计时锚点，进一步改曲线由实测决定。不可在曝光结束证据前旋转。

固件修改须保持普通负方向、HOME 有界反向例外、20 μs 脉宽及方向建立、安全停止、心跳、位置/微步约束、CAPTURE_HOLD 与同 ID 释放。固件构建配置与来源/hash同步。现场分别验收负载无失步、角度重复性、图像振动和首末稳定；源码估算和 host harness 不能证明 AVR ISR 或机械质量。不能为了赶时间跳过稳定等待、加速期间拍摄或使用计划角冒充设备证据。

如果这三轮仍不能满足目标，交付实测限制与主要耗时；不临时增加相机硬件触发线、改电路、连续旋转曝光或换设备作为隐含方案。

## 10 分阶段实施和验收交接

| 阶段 | 工作与产物 | 进入下一阶段的条件 |
| --- | --- | --- |
| P0 合同冻结 | 采用第 1 节已确认边界，冻结 SD/USB、单卡/Overflow、1000 秒、150 ms、版本、计数/终态/锁与兼容矩阵；保留当前分支改动 | 软件合同和待实机证明的协议点有明确记录，设备动作仍单独授权 |
| P1 协议与固件候选 | 介质能力/对象解析、SD receipt、取消清理；固件统一 150 ms 和末下降沿起算，保留脉冲曲线、HOME 与停止 | 离线零图片传输及关联/取消测试通过，固件 build/harness PASS；真实协议结论由 P2 取得 |
| P2 无射线台架验收 | 单卡/Overflow 触发、结束与续存；转台不同角度末脉冲至 READY、150 ms 图像质量、Stop/心跳；不对原卡做破坏性满卡试验 | 指定相机/转台与候选包证据齐全，PC 无采集中图片载荷；未证明部分 BLOCKED |
| P3 扫描与策略集成 | 参考和投影双目标、schema/profile、拍摄意图、READY→曝光→CAPTURE_DONE→下一 MOVE、恢复/锁/UI；1000 秒限制及多投影数 | 生产协调器离线全链与故障注入 PASS，RAM 旧路径与冻结值不退化 |
| P4 USB 导入和重构 | 双卡对象统一导入、续导/来源校验、封存、项目打开/重构/预览、传输占用和取消 | 380 对象正确映射、跨槽重名/冲突/取消 PASS；指定相机 USB 下载及旧项目/叠层链验收通过 |
| P5 射线与全链验收 | 对 P3 的 1000 秒策略验收，保留两秒再次开束、独立监测和 300 秒休息；验证采集至重构整链 | 对应源/功率/散热的连续运行、截止与故障关束、单卡和 Overflow 联动证据齐全 |
| P6 首期对照实测 | 记录 500/150 ms 固件与 RAM/SD 条件，区分等待/下载/休息收益；固定其它曲线和品质，测 360 张及导入/数据就绪时间 | 对应实机授权，150 ms 持续采集不丢帧/不错角/无不可接受振动，报告实际收益；不要求 720 秒 |
| P7 候选交付 | 构建来源/哈希、离线门禁、桌面启动退出、设备分项和全链记录、操作与导入指南 | 只有完整射线联动验收通过后才可标该候选已认证；正式入口保留现有可核验版本 |
| P8 后续 12 分钟优化 | 根据 P6 瓶颈，择需优化参数查询或进一步优化运动曲线，保留 150 ms 规则并重复质量验收 | 独立后续阶段，不阻塞首期迁移交付；真实设备动作按对应授权执行 |

先完成 P0/P1/P3/P4 能独立准备的离线合同、实现和门禁，再展示具体候选与 P2/P5/P6 的设备验收动作申请授权；表中阶段划分不要求在离线实现中途停下等待设备。依赖真实事件意义的最终生产开放等待 P2，完整射线联动等待 P5。P3/P4 是完整可用迁移的组成部分，不能交付“会拍卡但数据无法可靠重构”的完成结论。实机验收若暂未获授权，交付可核验的离线候选与具体待执行验收步骤，明确 BLOCKED，不覆盖正式启动入口。

### 10.1 文件修改范围

- 相机：`types.rs/ptp.rs/nikon.rs/actor.rs/properties.rs/mod.rs/tests.rs`；现有 RAM `capture.rs` 保留可核验行为，可在同设备目录新建 `card_capture.rs`、`storage.rs`，同步模块公开接口、README 和许可来源。
- 引擎：`capture_profile.rs/scan.rs/device_controls.rs/lib.rs/task_registry.rs/operations.rs`；导入可放现有领域顶层新 `.rs` 文件，登记 `modules.json`；无需新增领域目录。
- 数据消费：`scan_project.rs/reconstruction.rs`，必要的缓存/预览准入；保留当前 `reconstruction_*` 与材料处理改动，不做算法重构。
- 前端：类型和两 adapter、App 快照校验和计数/状态、相机草稿/保存目标、准备/恢复/导入入口、图像媒体、重构准入、英文错误和菜单。UI 文案全英文，交接文档中文。
- Tauri：复用保存根 native folder picker 与受限 engine 转发；USB 导入由引擎完成，不要求选择读卡器目录。媒体 IPC 只读取导入后已登记且 hash 验证的文件，禁止任意 renderer path 预览和新增相机进程。
- 运动与出束：首期修改 `firmware/rts9060_nano/rts9060_nano.ino` 的普通移动等待及末脉冲计时锚点，检查 Nano driver/planner 与扫描消费方是否存在重复等待；P3/P5 落实扫描上限与必要 deadline 消费方。HOME、脉宽/方向建立、加减速/巡航和警示灯时序保持现有语义，不全仓替换数字 500/600/300；P8 才进一步优化运动曲线。
- 文档：当前架构、执行流、能力矩阵、强依赖、模块和错误码、设备 README 及适用稳定规则同步；实施后用明确 SD 目标规则替代“仅 RAM”的描述，继续禁止静默回退。进度与证据记录归 validation/本地测试输出。

项目允许在现有职责内新增文件；新顶层目录、子系统目录、驱动/权限/进程、接线或电路变化需单独明确授权，不能借本规划预建。

### 10.2 必须覆盖的测试

| 类型 | 用例及通过条件 |
| --- | --- |
| 能力/介质 | 未广告 SD/下载操作、错误型号、错误 descriptor/值、无卡、只读、容量未知/不足均拒绝；64 位容量解析和单位无溢出；不试发未知私码 |
| 单卡/Overflow | 单卡正常；双卡逐存储容量、角色读回与基线；第一卡满后有效续存且 receipt 改记第二卡；总满/续存失败停止；同名对象不合并；Backup/分卡 JPEG 拒绝 |
| 存卡目标 | 完整参考+投影链没有 GetObject/GetPartialObject/GetThumb 或图片落盘；不能先下载再删除冒充零传输 |
| 事件关联 | 对象/完成两种顺序、迟到和重复、旧照片、额外对象、物理快门、未知事件、跨 session/generation 均有明确结果；无虚假 ExposureEnded |
| 150 ms 握手 | 最后 STEP 高电平期间不计时；下降沿后未满 150 ms 不发 READY，满后进入 HOLD 且只发一次；不额外等尾周期或 PC sleep；零位移/不同角度、计时回绕、ISR 时间戳读取、149 ms 时 Stop/掉线均不误曝光 |
| 运动事务 | 曝光及对象证据齐全才释放同 ticket；ACK/计时到期/错 ID/迟到 READY/未 IDLE/位置不符不推进；投影数 180/360/720 覆盖脉冲取整与唯一索引，无重复端点或末张多转 |
| 崩溃恢复 | 意图后/trigger 后/对象确认后/CAPTURE_DONE 前后/manifest 提交前后分别中断；不盲目重拍或吸收不明照片 |
| Stop/Pause | 各捕获和写卡阶段立即独立 OFF；相机清理未知保持隔离；pause 只在完成本次事务后终态；重复 Stop 的迟到事件不复活动作 |
| 数据与登记 | acquired_pending_import 重启保留、解除扫描占用与独立 USB 占用、导入与单拍/LV/新扫描互斥、UNKNOWN 清理不解锁；多个待导入项目不覆盖、放弃保留原图 |
| USB 导入 | 两卡正确 380 张、重名/跨文件夹/计数回卷、会话重连与旧 handle 失效、错误卡/缺失/多候选、截断/长度错误、NEF 格式/身份、传输结束错误、目标冲突、续导/取消、原子提交窗口和路径逃逸 |
| 重构与预览 | 待导入不可重构/图片预览；完整导入才就绪；旧 schema 2/3/hash/缓存以及单色、叠层颜色与材料链保留 |
| 射线 | 两秒再次开束不被重连绕过；前空整组连续、正常投影连续；1000 合法、1001 拒绝、整数秒/分秒显示、旧 600 冻结值与手动默认、300 秒休息；camera/engine/UI 卡住仍独立 deadline OFF；离线边界用受控时钟 |
| 性能/质量 | 区分 150 ms、SD 与上限变化，持续 360 张测分布和总时间；覆盖小角、1° 和较大角度的负载/图像稳定性，保留对照；150 ms 若未通过质量验收则报 FAIL，不静默扩大等待；另报 USB 与数据就绪时间 |

离线门禁按影响模块执行，至少：

```powershell
npm.cmd run module-map:check
npm.cmd run typecheck
npm.cmd run build
npm.cmd run test:frontend
npm.cmd run test:desktop
cargo test --manifest-path backend/Cargo.toml --workspace
```

首期包含固件变化，必须追加 `npm.cmd run firmware:build` 与 `npm.cmd run firmware:test`，重点扩展 `tests/firmware/host_harness.cpp` 的 150 ms/末下降沿/停止与回绕用例。外部 `tests/` 本机存在但不跟踪，执行方应核对是否具备；不能把缺套件说成通过。重构数值、ISR 调度、卡写入持久性和真实射线动作的结论须分别明确证据范围。

### 10.3 完成标准

完整首期 SD 迁移必须同时满足：扫描循环零图像传输；普通移动末脉冲结束后统一 150 ms READY 握手且通过负载/图像验收；科学品质和关联门禁保留；指定 D7100 的单卡、双卡 Overflow 存卡及结束证据验收；N 张投影及 20 张参考的可追溯账本；暂停/Stop/重启不丢映射；USB 导入、hash 与重构可用；1000 秒策略及独立保护一致；旧任务和成果保留；实测报告、指南和候选来源可核验。

1000 秒策略已由用户选定，软件实现及对应硬件验证属于首期范围；“12 分钟达标”是后续独立验收结论，未达标不阻塞已满足上述标准的迁移交付。分别报告迁移可用性、实测提速、1000 秒联动和后续目标的 PASS/FAIL/BLOCKED。不得因离线测试通过给 hardwareCertified 置 true，不自动覆盖现有正式启动入口或安装发布。

## 11 可直接复制的精简实施提示词

请在 `E:/LZU_Master/MicroCT/Micro-CT-App` 按 `docs/decisions/0004-camera-sd-card-acquisition-plan.md` 实现以下已确认方案。先读适用 AGENTS.md、当前源码和工作树差异，保留已有改动，在现有工作目录完成。

1. **存卡与导入**：D7100 唯一扫描相机，64 GB 卡，支持单卡和双卡 Overflow。扫描只回传控制/对象元数据，全部 NEF 留在卡上；安全关束收尾后通过 USB 批量导入，长度/NEF/SHA-256 校验及原子封存通过才可重构。保留现有科学品质和 10 暗场/10 空场。
2. **统一 150 ms**：所有普通移动完成目标脉冲、最后 STEP 回低后，固件等待 150 ms，再进入 CAPTURE_HOLD 并发 READY_TO_CAPTURE(ticket，脉冲位置)。不同角度与投影数共用此规则，零位移 MOVE 同样等待 150 ms；计时从末下降沿起算，避免额外尾周期。保留 HOME、加减速/巡航、方向、脉宽、心跳与 Stop。PC 不追加固定或按角度的 sleep，ACK/超时不能代替 READY。
3. **PC 协调握手**：MOVE → READY/位置和状态验证 → 保存拍摄意图 → SD 曝光 → 本次可靠曝光结束及对象/就绪确认 → 保存 receipt → 同 ticket 的 CAPTURE_DONE → IDLE → 提交本张记录 → 下一 MOVE。设备模块互不调用。按实际投影数规划角度与脉冲取整，不写死 1°/267 脉冲，不在末张额外转动；首期逐张确认后推进。
4. **射线策略**：扫描上限 1000 秒，新 SD 任务默认 1000，旧任务保留冻结值，手动默认 600。保留正常连续出束、独立 deadline/联锁/温度监测、确认 OFF 后至少两秒再次开束，以及到限 300 秒休息。故障/Stop/UNKNOWN 优先关束，不按预计总时长跳过保护。
5. **完整事务**：显式区分 HostFile 与 SD receipt，扩展版本化清单/profile、待导入状态、任务锁、续导、恢复、UI/preview 和重构消费方。保留旧项目与合法 RAM 恢复。错/迟到事件、触发结果未知、跨槽重名和重连对象身份必须可靠处理；不使用空 hash、假文件、计时猜曝光结束或盲目重拍。
6. **验收与交付**：先完成可独立执行的离线实现、相关门禁及 firmware:build/firmware:test，再展示候选和具体设备验收动作取得真实动作授权。验证不同角度、150 ms、Stop/回绕、单卡/Overflow、USB 续导、旧项目和持续 360 张的质量/时序。150 ms 表示固件预留，不冒充负载实测停稳；未过质量验收报告 FAIL。首期报告实际提速，12 分钟为后续目标。交付 PASS/FAIL/BLOCKED、命令、构建来源/hash 与剩余边界；未验收候选标“仅离线验证”，保留正式入口和原始数据。

## 12 本次规划核对结果和来源

| 检查 | 结果 | 范围 |
| --- | --- | --- |
| 活动系统设计与采集/恢复/重构调用链 | PASS | 源码、结构化图谱、现有测试和厂商参考的离线检查 |
| `npm.cmd run module-map:check`，写入规划前 | PASS | 覆盖 242 文件、16 模块、39 个固定目录边界；模块图谱为当前状态 |
| `npm.cmd run module-map:check`，写入规划及导航后 | PASS | 覆盖 243 文件、16 模块、39 个固定目录边界；模块图谱仍为当前状态 |
| 用户答复纳入后的模块图谱、文档格式与交接基线 | PASS | 243 文件/16 模块/39 边界；12 节完整、导航及 4 个关键源码 hash 核对，无过时的导入方式、存储限制或上限假设 |
| 固件逐脉冲累计与 150 ms 计时锚点 | PASS | 原 500 ms 的 1° 约 2.034 秒；末下降沿起算的 150 ms 候选约 1.672 秒，360 张/1 秒曝光约 16.0 分钟；均为离线推算 |
| 150 ms READY 握手与负载图像质量 | BLOCKED | 已纳入首期实施；当前固件仍为 500 ms，本次未修改/上传或执行真实运动 |
| D7100 SD 触发/事件/写卡语义 | BLOCKED | 未做本次真实相机动作 |
| USB 导入和双卡 Overflow 全链 | BLOCKED | 本次仅规划，未实施或做真实下载/续存验收 |
| 1000 秒、USB、单卡/Overflow、150 ms 与首期优先级 | PASS | 已记录用户明确答复；1000 秒来源为用户转述厂家咨询，非本次硬件认证 |
| 实测提速与后续 12 分钟目标 | BLOCKED | 尚无本次持续实测；当前运动预算冲突，12 分钟不作为首期门槛 |
| 当前整机 1000 秒连续出束与完整联动 | BLOCKED | 未实施对应策略，未获本次真实设备动作授权或完成验收 |

**仅离线验证。** 本次没有运行源码变更后的构建或全量测试，因为没有实施运行时代码变更；当前工作树原有功能也不由本次规划认证。

厂商与协议来源：

- [Nikon D7100 官方下载页](https://downloadcenter.nikonimglib.com/en/products/27/D7100.html)及[英文手册](https://cdn-10.nikon-cdn.com/pdf/manuals/dslr/D7100_EN.pdf)：印刷页 58 缓冲与访问灯、69 双卡、240 文件编号、348 容量。不要用其 1/250 秒条件下的连拍帧率保证 1 秒曝光的 CT 节奏。
- [Moxtek 70kV 12W MagPro Rev B 手册](https://moxtek.com/wp-content/uploads/xray-product/TUB-MAN-1007-70kV-MAGNUM-Rev-B.pdf)：页 3 散热/温度与页 6 两秒再次开束。
- [digiCamControl MIT NikonBase 原始源码](https://raw.githubusercontent.com/dukus/digiCamControl/master/CameraControl.Devices/Nikon/NikonBase.cs)：存卡/RAM 入口和事件的参考；本机相应文件在 `Related_files/digiCamControl-master/CameraControl.Devices/Nikon/`，不运行厂商工具，不移植其额外库。
- [Microsoft WPD MTP 扩展接口](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/supporting-mtp-extensions)：继续使用现有原生传输能力。

本次关键源文件 SHA-256 用于交接定位，不代替执行时源码阅读：

| 文件 | SHA-256 |
| --- | --- |
| `backend/ct-engine/src/scan.rs` | `FE217FCE1D530365E19A8267EDDCA3E604D721900FD9143760CE69375E831333` |
| `backend/ct-engine/src/devices/camera/capture.rs` | `C18205A6E1F09A9C17914F97E8DB4F7D550A85E51DB6022E6069D59B76C3DC1D` |
| `backend/ct-engine/src/devices/camera/actor.rs` | `7AAECFF0EA67791DF1D827EBBAA2CFACC2022778D91A35B621964BB05588A5E9` |
| `firmware/rts9060_nano/rts9060_nano.ino` | `ECADF6220E53954CB303E0E40FB85710F73735EBE5271A127C8A4BA93BD84423` |

Nikon 官方手册本次读取副本保存在 `.cache/debug/camera/sd-planning-20261010/D7100_EN.pdf`，SHA-256 为 `66B8D2425F5EEA7C2E942BA4F6048ED976DF5214A776C8CEA7E31EAC63B04A12`。缓存不提交，在线来源链接保留在本规划中。

## 13 实施进度记录（2026-10-10，仅离线）

本节只记录执行状态与证据，不改变第 1–12 节已冻结的边界与结论；分支 `native-device-controls-offline`。所有提交都先跑完整离线门禁再提交。

| 提交 | 覆盖 | 内容 | 门禁证据 |
| --- | --- | --- | --- |
| `e209ed1` | P1 / P3 子集 | 固件 2.2.0：普通移动 150 ms 从**末 STEP 下降沿**起算并消除隐含尾周期，零位移同等待；扫描上限 `scan::SCAN_MAX_XRAY_SEC=1000` 统一十处判据，前端改 mm:ss 精确换算（1000 s = 16:40），旧冻结值与手动 600 秒不变 | `npm run verify` 退出 0；host harness 8443 断言；AVR flash 14160 / RAM 1476 |
| `f5f6404` | P1 | `cardCapture/cardOverflow` 能力分离（缺广告即关闭）、`require_card_destination`（只认鲜读 `recordingMedia=0`）、`storage.rs` 严格存储解析与 `bind_storage_ids` 位置绑定、`ptp.rs` 严格卡对象解析、存卡事务骨架 | `verify` 0；module-map 245/16/39 |
| `ed7fcff` | P3 | schema-7 SD profile：`CaptureDestination`、`CardCapturePolicy`（150 ms 契约 + 固件 version/build/源哈希）、`scan_camera_issues_for(destination)`；`require_current_policy` 同时接受 6 与 7，1–5 仍只读，旧 profile 哈希不变 | profile 16/16；`verify` 0 |
| `17eec7b` | P3 | schema-4 卡清单：`CardCaptureRecord` 保留卡对象原始身份、显式数据状态词表、完成规则要求**已封存导入**（安全收尾但导入未完成绝不报 completed） | `verify` 0；cargo 398+2+56 |
| `e1cf968` | P1 修正 | 修复存卡事务**不可达**：改用卡向冻结读回；触发仅一次标准 `InitiateCapture`；唯一数据读取是 64 KiB `GetObjectInfo` 元数据；23 项剧本测试，含触发后 opcode 日志级"零图像载荷"证明 | `verify` 0；cargo 401+2+56 |
| `8608971` | P3 | `acquired_pending_import` 版本化状态：安全收尾后离开 active 槽（释放扫描占用与手动锁）但目录保留不可复用；`mark_acquired_pending_import` 要求完整卡证据；`mark_import_verified` 只在清单已证明封存导入时封为 completed | registry 14/14；`verify` 0 |
| `f7495c8` | P1 修正 | `confirm_runtime_diagnostic_for` 目的感知：卡会话用 `recordingMedia=0` 确认 0xD1B2 运行时状态，RAM 路径委托为原值 1 不变 | `cargo test --workspace` 退出 0 |

**已记录的教训**：存卡事务曾复用 SDRAM 专用的 `profile_readback` 与 `confirm_runtime_diagnostic`（二者硬要求 `recordingMedia=1`），使卡路径永不可达，而 9 个纯状态机单测仍全绿。因此本迁移的验收要求是"端到端剧本测试 + opcode 级零图像载荷证据"，不接受仅有纯单测通过。

**仍待完成（离线）**：B4c 扫描协调器存卡接线与 schema-4 写入器（参考与投影同目标、拍摄意图预写、卡任务默认 1000 秒、安全收尾后登记 `acquired_pending_import`）、B5 USB 批量导入与校验封存、B6 前端状态/计数/UI 与重构准入、B7 文档同步与全量门禁交付报告。**真实设备项（P2/P5/P6）仍为 BLOCKED**，需另行授权后按第 10 节步骤验收。
