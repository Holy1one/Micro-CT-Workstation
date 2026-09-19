# 当前进度（2026-09-16，第二轮）

## v0.2.0-alpha：控制台重构 + RTS9060 设备链路迁移

- 展示层按 `前端/重构交接说明.md` 全部重做：`src/App.tsx` 与 `src/styles.css` 重写，
  `src/tokens.css`（拷贝自 `前端/tokens.css`）为唯一色源；页面栅格 32/1/1fr/1/220/1/30，
  三栏 352/1fr/376（<1440px 时 320/340）；浅色默认 + 暗色夜间模式（data-theme，localStorage 记忆，
  切换不重挂载、不重置扫描状态）；运行态 READY/SCANNING/PAUSED/FAULT 挂在 `<html data-state>`。
- 中央 Live Scene：主题×转台角度四选一渲染图（`public/assets/3D-scene-{light|dark}[-144].png`），
  右上三个磨砂状态浮窗（X-RAY/CAMERA/SAMPLE）+ LIVE RENDER 指示 + 左下角度读数 + 安全条 +
  底部磨砂玻璃控制坞（4 颗 54×54 圆键，默认只显图标、hover 展开文字标签，图标用设计方 SVG）。
- 设备链路层新增 `src/engine/rts9060/`（移植自 kernal/software/host 已验证实现）：
  行协议编解码（命令名不变）、NanoTransport 接口 + 固件语义执行器（96000 脉冲/圈、
  ACK→READY_TO_CAPTURE 握手、STOP→POSITION_UNKNOWN）、Moxtek 12W 控制器模型（fail-closed 锁存）、
  D7100 拍摄模型（host-only、逐帧校验）、扫描工作流（预检 8 项→回零→逐视角 MOVE_ABS→出束→
  拍照→CAPTURE_DONE→提交；暂停在提交边界生效并保留脉冲计数；急停并行断束+STOP 并锁存 FAULT；
  恢复路径=释放急停→回零→重预检；每视角落盘检查点供 Restore 续扫）。
- `src/engine/workstationAdapter.ts` 把工作流桥接到不变的 `EngineAdapter` 契约；
  `src-tauri/` 与 `crates/ct-engine` 零改动，串口命令名零改动（验收 #14）。
- 数值口径：kV/µA 一律 1 位小数（30.2 kV / 101.0 µA）；角度 2 位小数、四处同值
  （读数/ANGLE/POS/日志），5 视角 → 72.00°/view，序列 0/72/144/216/288；相机 D7100、射线源 USB。

## 验证结果（第二轮）

- `npm run typecheck`：通过。
- `npm run build`：通过（vite 36 模块）。
- `npm run test:sites`：4/4 通过。
- 交互链路实机截图验收（agent-browser，1600×1000）：浅色 READY/SCANNING/PAUSED/FAULT 四态、
  暗色四态、Image Preview（5 帧深色瓦片 + 元数据）、Turntable 子日志独立计数、
  急停→释放→回零→重预检→READY 恢复路径、Restore 恢复 3/5 @144.00° 后 Start 变浅蓝 Resume——全部通过。
- 1600×1000 下 document 与全部面板 `scrollWidth/scrollHeight` 零溢出；
  1280×720（125% 等效）页面栅格固定、侧栏转为栏内滚动（控制台惯例）。
- 浏览器控制台无报错。
- 日志规则：最新在最上、110px 等宽时间戳、PASS 蓝/INFO·OK 绿/WARN 琥珀/ERR 红/ACTION 蓝；
  扫描关键节点（MOVE_ABS、出束窗口、拍照保存、CAPTURE_DONE、暂停/继续/回零/急停/预检）全部落日志。

# 当前进度（2026-09-16）

## 已完成

- 新建 `Micro-CT-App`：Tauri 2 + React/TypeScript + Rust `ct-engine` sidecar。
- UI 已按 `前端/首页.png` 与 `前端/按钮说明.png` 重做为固定 16:9 工业工作台：无整页滚动、左侧设备与参数、中央设备场景、右侧射线/运行状态、底部日志与影像区。
- 中央四键控制舱默认收拢，悬停或聚焦展开；支持回零、开始/暂停/继续、开发预览进度恢复、紧急停止。
- 开发预览支持连接、预检、回零、无射线流程、暂停/继续、恢复与停止；STOP 会使预检和回零失效。
- X-ray 始终 fail-closed；未实现真实设备命令。
- 开发预览不生成或伪造投影，真实影像计数保持 0。
- Windows、digiCamControl、Python 插件、FBP、材料分辨和 3D Viewer 均只有边界与目录预留，尚未实现。

