# Windows 安装与更新维护

当前版本来自 package.json、backend/Cargo.toml 和 backend/src-tauri/tauri.conf.json。只交付 Windows x64 MSI，兼容目标为 Windows 10 / Windows 11。版本账本 tools/release/msi-version-ledger.json 不重映射历史 MSI 版本；UpgradeCode 和签名公钥保持原值。

## 构建与校验

1. npm.cmd run installer:build 构建前端、桌面壳和同一来源的引擎，release/ 只生成最新 MSI；来源哈希、签名和许可证归 .cache/tests/release/<version>/。MSI 内含许可证和 WebView2 引导安装器（缺少运行时时自动联网安装）。构建不安装应用。
2. tools/release/windows-installer.wxs 提供当前用户安装、位置选择、开始菜单/桌面快捷方式复选框，以及 Windows 原生修改、修复、卸载。应用、引擎和许可证安装在同一目录；卸载保留任务、照片和用户数据。
3. inspect-msi.ps1 读取实际 MSI 的 ProductCode、UpgradeCode、版本及架构；使用 WiX dark.exe 在 .cache/ 中提取 MSI 文件，record-bundle-identities.mjs 接受提取的应用路径，核对 MSI marker、桌面二进制和相邻引擎，无须安装。历史安装维护验收只认证原构建。
4. sign-update-release.mjs 仅签最终 MSI 和发行清单，生成内部 MSI 更新容器并执行真实签名验证；不自动上传。签名材料继续使用 LocalAppData/MicroCTReleaseKeys 的 ACL/DPAPI，秘密不进入参数、日志或版本控制。

安装 UI 使用共享 WiX 对话框对应的 Tahoma 8/12/9 字体；不可单独放大字体而保留原控件尺寸。目录选择由 tools/release/windows-folder-picker.cpp 提供 Windows 原生 IFileOpenDialog，随 DPI 缩放图标，选择后更新 MSI 目录目标并刷新路径输入。prepare-installer-ui.mjs 用 MSVC/Windows SDK 构建即时 UI DLL 和绝对路径 WiX fragment；verify-installer-ui.ps1 检查实际 MSI 的全部对话框、字体、控件及原生目录动作。品牌 BMP 位于 assets/。

安装器允许 Windows Installer 请求 UAC 提升以处理受保护的卷回滚文件；ALLUSERS 不设置，并通过条件限制当前用户安装，保留既有 per-user 安装的升级关系。不得使用 InstallScope="perUser"，因为 WiX 会因此设置“不需要提升”标志，导致安装到其他卷时 Repair 的备份清理报 Error 1926 / Error 5。verify-installer-ui.ps1 检查最终 MSI 的 SummaryInformation bit 8 必须清除，同时禁止 ALLUSERS 非空。应用本身仍按普通用户运行。

## 使用

首次打开 MSI：选择目录，勾选所需快捷方式，开始安装。再次打开相同 MSI：Change 修改快捷方式，Repair 修复文件/注册信息，Remove 卸载。也可通过 Windows 应用列表进入修改/卸载。默认路径为 %LOCALAPPDATA%\Programs\Micro-CT Workstation；改变安装位置应先卸载再安装，数据保留。

## 更新流程

Help → Check for Updates 立即查询 GitHub 的最新稳定发行。当前版本相同或更高时显示 You're up to date；有更高版本并通过受签发行身份检查时显示 Download update，点击后才下载；完整校验后显示 Install update。查询失败显示失败和重试，不冒充已是最新版。正常页面不展示渠道、URL、密钥、哈希或离线导入。

引擎拒绝活动事务及持久任务锁，在维护占用下清理设备，提供本次 session/operation 关联的 ready 报告。UNKNOWN 或清理未确认不能授权安装。Tauri 消费匹配报告并等待 owned engine 正常退出；helper 等待原应用退出后运行 MSI，保留已选安装路径。更新不自动放弃任务、连接设备或开束。下载、校验、进程退出、安装完成各自独立记录。

历史验收包与记录在 .cache/tests/release/retained-history/ 本机保留，退出当前交付目录。安装检查不能替代设备硬件验收。

## 实现依据

- [Tauri Windows installer documentation](https://v2.tauri.app/distribute/windows-installer/)
- [Tauri CLI 2.12.1 MSI template](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.1/crates/tauri-bundler/src/bundle/windows/msi/main.wxs)
- [WiX standard installation UI](https://github.com/wixtoolset/wix3/blob/develop/src/ext/UIExtension/wixlib/WixUI_InstallDir.wxs)
- [Windows Common File Dialog](https://learn.microsoft.com/en-us/windows/win32/shell/common-file-dialog)
