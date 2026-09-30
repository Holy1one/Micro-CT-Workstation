# 扫描流程与重构预览修改审阅报告

> 历史记录：本文描述 2026-09-28 的候审方案。当前流程已取消后参考拍摄；以 `docs/architecture/execution-flows.md` 和源码为准。

日期：2026-09-28  
状态：2026-09-28 的阶段性候审记录，已由 `SCAN-RECONSTRUCTION-RELEASE-0.8.0.md` 接续；下文验证状态仅反映当日，不代表 0.8.0 最终结果。

## 一、提交范围与文件所有权

本轮在同一工作目录内分配互不重叠的编辑范围；主智能体负责跨模块契约和集成。

| 范围 | 负责文件 | 内容 |
|---|---|---|
| 生产采集协调 | `crates/ct-engine/src/lib.rs`、`crates/ct-engine/src/scan.rs` | 几何输入、四组参考帧、操作员门控、断点、manifest v2、重构任务入口 |
| 原生重构 | `crates/ct-engine/src/reconstruction.rs`、`crates/ct-engine/Cargo.toml` | RAW 解码、实测校正、CPU FDK、科学体、预览缓存 |
| 前端呈现 | `src/App.tsx`、`src/styles.css`、`src/scene/ReconstructionPreview.tsx` | 扫描弹窗、3D View 切换、底部重构入口、四视图和计算进度 |
| 契约和桌面集成 | `src/engine/types.ts`、`tauriAdapter.ts`、`workstationAdapter.ts`、`src-tauri/src/main.rs` | 类型、预览适配、只读体数据 IPC 与文件校验 |
| 配套 | `tests/preview-scan-lifecycle.test.mjs`、模块图谱与职责文档 | 旧预览测试构造兼容、模块所有权和调用边登记 |

三类生产设备底层实现 `devices/xray/`、`devices/camera/`、`devices/turntable/` 均未修改。`Related_files/Test/` 仅作为只读参考，没有复制其生成物进生产目录。工作树中另有与本轮无关的未跟踪文件 `docs/HALL-HOME-VALIDATION.md`，不属于本轮修改。

## 二、采集状态流

| 顺序 | 引擎动作 | 界面提示和确认 | 断点/安全边界 |
|---|---|---|---|
| 1 | 完成原预检后登记几何参数；恢复已有任务先核对原 manifest | **几何参数**：输入焦点至转轴、转轴至屏幕、屏幕直径、水平/垂直中心偏移（屏幕平面 mm）、转向、水平镜像、实测/估计 | 取消写准备阶段 checkpoint；扫描前仍需既有 HOME、安全门控 |
| 2 | Start 后等待前参考确认 | **拍摄参考帧**：移走样品；暗场 10 张后开束拍空场 10 张 | 前暗场逐帧提交；前空场逐帧受出束监控，单张后关束确认 |
| 3 | 前空场完成并关束 | **放入样品**：射线已关闭，放好后确认开始扫描 | 只有实测关束状态为 OFF 才可确认取放；引擎等待命令 |
| 4 | 执行既有投影事务 | 期间没有新增操作员步骤 | MOVE/到位、警告、射线读回、主机 NEF、CAPTURE_DONE 和原有冷却/暂停约束保留 |
| 5 | 投影完成、末角核对并关束 | **移走样品**：射线已关闭，移走后确认 | 操作员确认后才进行后参考；没有“后暗场确认”弹窗 |
| 6 | 后空场 10 张，关束；后暗场 10 张 | 无独立后空场/后暗场弹窗 | 文件数量、大小和 SHA-256 对账后封存 manifest v2，完成才解锁重构 |

上述四个弹窗的取消、Esc、关闭都进入 `cancel_scan_stage`；工作线程尝试 Nano STOP、射线 OFF 和警告 OFF，并将当前阶段写入断点。操作弹窗显示前与确认后均调用现有射线底层接口核实 OFF，等待期间每 250 ms 读取实测状态；关束不等待用户确认。前参考弹窗一次授权前暗场与前空场；取样弹窗一次授权后空场与后暗场。

输出目录新增 `references/pre-dark|pre-flat|post-flat|post-dark/frames`，每组按序记录 10 张 NEF 的文件名、大小、SHA-256 和曝光；`manifest.json` 记录几何、阶段、投影及参考帧。manifest 先写同目录暂存文件并 `sync_all`，Windows 用 `ReplaceFileW` 原子替换。恢复前仍需重新预检、必要 HOME、任务参数和文件哈希校验。

## 三、重构与界面