## 验证结果

- `npm run typecheck`：通过。
- `npm run build`：验证时通过；生成的 `dist/` 已在交接清理中删除，可按需重建。
- `npm run test:sites`：4/4 通过。
- `cargo test --workspace`：8/8 通过。
- `npm run engine:build` 与 `npx tauri build --debug --no-bundle`：验证时通过；macOS sidecar 和 `target/` 构建产物已在交接清理中删除，可按需重建。
- Codex in-app browser 1280 × 720：所有主要区域同屏可见，无整页滚动。
- 交互链路已验：连接 → 预检 → 回零 → 开始 → 暂停；恢复进度 → 紧急停止；X-ray 全程闭锁。

## 重要文件

- `src/App.tsx`：固定工业工作台与交互。
- `src/styles.css`：固定布局、工业视觉与悬浮控制舱。
- `src/engine/`：前端 adapter 契约与开发预览。
- `crates/ct-engine/`：Rust 领域状态机与 JSONL 协议。
- `src-tauri/`：桌面壳、sidecar 生命周期与 IPC。
- `public/assets/micro-ct-equipment-scene-v2.png`：中心设备场景素材。
- `docs/ARCHITECTURE.md`：长期混合架构决策。
- `docs/WINDOWS-INTEGRATION.md`：Windows 验收边界。

## 当前限制

- macOS 结果只证明 UI、协议与纯逻辑，不证明 Windows WebView2、设备驱动、CUDA 或安装包。
- UI 中的 X-ray 数值、Timer 和 Max X-ray 是第一版交互占位，未进入真实控制命令。
- “恢复进度”目前只恢复开发预览检查点，没有读取持久任务文件。
- 真实相机图片到达前，Image Preview 只显示空槽。

## Windows 验证

实测日期：2026-09-16。目标为 Windows 11 x64 桌面基线；未连接、枚举或驱动任何真实设备，未执行相机拍摄或 X-ray 输出。

### 环境证据

- OS：`cmd /c ver` 返回 `10.0.26200.9445`；注册表 `DisplayVersion=25H2`、`CurrentBuild=26200`、`UBR=9445`。注册表 `ProductName` 仍返回旧标签 `Windows 10 Pro`，本记录保留该差异，不据此改写系统版本。
- Node.js `v24.13.1`，npm `11.8.0`。PowerShell 执行策略阻止 `npm.ps1`，本次使用同一安装目录的 `npm.cmd`/`npx.cmd`。
- Rust stable `1.98.1 (48a229cea 2026-09-01)`，Cargo `1.98.1`，host/target `x86_64-pc-windows-msvc`。
- Visual Studio Build Tools 2026 `18.0.2`（installation version `18.0.11222.15`）；MSVC `cl 19.50.35719`、link `14.50.35719.0`；Windows 11 SDK `10.0.26100.0`。
- Microsoft Edge WebView2 Runtime `153.0.4234.32`；运行中的 WebView2 user agent 同为 `Edg/153.0.4234.32`。
- .NET：x64 host/runtime `8.0.11`，Windows Desktop Runtime `8.0.11`；.NET Framework `4.8.09221`（release `533509`）；没有安装 .NET SDK。

### 构建与测试

