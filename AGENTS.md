# Micro-CT-App 工作约定

本应用Micro-CT的目标是构建 Windows 桌面上位机：PC 独立控制 Moxtek 射线源、Nikon D7100 相机和 CH340/Nano 转台，并把命令、遥测、图像与扫描事务统一收敛到 Rust `ct-engine`。

## 执行前必读

开始任何涉及本仓库的实现、修改、测试设计、目录调整或架构判断前，必须先完整阅读：

1. `docs/architecture/README.md`：当前整体软件架构、控制链、模块边界和投影事务执行逻辑；
2. `module-map/README.md`：模块归属、隐藏依赖、影响分析和目录冻结机制；
3. `docs/architecture/directory-map.md`：根目录及重要子目录的具体职责和禁止混放的内容。

如果任务涉及新增/移动文件、调整模块边界、修改 IPC/JSONL/设备协议、创建目录或引入新依赖，还必须在动手前查看：

- `module-map/modules.json`：路径所有者、风险和必跑门禁；
- `module-map/edges.json`：编译器不可见的强依赖；
- `module-map/directory-policy.json`：允许存在的受控目录结构；
- `docs/architecture/module-graph.md`：当前生成的模块依赖关系。

读取后先按实际任务确定所属模块、受影响模块、允许编辑范围和验证门禁，再开始修改。源码与测试高于文档；发现必读文档与当前源码不一致时，先报告差异并在同一任务中修正文档，不得继续依赖已知过期说明。

## 架构

```text
React / TypeScript
  -> Tauri invoke
  -> EngineClient
  -> versioned JSONL stdio
  -> ct-engine
       -> NanoAdapter
       -> MoxtekAdapter
       -> CameraAdapter
```

- `crates/ct-engine/`：生产领域内核；唯一持有设备连接、预检、安全门控、扫描状态和事务进度。
- `src-tauri/`：Windows 桌面壳、原生路径/对话框、sidecar 生命周期与 IPC 转发。
- `src/`：React 表现层；只消费完整快照并发送命令。
- `src/engine/rts9060/` 与 `WorkstationAdapter`：浏览器开发预览，不访问真实硬件。
- `tests/`：前端、布局和桌面契约测试。
- `docs/`：架构或验收说明；内容必须与当前源码一致，历史材料应明确标记为历史。

## 控制逻辑

1. UI 把操作转换为领域命令，生产模式只通过 Tauri 发送给 `ct-engine`。
2. `ct-engine` 分别连接三类设备，验证身份与能力后才允许预检和扫描。
3. 每个投影视为事务：转台到位并确认、射线警告与出束确认、相机拍摄及文件确认、关束确认、`CAPTURE_DONE`、提交进度。
4. 暂停只在安全提交边界生效；STOP、E-STOP、掉线、超时和未知状态一律 fail-closed。
5. 恢复不得伪造旧安全条件；按当前状态重新建立设备连接、预检和 HOME 等必要条件。

Nano 串口协议为外部设备契约，当前核心命令/响应包括 `HEARTBEAT/HBACK`、`PING`、`INFO`、`STATUS`、`SET_MICROSTEPS`、`REARM`、`HOME`、`MOVE_ABS`、`MOVE_REL`、`CAPTURE_DONE`、`STOP`、`GET_HALL` 和 `XRAY_WARNING`。当前工作区没有活动 Nano 固件源码；不得读取封存源码或靠猜测改变协议。确需升级时，先建立新的活动固件来源，再同步设备固件、Rust adapter、预览实现与测试。

## 不变量

- Tauri IPC 失败时不得回退到浏览器模拟实现。
- React、Tauri 和 3D 场景不得保存或推进第二份生产扫描状态。
- 射线设定值不得冒充实测值；关束未确认时不得报告安全完成。
- D7100 使用主机侧保存与文件确认，不得静默回退到相机存储卡。
- 真实设备动作遵循根 `AGENTS.md` 的授权要求；测试和开发预览不得访问真实设备。
- `target/`、`dist/`、`node_modules/`、`src-tauri/binaries/`、日志、崩溃转储和会话记忆都是可再生成内容，不进入设计事实源。

