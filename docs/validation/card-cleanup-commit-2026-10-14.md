# 清卡记录原子提交失败 — 0.9.51 修复与实机恢复

## 现场数据与故障

用户报告图像清理阶段 `IMPORT_CLEANUP_FAILED: cleanup journal commit failed: 拒绝访问。 (os error 5)`。本次只读检查的任务是 `E:\Main\Pictures\Camera Roll\task5`，不是此前已删除的同名任务。

- schema-4 清单已封存：360 张投影、10 张前暗场、10 张前空场，共 380 个 NEF；`dataStatus=verified`、`completed=true`。
- 离线检查时独立 `card-cleanup.json` 为 `pending`，320 条 `deleted`、60 条 `pending`。剩余回执对应 `DSC_6506.NEF` 至 `DSC_6565.NEF`；后续授权实机恢复见下节。
- 逐一读取全部本地 NEF，380/380 长度及 SHA-256 与封存身份一致，共 10,104,581,228 字节。报告位于 `.cache/tests/ct-engine/20261014-card-cleanup-commit/task5-audit.json`。
- 日志文件没有只读属性，现有 ACL 允许当前用户修改。连续 320 次删除确认已落盘，现场现象符合短暂文件占用；导致现场占用的具体进程没有捕获，不能断言是某个杀毒或索引程序。
- 原子写入器原先仅调用一次 `fs::rename`。离线测试用不允许删除共享的 Windows 文件句柄短暂占用目标，旧代码确实返回同样的错误 5；这是可复现的软件缺口。

## 修复与恢复边界

`backend/ct-engine/src/task_registry.rs` 的共用持久写入器保留唯一临时文件、完整写入、sync 和原子替换。在 Windows 上，替换遇到错误 5/32/33 时每 20 ms 重试同一替换，最多 1 秒，可取消调用每轮检查取消。其他错误立即返回，持续占用或永久权限错误到限返回失败。错误补充目录创建、临时创建、写入/sync 或最终提交的路径，方便后续定位。

旧记录不会先删除或截断，也不修改文件权限。清卡继续要求先持久化删除意图、再精确删除、鲜读确认，最后清卡及任务登记成功才完成流程。失败时保留封存 NEF、已有删除进度和待导入登记。既有 Card import 入口重新校验本地文件与相机身份、重枚举并只处理剩余对象；已验证文件不重新下载或拍摄。

## 离线验证

证据目录 `.cache/tests/ct-engine/20261014-card-cleanup-commit/`。

| 检查 | 结果与证据 |
|---|---|
| 新回归用例在旧代码运行 | 预期 FAIL：同样的 `拒绝访问 (os error 5)`；`transient-before.log` |
| `cargo test --offline --manifest-path backend/Cargo.toml -p ct-engine atomic_store_ -- --test-threads=1` | PASS，3 项：短暂占用、持续占用保留旧记录、取消；`atomic-after.log` |
| `cargo test --offline --manifest-path backend/Cargo.toml -p ct-engine devices::camera::card_import -- --test-threads=1` | PASS，22 项，1 项实机测试忽略；`card-import.log` |
| 360 投影/20 参考图清卡故障注入 | PASS，在第 320 次删除时短暂占用日志后继续，全部 380 条 cleaned；零补拍，380 次下载及 380 次精确删除 |
| 本地原始图像只读校验 | PASS，380/380 长度及 SHA-256 一致；`task5-audit.json` |
| `npm.cmd run module-map:check` | PASS；`module-map.log` |
| `npm.cmd test` | PASS，typecheck、build、205 项前端及 27 项桌面测试；`frontend-desktop.log` |
| `cargo test --offline --manifest-path backend/Cargo.toml --workspace -- --test-threads=4` | PASS，514 + 2 + 56 = 572 项，10 项忽略；`workspace-verified.log`、`workspace-verified.exit.txt` |
| MSI 构建、包身份及正常启动/退出 | 以本证据目录的 `installer.log`、`desktop-smoke.json` 和 `.cache/tests/release/0.9.51/` 为结果 |

首轮串行完整回归有 513 项通过、1 项失败、9 项忽略：既有 Live View 管道测试在读取头部时收到 Windows 错误 233（对端已关闭），不是清卡持久写入错误。单独复核该测试已 PASS，日志 `pipe-failure-recheck.log`；最终完整回归结果单独保存，不覆盖首轮失败记录。

随后并行回归定位到导入测试的临时目录按“秒级时间 + topic + PID”命名，两个同名场景在同一秒复用目录，导致缺失对象测试读到另一个测试的模拟文件。为测试目录名增加纳秒标识，避免共享；只修改测试 fixture，不改变生产存储路径。该次失败保留在 `workspace-final.log`。

最终完整回归全部通过，共 804 项自动测试通过，10 项忽略不计入通过数。上述自动测试仅使用离线脚本传输和隔离测试目录；后续真实清卡单独记录如下。

## 授权安装与 task5 实机恢复

用户明确授权安装并清卡，确认射线已物理关闭，要求保留其他卡内图像；随后确认 D7100 已开机并连接 USB。执行范围是相机连接、恢复既有 task5 清卡和只读核验，没有拍摄、运动、射线源连接或开束。

