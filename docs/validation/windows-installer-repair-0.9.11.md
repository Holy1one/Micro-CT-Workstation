# 0.9.11 安装器 Repair 权限修复

## 故障与修复

用户已安装 0.9.10 到 E:\Application\Micro-CT Workstation，在重跑同一 MSI 的 Repair 时，清理 E:\Config.Msi\*.rbf 连续报 Error 1926 / Win32 Error 5。旧包实际 SummaryInformation WordCount 为 10，其中 bit 8 声明不需要提升权限。管理员只读诊断确认回滚目录保留 SYSTEM / Administrators FullControl；不放宽卷根目录或回滚目录 ACL。

0.9.11 的 Package 明确允许 elevated 权限，去掉会强制设置 limited 的 InstallScope="perUser"。ALLUSERS 保持未设置，通过条件限制当前用户安装，保留当前用户升级关系、UpgradeCode、安装目录和数据路径。UAC 由 Windows Installer 请求；应用自身继续普通用户运行。不禁用回滚、不忽略安装错误、不重新生成签名公钥。

## 验证

2026-10-08 在本机 Windows 11 x64（build 26200，250% 缩放）完成安装器实际验收。用户明确授权在原目录升级及 Repair；未连接或操作真实设备，设备能力仅离线验证。验收脚本和日志归 `.cache/tests/release/20261008-installer-repair/`；最终包、身份与签名归 `.cache/tests/release/0.9.11/`。

- PASS：新门禁针对旧的实际 0.9.10 MSI 拒绝 bit 8，复现原配置缺陷。
- PASS：模块图检查；桌面测试 27 passed。
- PASS：`node tools/release/build-update-release.mjs` 完成前端、Rust 和 MSI 构建，实际 MSI 全部 21 个对话框、200 个控件通过 UI 门禁；当前用户安装与允许提升权限检查通过。
- PASS：最终 MSI 与发行清单签名验证、MSI 提取文件身份检查。提取后的应用及引擎正常启动、退出（均为 exit 0）；250% 缩放下窗口大小图标分别为 80 / 40 像素。
- PASS：`verify-installed-repair.ps1` 使用最终 MSI，将 0.9.10 升级为 0.9.11，随后两次从 Maintenance 界面选择 Repair、确认 Repair、点击 Finish。三个安装客户端均 exit 0，无 Error 1926、Return value 3 或循环权限弹窗。
- PASS：三个 MSI 日志均记录用户同意 UAC 后 `MsiRunningElevated = 1`；升级记录 `Installation completed successfully`，两次 Repair 记录 `Configuration completed successfully`。
- PASS：每次操作后核对安装目录仍为 `E:\Application\Micro-CT Workstation`，应用及引擎 SHA-256 与最终 MSI 完全一致；旧产品注册已移除。开始菜单及桌面快捷方式保持原有未选择状态。
- PASS：`check-installed-app.ps1` 对 Repair 后的实际安装文件进行启动检查，使用隔离的引擎和 WebView 数据目录。界面显示 Control service Ready，无启动失败提示；正常关闭后应用 PID 61856、所属引擎 PID 62124 均 exit 0。
- PASS：操作完成后无 Micro-CT 安装客户端、应用或引擎进程遗留。`release/` 仅保留最终 0.9.11 MSI。

数据保留检查的持久文件基线为 0（`MicroCTWorkstation` 中排除日志和更新缓存后无文件），不能作为已有扫描数据的完整性验收。安装器未处理扫描图片目录；本次未用真实扫描数据样本验证。Windows 10 尚无实际安装验收；包保留 Windows 10 / 11 x64 版本约束。

## 交付身份

- MSI：`release/micro-ct-workstation_0.9.11_windows_x86_64.msi`，17,023,415 bytes。
- MSI SHA-256：`db8460f33aa4552880b2b16e9bd6da8b73011e8705dbdccc9dcba7f4f40102ff`。
- 应用 SHA-256：`32a20349c3f513a6758c0be482bb604ea0369f2b38f2152731ffbea7c9fe8376`。
- 引擎 SHA-256：`d46f42fa57dda927af0c7366634f20a814d6720cf75f221c685ae0d6badf8b95`。
- ProductCode：`{A398881F-EAE8-4808-B3A0-552D7465B1AA}`；UpgradeCode：`{A96DC7F4-7384-4B5E-9AB5-6BE9CB76C8F2}`。
- 构建来源：commit `1e98cf3b10d88d839b7ea367fb45a3a31977451b`（含本机未提交改动），sourceTreeSha256 `7e3f76f3281f7ae55f3685e1d69af138cef905faf72b7a58d0cfb8cab2f2d8f3`。

实际升级/Repair 报告为 `installed-repair-report.json`，日志为 `upgrade.log`、`repair-1.log`、`repair-2.log`；安装后启动报告为 `installed-startup/report.json`。最终包已固定身份并签名，验收后未重新构建或替换；未发布到 GitHub。

## 依据

- [WiX Package：InstallScope perUser 强制 limited](https://docs.firegiant.com/wix3/xsd/wix/package/)
- [MSI Word Count Summary 的 bit 8](https://learn.microsoft.com/en-us/windows/win32/msi/word-count-summary)
- [Windows Installer 安装上下文](https://learn.microsoft.com/en-us/windows/win32/msi/installation-context)
