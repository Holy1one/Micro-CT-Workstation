# 0.9.52 安装与 task5 桌面检查

用户于 2026-10-15 明确要求“安装更新并手动检查”。已将自动图像处理更新构建为 0.9.52，安装到原路径 `E:\Application\Micro-CT Workstation`。MSI 返回 0，Windows 安装登记和应用 About 均为 0.9.52。原 0.9.51 MSI 及身份记录保留在 `.cache/tests/release/0.9.51/`。

**仅离线验证。** 本次使用真实安装版应用和相邻生产引擎处理已封存的 task5，没有连接采集设备或执行拍摄、运动、开束及清卡。

算法与数据适用边界见 [自动图像处理验证](automatic-cbct-2026-10-15.md)。

## 检查结果

| 结果 | 检查 | 实际结果 |
|---|---|---|
| PASS | `npm.cmd run module-map:check` | 模块边界及模块图一致 |
| PASS | `npm.cmd run typecheck` | 类型检查通过 |
| PASS | `npm.cmd run test:frontend` | 205 项通过 |
| PASS | `npm.cmd run test:desktop` | 27 项通过，包含新版本及历史 MSI 分配检查 |
| PASS | `cargo test --manifest-path backend/Cargo.toml --workspace` | 引擎库 530 项、入口 2 项、桌面壳 56 项通过；12 项显式实验默认忽略 |
| PASS | `npm.cmd run installer:build` | 前端及桌面 release 构建通过；实际 MSI 21 个对话框、200 个控件检查通过；提取后的应用与引擎身份匹配本次构建 |
| PASS | `node tools/release/sign-update-release.mjs` | MSI、清单签名及实际更新验证器检查通过；未发布到远端 |
| PASS | 安装及正常启动 | 原目录升级成功，安装文件 SHA-256 与 MSI 身份一致，单一引擎进程来自安装目录 |
| PASS | task5 输入读取 | 380/380 文件、10,104,581,228 字节校验完成；暗场 10、空场 10、投影 360 |
| PASS | 默认按钮完整重构 | 安装版界面点击 Start reconstruction，以自动波段和内置处理完成，约 334 秒；输出独立 v3 缓存 |
| PASS | 结果一致性 | 科学体、预览、参考缺陷掩膜、固定增益掩膜、绿色投影、质量标记、估计标记、迭代报告共 9 个文件与先前离线实验逐字节一致 |
| PASS | 手动显示检查 | X/Y/Z 滑条均使切片更新，三维拖动改变角度，MIP/Opacity、Reset view 和 Equipment/Result 切换正常；没有页面脚本异常或 console error |
| PASS | 缓存复用 | 再次点击默认重构，约 3.3 秒取得 Verified reconstruction cache loaded，仍使用同一 v3 缓存 |
| PASS | 正常退出和普通重启 | 通过界面关闭，所属引擎退出；重新启动时不启用临时检查端口，原生 Task 菜单选择 task5 后，文件校验、默认按钮、缓存与三向切片/三维预览全流程通过 |
| PASS | 原数据保留 | 扫描 manifest 和旧 v2 体/预览哈希保持不变；原始 NEF 保留，新结果另存 |

首次检查的自动目录输入受本机中文输入法和原生控件自动化行为干扰，取消后先通过安装版真实 Tauri/ct-engine 的 `open_scan_project` 入口精确加载 task5。正常重启后的补查使用原生字符输入绕开输入法，完成了 Task → Open Scan Project → 选择 task5 → 文件校验 → 默认重构 → 结果显示，无需 IPC 检查入口。最终普通会话未打开 19352 临时检查端口，应用停留在 task5 结果页。

## 图像处理结果及限制

- 默认单色自动选择绿色通道。检测报告记录 1643 个参考异常候选、64 个传感器异常位置、1175 个保护位置；R/G/B 的 14/112/7 个位置保留实测固定增益。这些计数不表示检测准确率。
- 自动显示窗为约 `[-0.059761, 0.198117]`，来自有符号体的 P1–P99.5；完整 float32 体保留。界面没有新增独立交互页中的实际最暗/最亮调窗控件。
- 三维 PWLS-Huber 粗网格候选的最大修正超过幅度门槛，自动拒绝；最终采用自动校正后的 FDK。没有把被拒绝的迭代候选描述为已采用的改善。
- 点状和强中央伪影减弱，宽环、边缘及底部条纹仍然明显；本次检查不能确认干枣内部结构，不能替代体模或实物准确性验收。
- 全分辨率迭代、真实其他颜色场、真实 GPU 故障注入及新版本设备联动未验证。

## 来源与证据

安装包：`release/micro-ct-workstation_0.9.52_windows_x86_64.msi`

- MSI SHA-256：`b82f88dc8b905d4bddfe000166e33aba43cd37f60d3e2069dd4208fe9019c259`
- 安装程序 SHA-256：`c26337546035bb7ec2d4226adadc85c7b61e44018ce6f284b7463e48e34976e7`
- 相邻引擎 SHA-256：`40dde601eee1825b1b89d27a81d1cc5672c513392878bad3f7d0b791765e8e43`
- 构建基于 `a859a04caa169bce9f5a384abe636fee04e2e6a4` 上保留的工作区修改，`build-info.json` 记录 dirty 来源树哈希；没有重置已有用户改动。

正式生成结果：`E:\Main\Pictures\Camera Roll\task5\reconstruction\spectral\v3-925973e477be99d8b749`。

体数据 SHA-256：`453edf5ae87308c4e1479267826a17df52cb599579f8f216233ccbce1eacb29e`。

仓库根 `.cache/tests/release/0.9.52/` 保存：

- `workspace-tests.log`、`frontend-tests.log`、`desktop-tests.log`、`typecheck.log`、`module-map.log`。
- `installer-build.log`、`signature-verification.log`、`build-info.json`、`bundle-identities.json`、签名与安装包副本。
- `installation.json` / `installation.log`、`installed-identities.json`、`before-install.json`。
- `task5-project-snapshot.json`、`task5-complete-snapshot.json`、`task5-cache-reuse-snapshot.json`、`installed-result-verification.json`。
- `manual-ui-checks.json`，以及默认结果、切片/旋转检查截图。
- `normal-shutdown.json`、`normal-session.json`、`native-picker-check.json`、`normal-project-ready.json`、`normal-result-verification.json`。
- `normal-task5-result.png`：普通启动且检查端口关闭后的实际结果界面。
