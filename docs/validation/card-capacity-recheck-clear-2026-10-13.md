# D7100 容量复核、双卡清空与 0.9.44

> 历史范围：该轮容量与独立授权清理的历史证据；不构成当前全卡删除许可。当前只清理本次已验证回执对象，见 [ADR 0004](../decisions/0004-camera-sd-card-acquisition.md)。

## 判断纠正

用户要求重新判断大容量 SD 卡的可拍数量，并确认全部已备份到电脑、授权直接删除当前 SD 卡内容。先前“解锁后最多 374 张 / 354 投影”的说法将机身估算当成硬上限，现已纠正。

PTP 将 `FreeSpaceInBytes` 定义为剩余字节，`FreeSpaceInImages` 是按设备当前拍摄设置返回的剩余图像数，见 [PIMA 15740 的 StorageInfo 定义，第 22–23 页](https://people.ece.cornell.edu/land/courses/ece4760/FinalProjects/f2012/jmv87/site/files/pima15740-2000.pdf)。[Nikon D7100 手册的容量章节，第 348 页](https://cdn-10.nikon-cdn.com/pdf/manuals/dslr/D7100_EN.pdf)使用近似容量数据，并说明文件大小随场景变化。本项目按用户指定的 32 MiB/张规划，不再让正数机身估计额外收紧字节预算；这是规划选择，不能保证未来每张 NEF 都小于 32 MiB，或保证最终真实可拍张数。

解锁后、清理前两卡共有 20,044,349,440 字节（18.67 GiB），分别按 32 MiB 向下取整可预算 276 + 321 = 597 张。机身估算是 173 + 201 = 374 张。360 投影 + 10 暗场 + 10 空场 = 380 张，预算 12,750,684,160 字节（11.875 GiB）；在已协商双卡 Overflow 的前提下，字节预算足够。槽 1 尚为 0 张时，它的空余字节不能贡献任务容量。

本次清理前真实文件清单有 415 个 NEF 和 46 个 JPEG。NEF 大小为 18,803,066–33,007,374 字节，最大 31.48 MiB，平均 26,627,916 字节。这只是卡内既有文件的元数据样本，未证明它们都使用相同冻结拍摄参数，也未证明后续文件大小上限。

## 真实清理与授权

唯一目标是 Nikon Corporation D7100，USB VID 04B0 / PID 0430，序列号 8322769；协议长序列号 `83227690000000000000000000000000`。相机 owner 应用均已关闭。两卡 StorageID 为 65537、131073，均为 removable RAM，USB AccessCapability=2（允许删除）。没有打开转台或射线串口。

1. 元数据枚举确认 461 个照片文件和 4 个目录，记录完整对象清单、保护状态及容量。
2. 按用户“全部已备份、直接清理”的授权，对每个文件重新核对元数据并单独执行已广告的 `DeleteObject`；458 个成功，删除对象大小合计 11,315,514,659 字节。
3. 机身以 `0x200D ObjectWriteProtected` 拒绝删除 `DSC_5277.JPG`、`DSC_5277.NEF`、`DSC_5662.NEF`。设备未广告 `SetObjectProtection` 或 `SetObjectPropValue`，未试发这些操作。
4. 展示剩余文件及限制后，用户明确回复“允许格式化两张 SD 卡”。核对同一相机、StorageID、最大容量与卡槽标签，对两卡分别执行已广告的标准 `FormatStore [StorageID]`，使用相机默认文件系统格式。
5. 两次格式化均返回 `0x2001 OK`。重新枚举两卡均为 0 个对象、0 个文件；再次读回容量，相机会话正常关闭。

仅修改相机 SD 卡；电脑上的备份、原始扫描目录和任务文件保留。没有拍摄、修改摄影参数、运动或开束。

## 清理后容量

| 卡槽 | 剩余字节 | 可用 GiB | 机身估算张数 | 按 32 MiB 的规划张数 |
|---|---:|---:|---:|---:|
| 1 / 65537 | 15,514,664,960 | 14.45 | 290 | 462 |
| 2 / 131073 | 15,923,085,312 | 14.83 | 297 | 474 |
| 合计 | 31,437,750,272 | 29.28 | 587 | 936 |

相较清理前增加 11,393,400,832 字节（10.61 GiB）。空间增量包含文件系统分配开销，与对象文件大小之和不同。两卡各保留 65,536 字节格式化开销，未把最大容量直接当可用容量。单独槽 1 的 462 张预算已可覆盖本次 380 张；合计 936 张需已协商 Overflow，始终是预算而非硬件保证。

`D148` 原始值仍为 0，未写入卡槽方式；`D1F1` 为 587。`D1B2` 原始值 0 保持语义未知，不据此宣称全设备正常或射线安全。

## 软件修改

- `plan_camera_cards` 使用可用剩余字节和 32 MiB 预算，逐卡向下取整。原始正数 `free_images` 不改写，只作为估算诊断保留。
- 0 张卡既不贡献字节，也不贡献预算张数；未知张数/容量仍拒绝。未知型号、ROM、未协商 Overflow 和实际 StoreFull / StoreRemoved / 拍摄错误继续阻断。
- `CardPlan.writable_images` 表示本次预算张数，扫描日志改为 `budgeted frames`，不再写成原生剩余张数。通用 USB 写入规划仍拒绝 ReadOnly 与 ReadOnlyWithDeletion。
- 回归覆盖清理前真实 173/201 与容量字节、清理后 290/297 与容量字节、精确字节边界、0/未知和未协商 Overflow，以及 owner 只读规划不发拍摄/图像传输。
- 保留此前错误信息布局修复，未重新引入已撤回的容量预检/启动登记流程改造。

## 证据与验证

硬件原始证据目录：`.cache/tests/ct-engine/20261013-card-clear/`。

- `inventory-initial.json` / `inventory.json`：删除前文件与能力清单。
- `deletions.jsonl` / `clear-result.json`：458 次成功删除及 3 次保护拒绝。
- `format-operations.jsonl` / `format-result.json`：用户明确授权后的两次格式化与 0 对象读回。
- `after-format.json` / `capacity-summary.json`：清理后容量和文件大小统计。
- `card-maintenance.rs` / `build.ps1`：本轮隔离维护程序，复用生产 WPD/PTP 传输与解析；未新增产品内全卡清空入口。

## 最终门禁与安装

| 结果 | 检查 |
|---|---|
| PASS | 真实双卡清空：两次 FormatStore 成功，重新枚举均为 0 个对象，剩余字节回读及 WPD 会话关闭确认 |
| PASS | `cargo test --offline --manifest-path backend/Cargo.toml --workspace`：492 个引擎库测试、2 个引擎主程序测试、56 个桌面 Rust 测试，共 550 passed、10 ignored、0 failed |
| PASS | `npm.cmd run typecheck`；`npm.cmd run test:sites`：前端构建、202 个前端测试、27 个桌面合同测试；合计软件测试 779 passed |
| PASS | `npm.cmd run module-map:generate`、`npm.cmd run module-map:check`、`git diff --check` |
| PASS | `npm.cmd run installer:build`；MSI 实际解包后的应用及相邻引擎与构建产物一致；`node tools/release/sign-update-release.mjs` 签名及更新容器验证 |
| PASS | Windows Installer 静默升级至 `E:\Application\Micro-CT Workstation`，退出码 0；安装后的两个二进制哈希与最终 MSI 一致 |
| PASS | 使用隔离引擎与 WebView 数据启动已安装程序，Control service Ready，无设备命令；12,000 字符容量错误下布局偏移 0 px；正常关闭退出码 0 |
| PASS | 原 active-task.json、task5/manifest.json、task5/scan.log 的 SHA-256 与前轮及安装前一致；0.9.43 安装包和证据保留于原版本缓存 |
| BLOCKED | 本轮未执行真实完整 360 投影扫描；容量预算修正仅离线验证，不标记硬件采集认证完成 |

初轮完整测试发现旧“正数 5 张即拒绝”扫描 fixture 与新预算不一致，以及桌面测试仍固定 0.9.43。扫描 fixture 改为有字节但 0 张的真实拒绝情形，版本合同更新为 0.9.44 并保留 0.9.43 分配不可变断言；完整门禁重跑通过。初轮失败日志与最终日志均保留，未跳过失败项。

正常退出时引擎报告 `camera cleanup=true`、`STOP confirmed=true`、`unresolved cleanup=false`。本次启动未连接射线，`OFF required=false`、`OFF confirmed=None` 保留原义，不作为真实射线 OFF 验收。

最终安装包：`release/micro-ct-workstation_0.9.44_windows_x86_64.msi`。

- MSI SHA-256：`c3610700e3540b5bf6ac1c5c6b26513a36013ee07fad4937d55fb85b07a25c05`。
- 已安装应用 SHA-256：`9ea80c4b917cd851f1d818a4b82e8da8f164123b055436aaf765260431baa339`。
- 已安装引擎 SHA-256：`d9b78aaeebcde9e76caa8f3b92178c37ad7768ec3fe2b2d01da73440a18e301c`。

补充交付证据：`delivery-summary.json`、`install.json`、`installed-ui.json`、`normal-exit.json`、`protected-files-final.json`；发布身份与签名保存在 `.cache/tests/release/0.9.44/`。