- `npm ci`：成功，按 lockfile 安装 74 个包。沙箱内首次运行因 registry 请求 `EACCES` 并触发 npm `Exit handler never called`；允许联网并把 cache 放到 `%TEMP%\micro-ct-npm-cache` 后成功。
- `npm test`：最终 4/4 通过（含 `tsc --noEmit`）。一次与 `npm run build` 并行的检查在 `dist/client/index.html` 生成前触发，临时为 3/4；build 完成后顺序复跑通过，不是产品测试失败。
- `npm run build`：通过，Vite 转换 4575 个模块；生成 `dist/client/index.html`、`dist/server/index.js` 和 `dist/.openai/hosting.json`。
- `cargo test --workspace`：最终 8/8 通过。干净目录首次运行在 Tauri build script 检查 sidecar 时报告 `binaries\ct-engine-x86_64-pc-windows-msvc.exe doesn't exist`；按项目脚本生成 sidecar 后复跑通过。Cargo 首次取依赖还受用户级失效代理 `127.0.0.1:7892` 影响，本次仅对下载命令临时清空代理，没有修改全局配置。
- `npm run engine:build`：通过，生成 `src-tauri\binaries\ct-engine-x86_64-pc-windows-msvc.exe`，实测大小 693,248 B。
- `npx tauri build --debug --no-bundle`：通过；产物为 `target\debug\ct-workstation.exe`。
- debug 桌面程序成功启动，窗口标题 `Micro-CT Workstation`，同时启动同目录 `ct-engine.exe` sidecar，WebView2 页面为 `http://tauri.localhost/`。

### 布局与交互

- 100%：原生窗口 WebView 客户区实测 `1600 × 900`、`devicePixelRatio=1`。document/body 的 `scrollWidth × scrollHeight` 均为 `1600 × 900`，滚动偏移为 0；主区、左右栏、中央场景、188 px 底栏和 24 px 状态栏全部在 viewport 内。
- 125%：两块物理显示器都只报告 96 DPI，因此未改用户系统设置；改用同一 WebView2 Runtime 的 `--force-device-scale-factor=1.25` 验证。实测 `devicePixelRatio=1.25`、有效 CSS viewport `1280 × 720`，document/body 均无溢出，三栏、166 px 底栏和 22 px 状态栏全部同屏。此结果证明 WebView2 缩放布局，不替代在真实 125% 显示器上的最终人工验收。
- Windows debug 桌面中完整执行：连接 → 预检 → 回零 → 开始 → 暂停 → 继续 → 紧急停止；随后执行加载上次进度 → 紧急停止。每一步均到达预期 phase。
- 两次 STOP 后预检从 `PASSED` 失效为 `READY`，Home 与 Start 均禁用；继续前必须重新预检和回零。
- 全流程 X-ray Enable 均禁用，底栏保持 `X-RAY SAFE / DISABLED`；`imageCount=0`，五个 Image Preview 槽均为 `Awaiting capture`，影像区没有图片元素。
- `ct-engine.exe` 不带 `--preview` 直接启动时，开发预览连接返回 `PRODUCTION_LOCKED`；snapshot 为 `production_locked`、`xrayEnabled=false`。生产路径没有自动降级到模拟状态，STOP 仍可幂等执行。

### digiCamControl 与 adapter 边界

- Windows 卸载注册表记录 digiCamControl `2.1.7.0`，缓存安装包为 `C:\ProgramData\Package Cache\{c33d2323-4e2f-4b42-92a7-9f89f14ea398}\digiCamControlsetup_2.1.7.0.exe`。
- 注册的应用目录 `E:\Application\digiCamControl` 当前为空，常见路径中没有 `CameraControl.exe` 或 `CameraControl.Core.dll`；没有运行中的 digiCamControl 进程/服务。结合“没有 .NET SDK”，本机当前不能构建或验证真实 .NET bridge，也没有证据可选择 SDK 目标框架。
- 最小 bridge 保持独立 .NET 进程、UTF-8 JSONL stdio 和版本化 envelope，只接受 engine 发出的 discover/connect/health/configure/capture/stop/disconnect；文件大小、SHA-256、远端删除确认与设置快照必须回传。详细 v1 契约已写入 `bridges/digicam/README.md`。
- Nano、Nikon、Moxtek 的后续 Rust adapter 落点为 `crates/ct-engine/src/adapters/`；`ct-engine` 继续作为唯一设备/任务所有者。未获得实际 SDK/协议与明确硬件授权前不创建可启用实现，不猜测命令。

### 未完成项