## 修改边界

- 改 `crates/ct-engine/`：检查安全状态机、三类 adapter、JSONL 契约和 Tauri 客户端。
- 改 `src-tauri/`：保持壳层轻量，不把设备或扫描状态移入 Tauri。
- 改 `src/engine/types.ts` 或命令字段：同步 Rust DTO、预览实现和 IPC 契约测试。
- 改 `src/scene/`：仅改变展示与相机交互，不发送设备命令。
- 改硬件接口或 Nano 协议：同时检查 `Micro-CT-Hardware/` 与根级跨域契约。

## 单一事实来源

| 关注点 | 声明位置 | 主要消费方 |
|---|---|---|
| 模块路径归属、风险与门禁 | `module-map/modules.json` | 图谱生成、影响分析、开发者 |
| 编译器不可见的强依赖 | `module-map/edges.json` | 影响分析、评审与验收 |
| 当前架构与执行逻辑 | `docs/architecture/` | 所有模块与未来迁移设计 |
| JSONL 请求/响应 DTO | `crates/ct-engine/src/lib.rs` | sidecar、Tauri `EngineClient` |
| 前端命令与快照类型 | `src/engine/types.ts` | React、生产/预览 adapter |
| Nano 外部协议实现 | `crates/ct-engine/src/devices/turntable/nano.rs` | `ct-engine`；活动固件建立后做跨仓一致性校验 |

源码和测试高于文档，当前架构文档高于历史计划。机器生成图谱只用于导航，不替代源码证据。

## 模块化变更纪律

1. 非平凡改动先运行 `npm.cmd run impact`；已有基线时使用 `node scripts/impact.mjs --base <verified-ref>`。
2. 新增或移动路径时，同一改动更新 `module-map/modules.json`；新增字符串协议、跨进程或语义依赖时更新 `module-map/edges.json`。
3. X 射线、相机、转台分别在 `crates/ct-engine/src/devices/` 下独立开发；设备模块不得互相调用，跨设备顺序只由扫描协调器持有。
4. 未来 Arduino 主控作为新的 device backend 接入，必须保留 PC 直控实现和同一领域契约；两条路径不得静默回退，Arduino 不得成为第二个未经对账的生产扫描状态源。
5. 运行影响模块列出的门禁；未运行项明确报告 `BLOCKED`。

## 目录冻结与新增申请

当前顶层目录和 `module-map/modules.json` 声明的功能分区视为已冻结。优先把新文件放入现有职责最匹配的目录，不得为了方便创建 `new`、`misc`、`temp`、`common`、`utils2`、个人姓名或任务编号等含义不清的持久目录。

受控目录的当前直接子目录同时记录在 `module-map/directory-policy.json`，由 `npm.cmd run module-map:check` 强制检查。目录申请获批后，代码、相邻 README、`modules.json`/`edges.json`（按影响需要）和 `directory-policy.json` 必须在同一改动中更新。

以下情况必须在创建目录之前向人工发送一次申请，并等待明确批准：

- 创建任何新的顶层目录；
- 在任意层级创建一个代表新设备、新服务、新进程、新数据格式、新插件类别或新领域职责的目录；
- 新内容无法合理归入当前任何模块，或需要修改 `module-map/modules.json` 才能获得所有者；
- 为未来设想提前创建空目录、占位目录或未实现 backend。

申请必须使用以下信息，不得先创建再补申请：

```text
[NEW DIRECTORY REQUEST]
Proposed path: <仓库相对路径>
Owning module: <现有模块或拟新增模块>
Reason: <为什么需要这项功能>
Why existing directories are insufficient: <逐项说明不能放入哪里>
Planned contents: <文件类型、入口和公开接口>
Dependencies and affected modules: <调用方、被调用方、强依赖>
Lifecycle: <tracked source | generated | cache | test evidence>
Git policy: <tracked 或对应的 .gitignore 规则>
Verification: <新增后需要运行的门禁>
```

