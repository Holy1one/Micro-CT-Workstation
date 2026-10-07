# Camera / Turntable 两栏重设计确认

确认日期：2026-10-07（Asia/Shanghai）。使用者本轮明确选择保留当前 tooltip 与高级弹窗显示：Camera 字段的 Actual 保留在 tooltip 和 Advanced；Turntable 规定方向保留分区微标题和 tooltip。本轮没有进一步修改视觉源码。

## 来源与身份

截图来自已经构建并启动验证的候选 `release/frontend-panel-redesign-20261008/micro-ct-workstation-panel-redesign.exe`。原有目录名含 `20261008`，本轮保留原名；确认日期以本文件为准。

| 项 | 身份 |
| --- | --- |
| 分支 / HEAD | `native-device-controls-offline` / `11eed6cfea8f3f5118fcd07e70319f98fc398b51`，含已有未提交修改 |
| 候选 host SHA-256 | `56EC7FABF9770334302CF270B5504891CFCB12C1199621D2950D797A6F03ECA8` |
| 实际内嵌引擎 SHA-256 | `6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7` |
| 正式入口 SHA-256 | `C19B828D8F18D13D3EF2811B95B5094863AEE0E494611140001BC69716C91334`，未覆盖 |
| 原截图 run | `tmp/tests/desktop/20261008-011500-verified-shots/` |
| 原截图命令 | `node shots.mjs <tmp/screenshots/frontend/run-id>`、`node dialogs.mjs <同目录> verified`、`node fits.mjs`，经真实 Tauri WebView/CDP |

源码范围：`src/CameraPanel.tsx`、`src/TurntablePanel.tsx`、`src/DeviceConsoles.tsx` 与 `src/styles.css`。两栏及 Advanced/Bench 子页面沿用设备分组、井式控件、等宽读数与状态页脚；右列和其它栏目的视觉保持原实现。

## 最小留档集

- [Camera](camera.png)：设备选择、统一保存根、曝光字段、读回/Apply、独立拍摄和 Live View 入口。
- [Turntable](turntable.png)：确认位置、规定方向运动、Bench 与扫描准备入口。
- [Camera Advanced](camera-advanced.png)：摄影目录、Actual/Target、预设和运行诊断。
- [Turntable Bench](turntable-bench.png)：现有物理隔离确认页面。

这四张截图用于保留使用者已确认的两栏和子页面布局，未复制其它栏目的截图、缓存或设备日志；不含真实 Live View 帧或照片预览。

## 验证与边界

原截图会话记录 app/engine 正常退出 0/0；原量测 Camera body 548/548、Turntable body 297/297，overflow 均为 0。本轮同一候选又完成三次受控相机会话，正常退出均为 0/0；无视觉改动，因此沿用已构建候选与原量测，不把文档归档描述成重新设计或重新编译。

前端门禁与实机设备证据分别记录；最新门禁见本轮 `tmp/tests/desktop/20261007-191045-t7-short-exposure/` 留档的检查结果。相机 T7 与 F-12 复测见 [后端重验证 §5z](../../backend-revalidation-2026-10-06.md#5z-第-19-轮t7-独立-nef-pass与-f-12-长曝光对照)。完整射线联动、HOME/MOVE、扫描、重构精度和更新维护锁没有因此取得硬件认证。没有发布、安装或覆盖正式入口。