- 尚未在真实 Windows 125% 显示设置下人工复核；本轮使用 WebView2 强制 DPR 1.25。
- digiCamControl 主程序/SDK 程序集和 .NET SDK 缺失，bridge 只有契约，没有可执行文件；Nikon 发现、连接、拍摄、传输校验和远端删除均未测试。
- Nano 串口、Moxtek、硬件联锁、真实急停链、相机、CUDA、Python worker、代码签名和安装包均未验收。本次“通过”只表示 Windows debug 软件基线、开发预览状态机和默认闭锁行为通过，不表示硬件验收。

### Windows UI 重构复验（2026-09-16）

- 重新逐页检查 `前端/前端设计.pptx` 全部 4 页以及 `前端/首页.png`、`前端/按钮说明.png`，按参考的信息架构重构，而非照搬渲染图的卡片视觉。
- 移除重复的自绘应用标题栏，原生 Windows 标题栏下直接进入 `File / Edit / Tools / Help` 菜单行；可见界面、状态、错误和日志统一为英文。
- 三栏在 1600 × 900 下实测宽度约为 381 / 814 / 381 px；左右栏由原 292 px 加宽，中间场景缩小且图片使用 `contain`，避免挤压设备与控制信息。
- 字体提升到工作台可读尺寸：应用正文 13 px、设备名称 12 px、日志 11 px；125% 验证下分别为 12 / 11 / 10 px。
- 三台设备各有独立状态灯和 Refresh 控件；审计确认 `.device-refresh` 数量为 3。当前刷新只重读 engine snapshot，不伪造独立硬件连接结果。
- 底栏恢复为 5 个选择项。默认日志使用浅色工业配色和无分隔线的终端式连续行，时间格式为 `YYYY-MM-DD HH:mm:ss`；Image Preview 只在第五项激活后替换日志内容。
- 修复 React 条件分支复用导致的日志残影；日志与图片区使用独立 key。Image Preview 激活后日志容器不存在，5 个空槽强制单行：100% 下每槽约 269.8 px，125% 下每槽约 209.8 px；真实图片数仍为 0。
- 右下仅显示 Operation Status、流程节点和当前 Task / Scan setup / Safety 摘要，不再常驻 Image Preview。
- 使用 `前端/logo.png` 居中补白为正方形图标源，通过 `npx tauri icon` 生成 Windows `icon.ico` 等资源；从最终 `ct-workstation.exe` 提取的关联图标已目视确认一致。
- 1600 × 900 / DPR 1.0 严格几何审计通过：document、主区、左右栏、中央场景、四个侧栏面板、底栏、日志导航和日志内容均无 `scrollWidth/clientWidth` 或 `scrollHeight/clientHeight` 溢出。
- 1280 × 720 / DPR 1.25 严格几何审计通过：左侧设备与参数面板客户高度均为 250 px，参数区 `scrollHeight=clientHeight=250`，Apply Parameters 完整可见；其余所有主面板同样无滚动或隐藏截断。
- 最终 Windows debug 中复验：逐设备刷新 ×3 → 连接 → 预检 → 回零 → 开始 → 暂停 → 继续 → STOP → 加载上次进度 → STOP。两次 STOP 后预检和回零均失效，Home/Start 禁用，X-ray 保持 `SAFE / DISABLED`，Image Preview 没有伪造图片。

# 当前进度（2026-09-18，Windows 桌面菜单与原生对话框验收）

## 下拉菜单接线

- `File / Edit / Tools / Help` 共 15 项全部接真实功能，没有占位：`src/menuActions.ts` 定义，
  `App.tsx` 的 `handleMenuAction` 分发；`Edit` 的 Undo/Redo 因引擎没有撤销栈而长期置灰并给出原因。
- `Edit > Preferences` 为真实主题切换浮层；`Help` 三页内容（操作顺序 / 安全须知 / 关于）均实读引擎状态。

## 原生对话框真实点击验收（CDP 驱动，全程不抢焦点）

- 驱动方式：应用以 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port` 启动，
  页面用 CDP 的 `Input.dispatchMouseEvent` 点真实坐标，原生对话框用 `BM_CLICK` 窗口消息操作；
  窗口只在点击期间 `SW_SHOWNOACTIVATE`（显示但不激活），结束后立即最小化归还。
- 目录选择器（`docs/shots/qa_directory_picker_verify.py`，证据 `directory-picker-verify.json`）：
  真对话框为 `#32770` / `Select projection image directory` / 按钮 `选择文件夹·取消·帮助(&H)`；
  取消后 Save Path 保持原值；在对话框内导航到另一个目录并确定后，输入框与摘要同步变为该目录，
  Explorer 打开的也是该目录。
