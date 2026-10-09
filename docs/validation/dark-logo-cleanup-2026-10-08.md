# 深色 Logo、残留检查与最新安装包

日期：2026-10-08。版本：0.9.7。**本次没有安装应用，没有连接设备或发送设备动作；仅离线验证。**

## 当前结果

- 用户已卸载 0.9.6。原安装目录不存在，Windows 卸载登记、桌面和开始菜单/任务栏快捷方式均未发现该应用入口。
- release/ 仅有 micro-ct-workstation_0.9.7_windows_x86_64.msi，11,821,544 字节。旧 0.9.6 目录、安装包及其他交付文件已删除。构建来源、签名、许可证和身份记录归 .cache/tests/release/0.9.7/；许可证已进入实际 MSI。
- MSI SHA-256：615857b2fd1e79b86a49ef01117f5baed8d5183d2da36f41eb574177ea26347b。
- ProductVersion 0.9.7，ProductCode {77370FCB-35AB-40A5-A4F7-B77D502E9536}；沿用 UpgradeCode {A96DC7F4-7384-4B5E-9AB5-6BE9CB76C8F2}。目录选择、两个快捷方式复选框、修改、修复及卸载契约保留。
- Windows 10/11 x64 仍为兼容目标；本机 Windows 11 的构建、包提取、真实启动、主题展示与普通退出通过，Windows 10 尚无实机运行证据。

## 深色 Logo

保留 assets/micro-ct-logo.png 原始字节。新增 micro-ct-logo-dark.png（4096×4096 RGBA）和 app-logo-dark-256.png。调色规范为：保留外圈与轮廓，中央 T 使用浅蓝/青色提高辨识度，右侧梯形使用暗灰蓝降低亮度，透明背景不变。

imagegen 内置服务返回 HTTP 429；用户明确选择本地精确调色后才执行。原始提示词、可复现调色脚本、像素检查和对比预览在 .cache/tests/release/20261008-dark-logo-cleanup/。全部 alpha 和非目标区像素逐字节保持不变。T 区平均线性亮度从 0.0533 提升至 0.4180；梯形从 0.7103 降至 0.0733。

MenuBar 在正常界面和引擎启动界面均按实际 theme 选择 Logo；favicon 的初始加载和主题变化保持一致。有手动选择时优先采用该选择，否则跟随系统深浅色变化。Windows 安装快捷方式的静态图标仍保留透明原图。

## 残留检查与删除限制

以下三处已核对，但 **BLOCKED：自动审批拒绝删除，返回 blocked by policy**。没有更换工具绕过拒绝，未报告已清理。手动清理脚本位于 .cache/tests/release/20261008-dark-logo-cleanup/cleanup-c-drive.ps1；它再次核对未安装/未运行、精确路径、目录无链接、文件类型和旧引擎 SHA-256 后才执行。该脚本未自动运行。

| 路径（均在 C:/Users/Anbao/AppData/Local/） | 剩余内容 | 状态 |
|---|---|---|
| MicroCTWorkstation/ | 空 runtime/、updates/instances.lock（0 字节） | BLOCKED，仍存在 |
| edu.lzu.microct.workstation/EBWebView/ | 应用 WebView 配置与缓存，387 文件、42,612,659 字节 | BLOCKED，仍存在 |
| Temp/ct-engine.exe | 10,230,784 字节；SHA-256 db9b7dc3c838ae5d13033a9d871286e4bff82fdbd21f734dd8d369ce61173086 | BLOCKED，仍存在 |

此前 runtime/ct-engine-1dc09804191c519a.exe 已不存在。对 Local、Roaming、Temp、桌面、开始菜单及任务栏进行范围明确的名称、注册和目标路径检查；未发现第二处 C 盘 ct-engine/ct-workstation EXE。不是对整个 C 盘的无范围扫描。

MicroCTReleaseKeys 是仍在使用的签名密钥材料，保留且没有读取秘密。Windows 共享 WebView2 运行时、仓库最近访问链接、原始照片、扫描数据和任务成果均未删除。

此前从 release 移出的缓存历史目录 .cache/tests/release/retained-history/ 含 25 个旧 EXE/MSI/ZIP；它们已退出当前交付目录。本次删除这些缓存历史包也被自动审批拒绝，仍在原缓存位置，不能声称全部历史二进制已清空。

## 验证

| 结果 | 检查 |
|---|---|
| PASS | npm.cmd run typecheck；installer:build 内执行完整前端 build 与 Rust release 编译。 |
| PASS | npm.cmd run test:frontend：166 通过；test:desktop：24 通过，最终复跑亦通过。 |
| PASS | cargo test --manifest-path backend/Cargo.toml --workspace：353 通过、7 项硬件测试忽略。 |
| PASS | module-map:check：202 文件、16 模块、39 目录边界；修正生成目录排除仅作用于根 release，tools/release 纳入真实源码归属。 |
| PASS | WiX dark.exe 只提取实际 MSI；包内应用、MSI marker 与编译二进制一致，引擎与 release 来源一致。没有运行 msiexec 安装。 |
| PASS | inspect-msi.ps1：x64、Win10 build 10240 起、目录/快捷方式、repair/modify 与身份检查。 |
| PASS | MSI 与发行清单 Minisign 签名、真实更新包及内部验证容器检查；没有发布到远端。 |
| PASS | 实际包内应用的 9 项主题检查：系统深/浅色切换、手动切换、选择持久化、透明像素、T 相对梯形亮度、英文界面、版本与断开设备状态。 |
| PASS | 无调试器的独立普通启动/退出检查：应用 0、引擎 0，WM_CLOSE 发送成功；所有数据目录隔离在 E 盘缓存。 |
| FAIL（调试器场景） | 主题调试会话中两种关闭自动化均未退出，原生 API 返回 failed to send message to the webview。只终止经过身份核对、全部设备断开的测试应用，随后引擎 EOF 清理退出 0；没有用此结果冒充普通退出通过。独立无调试器控制检查通过。调试器场景原因未确证。 |
| PASS | git diff --check；验证结束无应用或引擎进程。 |
| BLOCKED | 上述三处 C 盘残留和缓存历史包删除；自动审批拒绝。 |
| BLOCKED | Win10 实机验收、硬件验收；本次均不在实测范围内。 |

本机原始证据：.cache/tests/release/20261008-dark-logo-cleanup/。旧验收记录只证明对应历史构建，不代表当前安装状态。本次必要包与资料均可用，残留删除尚未完成。
