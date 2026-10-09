# Windows 安装与更新验收

日期：2026-10-08。版本：0.9.6。本次完成 Windows 安装交付、引擎安装位置迁移、更新界面简化和历史交付材料整理；仅离线验证，没有连接设备或发送硬件动作。

本记录为 0.9.6 历史验收。用户已卸载该版本，旧 release/0.9.6/ 已清理；当前 0.9.7 安装包、深色 Logo 与未清理残留见 [当前记录](dark-logo-cleanup-2026-10-08.md)。

## 当前交付

- 安装器：release/0.9.6/micro-ct-workstation_0.9.6_windows_x86_64.msi，11,792,872 字节（11.25 MiB）。只交付 Windows x64 MSI。
- 已安装到：C:/Users/Anbao/AppData/Local/Programs/Micro-CT Workstation。开始菜单与桌面快捷方式均指向该目录内的 ct-workstation.exe。
- 安装向导提供目录选择和两个快捷方式复选框；再次打开安装器提供 Change、Repair、Remove。Windows 应用列表中已登记版本与维护入口。
- WebView2 已存在时复用；缺少时由内置微软引导安装器联网安装。本机运行时版本 154.0.4258.62，未重复安装运行时。
- 引擎与应用一起安装；启动前校验编译时记录的 SHA-256。缺失或被修改时提示修复，不向用户运行缓存释放可执行文件。

| 身份 | 本次实测 |
|---|---|
| MSI SHA-256 | 4e059591940cf13575d4555d98f48083ad1775e06c154e9d507485fcd1efb596 |
| 安装后的应用 SHA-256 | 10655b5ad254cd820b3be63ec1f3fb8b2c383f3b2e02cb274d32fae929759419 |
| 安装后的引擎 SHA-256 | db9b7dc3c838ae5d13033a9d871286e4bff82fdbd21f734dd8d369ce61173086 |
| ProductCode | {497BF573-90CC-4FF9-93F7-28740563FC4D} |
| UpgradeCode | {A96DC7F4-7384-4B5E-9AB5-6BE9CB76C8F2}，沿用原值 |
| MSI ProductVersion | 0.9.6，已登记不可变版本账本 |
| 安装后的 bundle marker | __TAURI_BUNDLE_TYPE_VAR_MSI |

源提交 1e98cf3b10d88d839b7ea367fb45a3a31977451b，工作树含有效未提交改动。构建时来源记录位于 release/0.9.6/build-info.json；安装身份、签名、清单和许可证位于相同目录。更新使用现有固定公钥完成数据、全局签名、版本和文件哈希验证；不将该签名等同于 Windows Authenticode 发布者证书。本次没有向远端发布。

## 简化更新流程

Help → Check for Updates 打开后立即查询 GitHub 最新稳定发行。有更高版本并通过受签发行身份检查时显示 Download update，点击后才下载；完整校验后显示 Install update。设备维护清理、任务锁、引擎退出和 helper 身份检查继续在后端执行。

查询失败保留失败和重试；只有取得有效最新发行版本，且本机版本不低于它，才显示 You're up to date。本次真实查询取得 v0.8.2，本机 0.9.6，因此显示已是最新版。普通页面已移除渠道、地址、签名细节、离线导入和多层安装确认；Help 菜单只保留一个更新入口。页面、菜单、错误呈现与无障碍文案保持英文。

## 验证

| 结果 | 命令或检查 | 证据 |
|---|---|---|
| PASS | npm.cmd run typecheck、npm.cmd run build | 类型及生产前端构建通过；删除未使用接口后生成的 index 哈希未改变。 |
| PASS | npm.cmd run test:frontend | 166 项通过；更新入口与菜单数量契约已同步。 |
| PASS | npm.cmd run test:desktop | 24 项通过；后续更新反馈/菜单定向回归 8 项通过。 |
| PASS | cargo test --manifest-path backend/Cargo.toml --workspace | 353 项通过，7 项硬件测试保持忽略；覆盖缺失/篡改引擎、下载操作关联、无有效元数据不得报告最新版、维护锁和更新签名。 |
| PASS | npm.cmd run installer:build | 最终 MSI 构建成功；WiX 用户目录组件使用 HKCU key path 与明确清理规则。 |
| PASS | inspect-msi.ps1、record-bundle-identities.mjs | MSI 版本、架构、OS 门槛、目录/快捷方式控制、修复/修改入口及实际安装的应用/引擎核对通过。 |
| PASS | 实际安装与维护 | 首次省略快捷方式、修改添加快捷方式、移走引擎后修复并核对原哈希、卸载、最终重新安装：5 次 MSI 操作退出码均为 0。 |
| PASS | 真实安装器界面 | 维护欢迎、Change/Repair/Remove、快捷方式复选框及修复确认页读取通过；在执行前取消，退出码 1602。 |
| PASS | 已安装应用启动与正常退出 | 9 项真实 Tauri/WebView 检查通过；生产引擎 idle、设备均未连接；引擎从安装目录启动。应用/引擎退出码均为 0。 |
| PASS | sign-update-release.mjs | 最终 MSI、发行清单签名及内部 MSI 更新容器通过真实验证器校验。 |
| PASS | npm.cmd run module-map:check | 模块归属、目录约束及生成图谱通过。 |
| PASS | 本机 Windows 11 x64 | 系统 build 26200；完成安装、维护、启动及更新查询。注册表遗留 ProductName 标签不作为系统版本判断。 |
| BLOCKED | Windows 10 实际运行验收 | 本机没有 Windows 10 环境；已设置 Windows 10/11 x64 安装门槛并使用 Tauri/WebView2 标准路径，尚未在 Windows 10 实机运行。 |
| BLOCKED | 硬件与更高版本在线更新完整验收 | 本次没有硬件动作；公开发行版本低于本机版本，未伪造线上更新成功或硬件认证。 |

本机原始证据位于 .cache/tests/release/20261008-windows-installer/，包括安装日志、maintenance/registry/identity 报告、签名验证、测试日志和桌面截图。正式验收以最终成功记录为准；初期完整运行时下载与 WiX 组件校验失败已经修正。

## 清理与保留

- 已删除 C:/Users/Anbao/AppData/Local/MicroCTWorkstation/runtime/ct-engine-1dc09804191c519a.exe，共 10,224,128 字节。安装版运行后未重新生成旧引擎文件；任务、照片、扫描数据和签名密钥保留。
- 旧构建入口退出 package.json 与工具目录；当前 release/ 只保留 README 和 0.9.6 安装发行材料。
- 七组历史交付包与原始验收报告移入 .cache/tests/release/retained-history/，来源和唯一有效证据完整保留。根目录、模块图谱、发行指南及接口文档已同步安装交付方式。
- WPD 的 Portable device 描述属于 Windows 设备技术名称，保留其设备身份语义，与软件交付方式无关。