- 保存对话框（`docs/shots/qa_save_dialog_verify.py`，证据 `save-dialog-verify.json`）：
  真对话框 `#32770` / `Export session log`；取消不写盘；选择目标后写出 607 字节日志，
  内容含 `Micro-CT Workstation session log` 与当前 Save Path；`.exe` 目标没有产生任何文件
  （对话框过滤器会补 `.log`，真正的拒收逻辑由 `cargo test` 覆盖）。
- 缺陷修复：`File > Open Image Folder` 原先只 `dispatch` + `revealDirectory`，没有 `setupPendingSync`，
  导致选完目录后 Save Path 输入框仍显示旧路径；已修复并由
  `tests/menu-wiring-contract.test.mjs` 的 “File > Open Image Folder refills the Save Path draft after picking” 防回归。

## Headless 布局审计

- 新增 `docs/shots/qa_layout_measure.py`：headless Edge + CDP，测量 1600×900 / 1920×1080 / 1280×720。
  三档下 `document` 的 `scrollWidth×scrollHeight` 与 `clientWidth×clientHeight` 完全一致，
  五个面板的纵横向溢出全部为 0，没有裁切；中央场景区底部余量 8–12 px（固定 16:9 画布缩放的自然结果），
  无需修正。证据 `docs/shots/layout-measure.json`。

## 菜单功能端到端验证（真实点击，CDP 驱动）

- 新增 `docs/shots/qa_menu_functions_verify.py`，证据 `docs/shots/menu-functions-verify.json`（`verdict: true`）。
  15 项菜单之外又逐个点击了有副作用的条目并读取 DOM 结果：
  - `File > New Scan Task` → Task ID 变为 `scan-20260918-160834`（符合 `scan-\d{8}-\d{6}`）
  - `Edit > Reset Parameters` → 三个字段回到 120 / 200 / `00:10:00`（= 600 s）
  - `Edit > Undo/Redo` → 仍为 `disabled`，提示 `engine has no undo stack`
  - `Edit > Preferences` → 标题 `Preferences`；点 Dark 后 `data-theme=dark`，点 Light 后回到 `light`
  - `Tools > Run Preflight` → 新增日志 `8/8 preview checks passed · real interlocks unverified`
  - `Tools > Home All Axes` → 新增日志 `Preview HOME complete · 0.00°`
  - `Tools > Device Diagnostics` → 列出 X-Ray Source / Turntable-Nano / Camera（外加 Link summary）
  - `Help` 三页 → 标题分别为 User Guide / Safety Notes / About Micro-CT Workstation，About 正文含 `ct-engine`
- 判据采用**日志增量比对**（点击前后快照求差），避免用历史日志里恰好出现的 preflight/home 字样蒙对。
- 截图证据已移除：同一窗口两次 `PrintWindow` 抓到的帧字节完全相同（MD5 一致），说明未激活状态下
  抓的是合成缓存帧，不能作为视觉证据；本项以 DOM 断言为准。

## 校验与新增资产

- `tsc --noEmit` 通过；Node 契约测试 21/21 通过；`cargo test` 12/12 通过。
- 新增：`docs/shots/qa_directory_picker_verify.py`、`qa_save_dialog_verify.py`、`qa_layout_measure.py`，
  证据 `directory-picker-dialog.png`、`save-dialog-native.png` 与三个 JSON。
- 验收产物均落在 `%TEMP%` 并在脚本结尾清理，不留在用户目录。

# 当前进度（2026-09-19，X 射线真实控制与设备端死开关验收）

## Moxtek 设备端机制（真机实测 + 官方软件反编译交叉验证）

- 官方 `12WattControllerVer001`（.NET ClickOnce，桌面 `12W-Software`）反编译出三个此前未知的命令：
  `0x74` = USB Auto Shut Down 开关（payload 1=armed / 0=released），`0x76` = 设延时（u16），
  `0x77` = 读延时。官方监控定时器 500ms 才轮询一次。