等待审批时可以继续不依赖该目录的工作，但不得创建目录、占位文件或把内容临时塞进错误模块。人工拒绝后必须使用现有目录或重新提交不同方案。

无需单独申请的情况只有：

1. 工具在下表固定位置生成编译/缓存目录；
2. 在 `tmp/` 固定分类下创建一次运行目录；
3. 在现有模块内部仅为拆分同一职责而创建实现子目录，且没有新公共边界、进程、协议或所有者。此类改动仍须同步相邻 README、`module-map/modules.json`、`module-map/directory-policy.json` 和相关 import；如果是否属于“同一职责”存在疑问，按需要申请处理。

## 编译、缓存、调试与测试产物

所有可再生成内容只允许进入下列固定位置，不得散落在源码目录、仓库根或 `docs/`：

| 内容 | 固定位置 | Git 策略 |
|---|---|---|
| Node 依赖 | `node_modules/` | 忽略 |
| Rust/Cargo 编译与测试缓存 | `target/` | 忽略；禁止在子 crate 设置第二个 `target/` |
| Vite 缓存 | `.vite/` | 忽略 |
| 前端/Sites 构建输出 | `dist/` | 忽略 |
| Tauri sidecar 副本 | `src-tauri/binaries/` | 忽略 |
| 通用工具缓存 | `tmp/cache/<module>/` | 忽略 |
| 自动/过程测试输出 | `tmp/tests/<module>/<run-id>/` | 忽略 |
| 调试日志、trace、dump | `tmp/debug/<module>/<run-id>/` | 忽略 |
| 临时运行会话、模拟数据 | `tmp/sessions/<module>/<run-id>/` | 忽略 |
| 未筛选截图和视觉对比 | `tmp/screenshots/<module>/<run-id>/` | 忽略 |
| 覆盖率输出 | `coverage/` | 忽略 |
| 经人工筛选的长期验收证据 | `docs/shots/<YYYY-MM-DD>-<topic>/` | 跟踪；必须有说明文件 |
| 发布候选中间产物 | `release/` | 忽略；正式便携包按发布流程进入 `portable-release/` |

`<module>` 使用稳定短名：`ct-engine`、`xray`、`camera`、`turntable`、`frontend`、`preview`、`scene`、`desktop`、`tooling`。`<run-id>` 使用 `YYYYMMDD-HHmmss-<short-topic>`，不得使用“final”“new”等无法追溯的名称。

任何会生成文件的简单过程测试，都把本次产生的日志、JSON、图片、临时输入和输出统一放进一个 `tmp/tests/<module>/<run-id>/`。至少保存：

- `command.txt`：实际执行命令和必要参数；
- `result.json` 或 `summary.md`：时间、退出码、PASS/FAIL/BLOCKED 和结论；
- `stdout.log`、`stderr.log`：仅在命令产生有诊断价值的输出时保存；
- 其他文件：使用描述用途的稳定名称，禁止写入仓库根目录。

默认只保留为本地临时产物。确需长期留档时，由人工筛选最小证据集后移动到 `docs/shots/<YYYY-MM-DD>-<topic>/`，并添加 README，写明来源提交、执行命令、环境、结果、未测边界和为何值得版本控制。禁止把整个缓存目录、依赖、原始大日志或含敏感路径/设备信息的文件直接提交。

## 验证

```powershell
npm.cmd ci
npm.cmd run typecheck
npm.cmd run test:sites
npm.cmd run module-map:check
cargo test --workspace
```

只运行与改动匹配的必要门禁，但涉及设备、安全、IPC 或发布时必须覆盖相关全链。测试通过只证明离线逻辑；真实运动、相机和射线结论必须有本次明确授权下取得的设备证据。

交付报告列出改动文件、影响的控制链、执行命令及 `PASS/FAIL/BLOCKED`、未测边界和剩余风险。不在本文件记录版本进度、阶段计划或会话历史。
