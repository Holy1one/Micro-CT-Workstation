# Micro-CT 0.8.0 扫描与重构版本审阅报告

> 历史记录：本文描述 2026-09-29 的实现。当前流程已取消后空场、后暗场及结束时的整批文件复核；以 `docs/architecture/execution-flows.md` 和源码为准。

日期：2026-09-29。状态：已完成源码、离线自动验证和 Windows 便携版构建；未连接或操作 CT 实机设备。

## 改动与边界

| 范围 | 本版结果 |
|---|---|
| 采集协调 | 预检后输入并保存几何；前暗场、前空场各 10 张；确认装样后执行原投影核心；确认取样后拍后空场、后暗场各 10 张。取消、故障与关束失败保留阶段断点，恢复逐文件核对 SHA-256。操作弹窗显示前、等待期间和确认后核实射线实测 OFF。 |
| 重构 | 仅扫描清单封存后解锁。CPU FDK 为基线；兼容 GPU 上运行真实 WGSL FDK，首帧与 CPU 比对并测时，预计至少快 15% 才采用 GPU，失败时从头用 CPU 重算。小规模扫描可选 64³、5 轮 CPU SIRT；超过计算预算置灰。CGLS 未实现，保持禁用。 |
| 界面 | 3D View 内没有重构入口。底部 Reconstruction 区是选择方法、打开结果、返回设备场景及控制三维显示的唯一入口。首次计算显示可最小化且不可关闭的进度弹窗；完成后自动关闭，后续复用校验过的缓存。 |
| 版本 | Cargo、npm、Tauri 和界面版本统一为 0.8.0；`Cargo.lock` 已更新。 |

射线、D7100 相机和 Nano 转台的生产底层实现没有修改。`Related_files/Test/` 只读参考，未复制实验生成物进入生产代码。无关未跟踪文件 `docs/HALL-HOME-VALIDATION.md` 未纳入本轮。

### 改动文件与控制链

| 控制链位置 | 改动文件 | 作用 |
|---|---|---|
| 扫描与重构领域内核 | `crates/ct-engine/src/lib.rs`、`scan.rs`、`reconstruction.rs`、`fdk.wgsl`、`Cargo.toml` | 扫描阶段和断点归引擎管理；封存后读取扫描数据进行离线重构。 |
| 桌面 IPC | `src-tauri/src/main.rs` | 把结果文件以受限只读接口交给界面；不控制设备。 |
| 前端契约与界面 | `src/engine/types.ts`、`tauriAdapter.ts`、`workstationAdapter.ts`、`src/App.tsx`、`styles.css`、`scene/ReconstructionPreview.tsx` | 显示引擎快照、操作弹窗和底部唯一重构入口；四视图只读呈现。 |
| 版本与依赖 | `Cargo.toml`、`Cargo.lock`、`package.json`、`package-lock.json`、`src-tauri/tauri.conf.json` | 统一 0.8.0 版本并锁定重构依赖。 |
| 契约测试与架构资料 | `tests/console-feedback.test.mjs`、`engine-session-evidence.test.mjs`、`fault-banner-safety.test.mjs`、`preview-scan-lifecycle.test.mjs`、`first-launch-response.jsonl`、`module-map/modules.json`、`edges.json`、`docs/architecture/`、相关 README 和本报告 | 对齐新快照、文件归属和执行流程。 |

控制链为 React 操作弹窗与底部入口 → Tauri IPC → `ct-engine` 命令和扫描协调器 → 既有设备适配器；重构在扫描清单封存后由 `ct-engine` 离线读取文件，经 Tauri 只读接口返回预览数据。设备适配器仍独占实际 I/O。

## 验证

| 状态 | 命令或检查 | 结果 |
|---|---|---|
| PASS | `cargo test -p ct-engine --offline` | 66 项通过，含重构几何、SIRT 算子与体模、GPU 选路阈值、扫描恢复及既有设备模拟测试。 |
| PASS | `cargo test --workspace --offline` | 引擎 66 项、桌面 15 项通过。桌面测试将 `LOCALAPPDATA` 指向仓库内 `tmp/tests/desktop`，避免沙箱拒绝写用户配置目录。 |
| PASS | `npm.cmd run typecheck` | TypeScript 类型检查通过。 |
| PASS | `npm.cmd run test:sites` | 站点构建与 100 项测试通过。首次启动测试使用本版 `ct-engine` 无设备 JSONL 快照。 |
| PASS | `npm.cmd run module-map:check`、`git diff --check` | 206 个文件归属、15 个模块、26 个冻结目录边界与模块图检查通过；无 diff 空白错误。 |
| PASS | 1920×1080 浏览器预览截图 | 主界面与底部 Reconstruction 入口可见，3D View 内无重构入口。截图仅验证未扫描状态布局。 |
| PASS | `npm.cmd run portable:build` | 成功生成 `portable-release/micro-ct-workstation-portable.exe`；SHA-256：`bb7a8b8f4a6d2e93949b48d6aff9e9e38d40ebb0bc5de49d8e44f04b26a118e5`。 |

便携版构建时工作树包含未提交改动，`build-info.json` 因此标记 `sourceDirty=true`。EXE 与构建元数据为被忽略的本地交付产物，未进入版本控制。

## 尚待实机和真实数据验证

- 没有连接 Moxtek、D7100、Nano，也没有执行 HOME、转动、曝光或开束。射线安全结论仅来自离线逻辑与设备模拟测试。
- 活动工作区内没有可用于本轮完整验证的 D7100 NEF 数据组。`rawloader` 对实拍 NEF、真实暗空场校正、FDK/SIRT 图像质量与几何精度尚未用本机真实扫描数据验收。
- GPU FDK 已编译且有 CPU 数值比对和失败回退逻辑；本轮未用真实扫描数据触发 GPU 计算，因此显卡驱动兼容、实际加速和 GPU 结果图像仍待目标机器验证。无 GPU 或测得加速不足 15% 时使用 CPU。
- SIRT 是限预算的小体积预览方法；CGLS 没有数值实现，界面与引擎保持禁用。256³ FDK 体素间距是计算采样间距，不能当作实测空间分辨率或 HU。
- 相机文件已落盘但清单尚未提交时，恢复会拒绝未登记文件并要求人工核查，防止静默覆盖；跨 HOME 的物理角度配准也无法仅凭文件哈希证明。

本版仅完成离线验证。投入 CT 实机使用前，需针对端口、射线参数、现场联锁、相机文件与几何体模执行独立验收。