- 设备端死开关实测：armed 状态下最后一次 enable 后约 0.27 s 固件自动清除出束标志并受控降压；
  只读轮询（GET_STATUS）喂不住，重发 enable（≤100ms）可以。因此官方软件也必须取消勾选才能持续出束。
- 管子无故障：20/30/40/60 kV 四个功率点稳态输出与设定误差 <1.2%（`qa_xray_setpoint_sweep.py`）。
- 早前"出束成功但秒掉"的根因即此死开关；此前所有未持握通信的出束均为爬升瞬态碰巧通过判定。

## 本轮改动（crates/ct-engine + src-tauri）

- `xray.rs`：连接时先读状态——检测到遗留束流则如实上报 `beam_on`（UI 红按钮），不静默强制关；
  新增 `refresh_status`（非破坏性轮询）与 `set_usb_auto_shutdown(0x74)` / `read_usb_shutdown_timer(0x77)`；
  `XrayHealth` 增加 `beam_on` / `usb_auto_shutdown` / `usb_shutdown_delay`。
- `lib.rs`：生产模式 `xray_toggle` 实现真实手动开关束——关束永不锁定；开束要求
  射线已连接 + 未 busy + 未锁存 + SEND V/I 已确认 + **USB Auto Shut Down 已释放**
  （勾选态拒绝 `SAFETY_LOCK_REQUIRED`，即"勾选=锁定，释放=授权"）。
  `usb_auto_shut_down_toggle` 现在真实下发 0x74 到设备；连接（retry_device/preflight）时把引擎
  勾选状态同步到设备。`xray_connected` 不再要求 `beam_off_confirmed`（出束中也算已连接）；
  `manualControlsEnabled` 只要求射线连接，脱离 preflight/HOME（X 射线可脱离 CT 单独控制）。
  新增 `shutdown()`：stdin EOF 后确定性 stop scan + force OFF + 断开全部设备。
- `main.rs`（ct-engine）：stdin EOF（窗口关闭/壳崩溃/被杀）后显式 `engine.shutdown()`。
- `engine_client.rs`：Drop 改为 stop → 关闭 stdin（触发引擎 EOF 清理）→ 最多等 8 s → 卡住才 kill；
  `xray_toggle` IPC 超时 20 s。
- `nano.rs`：fail-fatal 会话模型改容错——命令级错误（ERR/安全拒绝/单次超时）只返回给调用方，
  会话保持可恢复；心跳丢失与意外异步行记录后继续；仅传输层错误判 Lost。
  这是"扫描转完一个角度不继续"的主要修复（此前一次串口抖动即永久断链）。

## 真实硬件验收（`docs/shots/qa_xray_real_beam.py`，证据 `xray-real-beam-verify.json`，VERDICT true）

- ARMED（勾选）手动开束被拒 `SAFETY_LOCK_REQUIRED`；释放后 20 kV / 50 µA（1 W）真实出束
  **无重发 enable 持握 8 s**，回读 20.02–20.13 kV / 48.8–50.2 µA，与设定一致。
- 手动关束确认；重新 ARMED 后开束再次被拒。
- 释放态出束中 stdin EOF（模拟关窗）→ 引擎退出码 0 → 重连确认束流已关（软件清理链有效）。
- 释放态出束中 `taskkill /F` → 束流物理保持 → 重连**检测到 beam_on（红按钮态，20.0 kV / 49.8 µA
  实测仍在输出）**→ 手动 Xray Disable 关束确认。遗留束流检测链路真实有效。
- 注意：armed 死开关只约束"armed 期间的出束"；对释放态下已存在的遗留束流无追溯力，
  必须靠连接检测 + 手动关闭（已按此实现并验证）。

## 校验

- `cargo test --workspace`：ct-engine 32/32、ct-workstation 12/12 通过（含新增的
  连接束流检测、0x74/0x77、命令容错恢复用例）。
- `npm run typecheck` 通过；`npm run test:sites` 22/22 通过。

## 未验收项

- 完整真实扫描链（4 视角出束+拍照）依赖 digiCamControl/D7100，本机尚未安装相机控制端；
  Nano 容错修复与 X 射线持握已分别单体验证，整链待相机就位后跑 `tmp/hw_runtime/run_rust_engine_4pt.py`。
