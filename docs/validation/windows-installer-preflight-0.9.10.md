# 0.9.10 安装器显示与预扫描提示

本次仅离线验证；没有安装、修复、卸载应用，也没有连接或操作真实设备。兼容目标为 Windows 10 / Windows 11 x64；实际桌面验证为 Windows 11 build 26200、250% 缩放（DPI 240），未在 Windows 10 实机上运行。

## 改动

- 安装器的共享 WiX 对话框恢复其对应的 Tahoma 8/12/9 字体，解决字号与控件尺寸不匹配造成的裁切。全部 21 个对话框、200 个控件经实际 MSI 表检查；全部对话框已渲染并测量 Static 文字，没有溢出控件高度。
- 用 Windows 原生 IFileOpenDialog 替换固定 16px 文件夹图标的旧 BrowseDlg。目录选择动作只更新 MSI 会话目录目标与属性；使用属性事件刷新路径字段，选择结果进入下一步，取消保留原路径。原生选择线程优先使用英文资源；系统文件夹和用户文件名按 Windows 显示。
- 安装器标题区和侧边图使用项目 Logo 的高 DPI 品牌 BMP。
- engine_snapshot / engine_command 在首次启动时异步等待 bootstrap，最长 15 秒；成功前不误报“still starting”通信故障，失败或超时仍报告错误。已送达命令不重试。
- Scan Preparation 展示任务草稿、设备、相机配置、存储、射线 OFF/联锁/温度/设定及 USB Auto Shut Down 的只读缺项。本次操作阶段、同次日志、失败原因和修正建议在窗口内呈现；未通过保留窗口。Stop、代次与会话保护、清理未知状态及持久任务锁保留。
- 全局真实故障卡片修正事实区布局；Image storage root 的按钮贴近输入框右端。

## 验证证据

证据目录：`.cache/tests/release/20261008-installer-dpi/`。

| 检查 | 结果 |
|---|---|
| typecheck、前端构建、模块图 | PASS |
| Rust workspace | PASS：354 passed，7 ignored |
| 前端及桌面测试 | PASS：前端 170 passed，桌面 27 passed |
| 实际 MSI 字体、全部对话框/控件、原生动作与路径刷新事件 | PASS |
| 21 个安装页面在 DPI 240 下渲染及文字高度测量 | PASS；gallery-report.json |
| 欢迎→目录→原生选择/取消/改变路径→快捷方式→确认→取消 | PASS；flow-report.json、preview-flow.log |
| 浏览器首次启动、目录按钮位置、预扫描缺项与失败窗口 | PASS；frontend-ui-report.json 与 PNG |
| 最终 MSI 提取应用的真实桌面启动 | PASS；控制服务 Ready，没有启动通信误报 |
| DPI 240 下原生窗口图标 | PASS；small/fallback 40px，large 80px，符合当前 DPI 系统尺寸 |
| 最终应用与所属引擎的正常退出 | PASS；两者退出码均为 0 |

安装器预览使用隔离复制品：删除全部 InstallExecuteSequence、删除 UI 的 ExecuteAction 和应用启动动作、替换 ProductCode。仅浏览页面并最终取消（1602），没有点击 Install。逐页展示也使用该隔离复制品；没有执行安装事务。

最终 MSI、构建身份与启动/退出结果归 `.cache/tests/release/0.9.10/`，公开交付目录 release/ 只保留最终 0.9.10 MSI。版本、构建来源和 SHA-256 以 build-info.json 为准。包不是硬件认证。

## 最终交付身份

- 文件：`release/micro-ct-workstation_0.9.10_windows_x86_64.msi`
- 大小：17,023,347 bytes。
- MSI SHA-256：`e72b1c5dcdb9c11815092b9d7059371975a4979fb7034da51746c7882f4131d6`。
- 应用 SHA-256：`60b562f6d5a0340ab763988f98084a0ffe1338ad9226941ba2d2ef3f55b2fb76`。
- 引擎 SHA-256：`cf9e69daf859fb0884d63a1957118c593140fa47b902f3cadb108ac598421a52`。
- 最终启动证据：`startup-ui-report.json`、`runtime-report.json`；只检查该 MSI 提取应用的窗口与所属引擎，不安装应用、不连接设备。
- 更新包及清单的签名验证 PASS；没有发布更新。

## 后续用户安装证据：FAIL

2026-10-08，用户手动运行同一份 0.9.10 MSI 后报告连续弹窗。只读检查确认：安装器客户端 PID 55508；Windows Installer 已登记最终 ProductCode `{0A575821-330E-4D33-A286-3263E9ECE49E}`、版本 0.9.10、状态 5（已安装），位置为 `E:\Application\Micro-CT Workstation\`。

当前事务在处理 `E:\Config.Msi\*.rbf` 时连续记录 Error 1926 / Win32 Error 5，Windows 事件 11926 与 MSI 日志一致。管理员级只读检查经本次用户授权执行：回滚目录受保护，SYSTEM 和 Administrators 均为 FullControl，未发现缺失这两项权限的证据；当前未提升用户无法读取该目录 ACL。证据指向清理受保护回滚文件时的权限冲突，具体失败调用尚未定位。

当前页面显示 Removing backup files，Cancel 已隐藏。经授权向已核对的安装窗口发送一次正常关闭请求后，弹窗已收起，客户端进程仍未退出。随后取得单独授权，核对客户端路径及创建时间后，仅结束 PID 55508：客户端退出、所属工作进程退出，Windows Installer 事件 1042 确认该事务结束。没有结束系统安装服务。

结束后再次只读核对：产品状态仍为 5，安装目录的应用和引擎哈希仍与本次最终 MSI 完全一致。事务结束不等于本次维护成功；未运行已安装应用验证。本次未修改权限或再次安装。先前 UI 预览与离线检查 PASS 不覆盖真实维护事务；该权限问题尚未解决。取消与管理员只读诊断 PASS，安装权限问题保持 FAIL。

诊断准备及日志快照位于 `.cache/tests/release/20261008-installer-permissions/`。

## 实现依据

- [WiX 标准 UI 字体与流程](https://github.com/wixtoolset/wix3/blob/develop/src/ext/UIExtension/wixlib/WixUI_InstallDir.wxs)
- [Windows Common File Dialog](https://learn.microsoft.com/en-us/windows/win32/shell/common-file-dialog)
- [Windows Installer 目录目标](https://learn.microsoft.com/en-us/windows/win32/msi/changing-the-target-location-for-a-directory)