- 扫描封存不会启动计算。首次点击 3D View 右上角“重构”或底部日志区的“重构”入口时选择方法；只有选择后，`ct-engine` 才启动后台任务。
- 当前实现可选 **CPU FDK**：原生 `rawloader` 解码线性蓝色 CFA，前后暗场/空场插值校正，圆轨道锥束 FDK，保存 256³ signed float32 科学体、覆盖掩膜与 128³ uint8 预览。体素间距是数值采样间距，不是实测空间分辨率。
- `result.json`、`preview.json` 与数据文件记录哈希；体数据、覆盖掩膜和预览文件写盘同步后，才以 `result.json` 作为完成标记，发布前重核扫描 manifest。重复选择同一方法时先核对扫描 manifest 与缓存哈希。桌面 `reconstruction_preview(method)` 只从引擎登记的已完成缓存读取并验证路径、大小、哈希。
- 3D View 中左上 XY、右上 XZ、左下 YZ、右下可拖动三维体；三轴切片联动，三维可切换 MIP/半透明；无 WebGL2 时保留真实体数据的静态回退。计算进度弹窗不可关闭、可最小化，完成后自动消失。
- 基本采集页结构保留；底部 Image Preview 下新增 Reconstruction 入口，右侧区域显示任务、方法、状态、参考帧计数与缓存摘要。浏览器开发预览没有真实 RAW 数据，因此不解锁生产重构。

## 四、未完成能力与审阅风险

| 等级 | 项目 | 当前状态与影响 |
|---|---|---|
| 高 | GPU 探测、预计耗时比较与快 15% 才用 GPU | **未实现**。当前 CPU FDK 始终走 CPU，即使机器有 GPU；不能宣称已满足 GPU 自动选路。 |
| 高 | SIRT、CGLS | **未实现数值后端**，方法在 UI/引擎均置灰，避免假计算。原计划的多方法能力仍待补齐。 |
| 高 | 编译与 Rust/TS 类型闭合 | 按用户要求完全未编译、未运行 typecheck/单测；新增 `rawloader` 尚未写入 `Cargo.lock`，Rust 类型、Windows FFI、Tauri IPC 和浏览器布局均待最终验证。 |
| 高 | 真实设备安全与数值正确性 | 仅离线源码审阅。未连接 Moxtek、D7100、Nano；真实参考帧时序、D7100 NEF 解码、FDK 体模和屏幕几何没有本轮硬件/数值证据。 |
| 中 | 中断恰好落在相机文件写完、manifest 尚未提交之间 | 恢复校验会拒绝未登记文件并要求人工核查；不会静默跳过或覆盖该帧。 |
| 中 | 准备阶段取消 | 几何输入弹窗取消会写入 `preparation-checkpoint.json` 记录任务参数与阶段；几何值尚未确认，因此重启后仍需重新预检并输入几何参数。 |
| 中 | FDK 适用角度 | 当前重构要求至少 3 个、等间距完整圆周、无重复终点；其他采集参数仍可采集，但重构将报告角度不适用。 |
| 中 | 预览性能与视觉 | 128³ JSON 数组通过 Tauri IPC 到前端，体渲染和固定画幅是否流畅、是否遮挡现有控件须在编译后实测。 |
| 中 | 缓存命中与原始文件变化 | 再次打开时校验 manifest 与派生文件哈希以保持快速加载；不会重新散列全部 NEF。若封存后的原始 NEF 被外部修改但 manifest 未变，既有缓存仍可能显示旧结果。 |

## 五、已做静态检查和待批准门禁

| 状态 | 检查 | 结果 |
|---|---|---|
| PASS | `git diff --check` | 没有 diff 空白格式错误；未证明语义正确。 |
| PASS | `npm.cmd run module-map:generate`、`npm.cmd run module-map:check` | 203 个文件归属、15 个模块及 26 个冻结目录边界核对通过，模块图为当前版本。 |
| PASS | `module-map/*.json` 解析 | 三个 JSON 声明可解析。 |
| PASS | `npm.cmd run impact` | 识别 engine、桌面、前端、scene、文档及发布影响；仅做影响分析。 |
| BLOCKED | `cargo test --workspace`、`cargo test -p ct-engine`、`npm.cmd run typecheck`、`npm.cmd run test:sites`、`npm.cmd run build` | 用户要求先审阅后最终编译，尚未执行。 |
| BLOCKED | `rustfmt --check` | 本机缺少 rustfmt 组件；未安装。 |
| BLOCKED | 真实设备试验与重构数值验收 | 本次没有真实设备动作授权与编译后运行证据；仅离线审阅。 |

## 六、建议审阅顺序

1. `crates/ct-engine/src/scan.rs` 与 `crates/ct-engine/src/lib.rs`：优先看弹窗门控、开关束、取消/恢复、清单封存和重构解锁。
2. `crates/ct-engine/src/reconstruction.rs`：确认几何单位、RAW 通道选择、暗空场校正、FDK 近似与输出边界。
3. `src/App.tsx`、`src/scene/ReconstructionPreview.tsx`、`src/styles.css`：确认弹窗文案、底部入口和四窗格布局。
4. `src-tauri/src/main.rs`、`src/engine/types.ts`、`src/engine/workstationAdapter.ts`：核对 IPC、字段与浏览器开发预览的隔离。

审阅通过后，先补锁文件并运行上述必需门禁，修复编译与测试暴露的问题；再进行离线 D7100 样本/体模数值验证。真实设备验证须另获明确授权并核对端口、参数及现场条件。GPU 与迭代算法完成前，本报告所列功能缺口保持未完成状态。
