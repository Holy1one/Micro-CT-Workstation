# Micro-CT Workstation

Micro-CT-App 是 Windows 工业上位机工程：React 提供操作界面，Tauri 提供桌面能力，Rust `ct-engine` 统一控制 Moxtek 射线源、Nikon D7100 相机和 RTS9060 Nano 转台。

## 架构原则

- `ct-engine` 是设备连接、安全门控、扫描状态和投影事务的唯一权威。
- Tauri 只管理窗口、原生路径/对话框、sidecar 生命周期和 IPC 转发。
- React 只发送领域命令并渲染完整快照；3D 场景只读展示。
- 浏览器开发预览完全离线，不能访问真实设备；Tauri IPC 失败时不能回退到预览。
- 每个投影在转台到位、出束状态监测、主机侧图像文件确认和 `CAPTURE_DONE` 全部成功后才提交；射线在设定的连续出束时间内跨投影保持开启，到时关束冷却。

完整架构入口见 [`docs/architecture/README.md`](docs/architecture/README.md)，逐目录说明见 [`docs/architecture/directory-map.md`](docs/architecture/directory-map.md)，PC 直控与未来 Arduino 主控双线方案见 [`docs/architecture/dual-control-topology.md`](docs/architecture/dual-control-topology.md)。

## 核心模块

- `crates/ct-engine/src/devices/xray/`：Moxtek 通信、实测回读和 fail-closed 关束。
- `crates/ct-engine/src/devices/camera/`：D7100 / DigiCamControl 和主机文件确认。
- `crates/ct-engine/src/devices/turntable/`：Nano 协议、运动确认、警告和急停。
- `crates/ct-engine/src/scan.rs`：跨设备投影事务编排。
- `src-tauri/`：薄桌面壳和 versioned JSONL sidecar 客户端。
- `src/engine/`：前端契约、生产 adapter 和浏览器预览。
- `module-map/`：机器可读模块归属与隐藏依赖。

## Windows 入口

Windows 11 x64 是真实设备集成与最终发布平台。安装 MSVC Build Tools、WebView2、Rust `x86_64-pc-windows-msvc`、Node 与 .NET 后运行 `npm.cmd run tauri:dev`。真实相机、串口和射线操作必须取得本次明确授权，详见 `docs/WINDOWS-INTEGRATION.md`。

## 免安装版发布与启动

运行 `npm.cmd run portable:build`，将当前前端、桌面壳和内嵌引擎一起编译，并更新唯一交付入口 `portable-release/micro-ct-workstation-portable.exe`。直接双击该 EXE，无需 CMD 或批处理启动器；Windows 需具备 WebView2 运行时。同目录 `build-info.json` 记录构建时间、源提交、工作区是否含未提交改动和 EXE SHA256。

`npm.cmd run build` 只更新网页产物，不能更新桌面 EXE。`target/release/` 是 Cargo/Tauri 编译输出和缓存，不作为用户启动入口；关闭相关程序后可删除该目录，但下次构建会重新生成并增加编译时间。日常保留编译缓存，只使用 `portable-release/` 中的正式交付副本。

应用将内嵌 `ct-engine` 校验并释放到本机运行时目录，在后台通过管道通信。引擎进程必须运行，但无需显示控制台；退出时请关闭主窗口，由应用执行停止、关闭管道和引擎清理。

## 界面尺寸与窗口规则

界面以 1920×1080 为设计基准，必须铺满窗口客户区四边。控件使用统一缩放系数，网格宽高随实际客户区适配；1920×1080 桌面扣除标题栏和任务栏后也不得出现左右留边。超宽或较矮客户区由中央区域和布局尺寸吸收比例差异，不拉伸文字、不裁切控件、不引入固定栏目滚动。

桌面默认最大化。物理屏幕宽度 ≤1920 或高度 ≤1080 时不允许保持窗口化；高分屏在 DPI 换算后工作区小于 1888×1072 逻辑像素时也采用这一限制。限制模式保留最小化和关闭，禁用原生缩放/最大化按钮，并在还原、拖拽或跨屏移动后重新应用最大化策略。其他屏幕允许窗口化，最小客户端为 1728×972 逻辑像素（再按可用工作区上限收敛）。

射线实测缺失或读取失败时显示“—”，输出状态区分已确认 ON、已确认 OFF 和 UNKNOWN；设定值始终单独展示。控制服务连接与设备连接分别显示。界面不拥有设备或扫描状态。

客户区顶部为 26px 紧凑菜单，不放品牌标识或重复 Production 文案。解释通过圆圈问号悬停查看，输入错误直接显示。投影数接受 1–3600 的任意正整数；曝光单位 ms，D7100 额定定时快门为 0.125–30000 ms，连接后按相机枚举的上下限及原生档位校验并读回确认。最长出束时间按 min 输入，默认 10；引擎仍使用秒并保留既有冷却规则。

日志栏使用 270 个设计像素高度，上方参数区和设备视图随之收紧。操作坞采用更透明的磨砂底，按钮为 70×58，图标保持 34×34。3D 转台与样品依据确认的转台脉冲位置换算角度，并在反馈之间做 300 ms 匀速过渡；数值读数始终是确认角度。暂停、停止、失联及完成会终止过渡或定位到最终确认姿态，界面不会自行预测下一步运动。

## 开发与验证命令

```powershell
npm.cmd ci
npm.cmd run dev -- --port 4173 --strictPort
npm.cmd run tauri:dev
```

验证：

```powershell
npm.cmd run typecheck
npm.cmd run test:sites
npm.cmd run module-map:check
cargo test --workspace
```

运行 `npm.cmd run impact` 可根据当前 Git 工作区列出所属模块、受影响模块、强依赖和必跑门禁。
