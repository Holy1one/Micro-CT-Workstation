# UI 交互修复留档（失败面、Troubleshooting、全英文、弹窗仪器化）

确认日期：2026-10-08（Asia/Shanghai）。本目录只保留人工筛选后的最小证据集，用于长期对照；原始 run、全部截图与量测在 `tmp/tests/` 与 `tmp/screenshots/`（忽略，不入库）。

## 来源与身份

| 项 | 身份 |
| --- | --- |
| 分支 / HEAD | `native-device-controls-offline` / `11eed6cfea8f3f5118fcd07e70319f98fc398b51`，含既有未提交修改 |
| 候选 host SHA-256 | `0532EAAA6A21EE945878A0E73271205011B7A9383AE1564F67E7B2779CC11DC4` |
| 实际内嵌引擎 SHA-256 | `32D6424758EE266C7E037883AF656E129C683D528FE2C35DA7457FAA8D87A47F`（路径由 `process.json` 核对） |
| 正式入口 SHA-256 | `C19B828D8F18D13D3EF2811B95B5094863AEE0E494611140001BC69716C91334`，未覆盖 |
| 构建命令 | `npm.cmd run candidate:build`（`--offline-candidate`，输出 `release/native-device-controls-offline/`） |
| 验证 run | `tmp/tests/desktop/20261008-042007-ui-final-verify/`（截图 `tmp/screenshots/frontend/20261008-042007-ui-final/`） |
| 缺陷发现 run | `tmp/tests/desktop/20261008-035948-ui-after-verify/`（旧候选 `23D53DF1…`，暴露两个缺陷） |

## 本轮改动范围

- **失败面（U1）**：新增 `src/FailureBanner.tsx`，portal 到 `body`；按「稳定 key + dismissed 集合」去重，新增 `deriveFailureReport` 与 `useDeviceActions.errorAttempt`。`cancelled` 本身不算失败，只有带错误的 cancelled 才升级为 `failed`。
- **Troubleshooting（U2）**：`Help → Troubleshooting…`，5 组导航；`FailureBanner` 与页面共用同一份 `FAILURE_ADVICE`（86 条码，全部存在于 `docs/frontend-error-codes.md`）。
- **字体与 Unknown（U3）**：字号 token 化；X-ray UNKNOWN/未监测保持醒目（`--warnText`），普通只读未知保持中性。
- **扫描准备窗口（U4）**：`WorkstationDialog` 新增 `footer`/`footerNote`；准备窗口固定高度，只有 readiness 报告可滚动。
- **Camera Advanced（U5）**：左侧 8 项导航 + 右侧单 pane（34 项 = 9+5+10+7+3，另有其它实时 descriptor 属性分组），切换分组不发命令。
- **全英文（U6）**：新增 `src/engine/native-labels.ts`；覆盖 34 个参数名（其中 15 个引擎原为中文）、枚举选项、`disabledReason`/`options[].reason`/`capabilities.disabledReasons`；未命中的中文原因降级为一句中性英文，原始文本仍留在日志/诊断/导出。另修正 `UpdateDialog.tsx` 全中文界面与日志 `source` 的中文标签。

## 本轮发现并修复的两个真实缺陷

1. **失败卡片遮挡安全 Stop 控制坞**。控制坞锚定在 3D 场景顶部居中（18px 顶距 + 展开 82px 键位行），失败卡片同为顶部居中，卡片矩形压住坞胶囊；点击坞会打到卡片。修法：`FailureBanner` 按画布 zoom 接收坞车道偏移（208 设计 px），卡片从车道下方开始。
   实测：卡片 top=166 > 坞展开 bottom=153，坞胶囊 `insideCapsule=true`，失败层 `pointer-events:none`、卡片 `auto`。
2. **Camera 栏 Color temperature 与 Mode/Auto ISO/Focus 读数叠字**。第 5 个原生控件加入后，固定栏内容比可用高度多 48 设计 px，而 `.device-section { min-height: 0 }` 允许 flex 子项被压到内容以下，section 变矮后内容画到下一行；`.device-body { overflow: hidden }` 又让 `scrollHeight == clientHeight`，使「无栏内滚动条」断言显示 0 溢出而未捕获。
   修法：收紧 Camera/Turntable 栏节奏（body gap 9→6/5、section gap、grid gap、field gap、控件高 34→30），并对两栏直接子项加 `flex: 0 0 auto`，禁止静默压缩。
   实测：两栏 body `overflow=0`、section `contentOverflow=0`、直接子行两两无重叠。

## 最小留档集

- [full-window-after.png](full-window-after.png)：1536×816 客户区（dpr 2.5、zoom 0.8）整窗布局，画布四边贴合客户区。
- [camera-panel-after.png](camera-panel-after.png)：修复后的 Camera 栏——Color temperature 独立成行，Mode/Auto ISO/Focus 读数不再叠字，全部控件与页脚可见。
- [failure-surface-dock-clearance.png](failure-surface-dock-clearance.png)：失败卡片位于控制坞车道下方，坞（STOPPED 胶囊）可见可点。
- [help-troubleshooting.png](help-troubleshooting.png)：Help → Troubleshooting，5 组导航 + 症状/错误码/具体动作表格 + "What this does NOT mean"。
- [scan-preparation-fixed-height.png](scan-preparation-fixed-height.png)：固定高度准备窗口，header/footer 固定，仅 readiness 报告可滚动。
- [camera-advanced-two-pane.png](camera-advanced-two-pane.png)：Camera Advanced 左导航 + 右 pane + 固定动作页脚。

## 验证与边界

- 离线门禁（最终源码）：`npm.cmd run typecheck` 0；`npm.cmd run test:sites` **186/186**；`npm.cmd run module-map:check` PASS（1969 files / 16 modules / 25 boundaries）。本轮 Rust 未改，故未重跑 `cargo test --workspace`；候选构建本身完成 release 编译。
- 实机运行（离线候选，未连接任何设备）：正常关闭 **app 0 / engine 0**，9400 释放，未强杀。
- 量测：画布贴合客户区（left/top=0，宽高差 ≤1）；Camera/Turntable/X-ray/Operation 栏均无栏内滚动、无内容溢出、无行间重叠；准备窗口与 Advanced 为固定高度（实测 688 = 860 设计 px × zoom 0.8）且仅指定区域可滚动；三处界面 `document.body.innerText` 无 CJK；失败面「出现一次→关闭后 400 ms 轮询不重现→新尝试重新弹出」全部通过。
- **未验证边界**：本轮的视觉量测窗口是 1536×816（dpr 2.5、zoom 0.8、设计画布 1920×1020），**不是原生 1920×1080 窗口**；1920×1080 的精确对齐由静态 `layout-geometry` 断言覆盖（test:sites PASS），但未在 1920 物理窗口下再次目视。2K/4K、DPI 125/150/200/250% 亦未在本轮复测。
- 本轮未连接相机/转台/射线源，未做任何设备动作；设备实机结论仍以 2026-10-08 的非扫描实机验证 run 为准。未发布、未安装、未覆盖正式入口。