证据目录 `.cache/tests/ct-engine/20261014-task5-cleanup-hardware/`。安装 0.9.51 的 MSI 返回 0，安装路径 `E:\Application\Micro-CT Workstation`。安装后应用及引擎哈希与验过的安装包一致：

- `ct-workstation.exe`：`1a3bda6b2e8f4f8f2d1d3208a58577aa348e00d931305c68750e9d5ff112ebde`。
- `ct-engine.exe`：`4e32924d518bf3cd888edab408c793507fb26bc0c5dd4f5c7303ad2e7ac1b696`。

生产连接鲜读身份与 task5 冻结身份逐字段一致：Nikon Corporation / D7100，USB VID `04B0`、PID `0430`、机身号 `8322769`，PTP serial `83227690000000000000000000000000`，backend `windows_wpd_nikon_d7100`。外置硬盘 T9 同时存在但未选择。

清理前通过 Windows Shell MTP 命名空间只读记录两卡全部文件，释放 COM 后才连接生产相机会话。通过已安装应用的生产 IPC 调用 `camera_connect(deviceId)`、`start_card_import(taskId=task5, savePath=E:\Main\Pictures\Camera Roll)`，使用既有校验及精确删除链；完成后调用 `camera_disconnect` 并确认关闭。没有直接修改 task5 的持久状态。

| 检查 | 结果与证据 |
|---|---|
| 安装及二进制来源 | PASS；`installation.json`、`installation.log`、`installed-identities.json` |
| 清理前完整卡清单 | PASS，164 个 NEF；目标 60 个名称唯一且大小匹配，其他 104 个加入保护清单；`card-inventory-before.json`、`card-inventory-before-checks.json` |
| 生产相机身份与冻结记录 | PASS，全部身份字段一致；`camera-connected-snapshot.json`、`authorized-cleanup-plan.json` |
| 生产恢复清理 | PASS，60 条精确删除读回确认，仅 `DSC_6506.NEF` 至 `DSC_6565.NEF`；`exact-cleanup-confirmations.json`、`card-import-completed-snapshot.json` |
| 持久清卡状态 | PASS，`cleaned`、380 条 `deleted`、零 pending/deleting；`card-cleanup-after.json` |
| 正式任务登记 | PASS，task5 由 `acquired_pending_import` 变为 `completed`，待导入任务数为 0，task3/task4 记录不变；`registry-before.json`、`registry-after.json` |
| 其他卡内图像 | PASS，新的原生 PTP 会话实际枚举 104 个 NEF；全部与保护清单的名称、存储卡、长度、对象 handle 和拍摄时间匹配，60 个目标全部不存在；`card-inventory-after-native.json`、`hardware-verification.json` |
| 本地原始图像 | PASS，380/380 再次通过长度/SHA-256 核验，共 10,104,581,228 字节；封存 manifest SHA-256 不变；`audit-task5-after.ps1`、`task5-audit.json`、`hardware-verification.json` |
| 相机会话收尾 | PASS，生产断开返回 completed/cleanupConfirmed，独立只读核验会话也正常关闭；`camera-disconnected-snapshot.json`、`native-inventory.log` |
| 恢复普通安装版入口 | PASS，应用和引擎正常退出后重新启动生产模式，唯一引擎 owner，临时浏览器检查端口关闭；`normal-session.json` |

生产操作 `a0ca8a0e672dddfd593b96a8c8664853-op-4` 从 2026-10-14 15:26:52.363 UTC 至 15:29:18.683 UTC 完成；未再次出现日志提交错误，全部本地文件已有 verified 身份，未补拍或重新下载。

Shell 清理后仍返回缓存的 164 个文件，不能用作删除完成证据，原始缓存结果保留在 `card-inventory-after-shell-cached.json`。因此在生产相机会话确认关闭后，以仅允许 `GetDeviceInfo/GetStorageIDs/GetObjectHandles/GetObjectInfo` 的只读核验程序鲜读实际设备；复用维护中的 WPD transport 与解析器，零删除/写入/图像载荷操作。比较拍摄时间时把 Shell 的 UTC 时间转换为相机记录的 UTC+8 字符串。实际 PTP 返回 104 个文件，匹配检查见 `hardware-verification.json`。

## 交付边界

0.9.51 已获授权安装，MSI 路径 `.cache/tests/release/0.9.51/micro-ct-workstation_0.9.51_windows_x86_64.msi`，SHA-256 `0eebfb24e92c013261580aaf0f53d4def52b4ab009603483b4069de9ca4204d2`。0.9.50 历史包及其身份仍保留在 `.cache/tests/release/0.9.50/`；安装前的 0.9.49 身份见离线证据目录的 `installed-before.json`。

**PASS：离线回归、修复版安装及本次真实 task5 剩余清卡恢复。** 其他 104 个卡内 NEF 与全部 380 个本地原始 NEF 保留，task5 可按既有流程加载后重构。本次证据只覆盖安装与清卡恢复，不代表完整扫描、辐射联锁或图像质量验收；相机 `hardwareCertified` 仍为 false。射线源未连接，软件保持 UNKNOWN/未监测，操作员的物理关闭确认不改写设备读回状态。
