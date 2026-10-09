# Micro-CT-App 工作约定

## 共同必读

所有任务先读本文件。当前用户要求优先；源码、配置和测试高于说明文档。沿相关调用和数据流完成最小完整改动，保留用户已有修改。真实设备动作遵循上级工作区的 AGENTS.md。

## 按任务读取

| 任务 | 补充读取 |
|---|---|
| 目录整理、新增或移动文件 | docs/architecture/directory-map.md、modules.json、directory-policy.json |
| 前端修改 | docs/architecture/frontend-functional-spec.md、frontend-error-codes.md、frontend/AGENTS.md |
| 后端修改 | docs/architecture/execution-flows.md、相应设备 README、backend/ct-engine/AGENTS.md、modules.json 门禁 |
| IPC、协议、快照字段修改 | docs/architecture/edges.json、全部相关生产消费方及测试 |
| 桌面能力修改 | backend/src-tauri/AGENTS.md、对应平台接口与桌面测试 |
| 发布修改 | docs/guides/desktop-update-distribution.md、tools/release/、版本账本和公钥 |
| 整体架构判断 | docs/architecture/README.md、directory-map.md、modules.json、edges.json |

表中的 JSON 均位于 docs/architecture/。详细说明在相应文档维护；规则文件不保存进度、会话流水或一次性任务提示词。

## 设计边界

- backend/ct-engine 是生产设备、任务、安全状态、扫描事务和进度的唯一权威。
- frontend/src 只展示完整快照和发送领域命令；backend/src-tauri 只管理 OS 能力、引擎生命周期和 IPC；三维场景只读展示。
- frontend/preview 仅用于离线浏览器开发，不接触真实设备。生产 IPC 失败不得回退到模拟成功。
- 相机、射线和转台设备模块互不调用；跨设备扫描顺序由扫描协调器持有，独立 HOME 安全裁决由领域入口持有。
- 设定值不得冒充实测值；UNKNOWN、超时、失联和关束未确认不得显示或处理为正常、安全完成。
- 相机和转台可独立连接、自检；正式扫描首期采用 D7100 主机 RAM NEF。其他设备能力以身份、实际描述及可验证语义为依据，不试发未知厂商操作。
- Live View 开启时拒绝拍摄；独立拍摄、Live View、手动旋转不自动开束。普通转台运动只沿既有生产方向，HOME 保留固件内部有界反向退让。
- HOME 在射线已连接时先关束读回确认，结束不自动开束；台架物理隔离确认不能覆盖曾连接射线后的 UNKNOWN。
- 扫描、暂停、清理、重构和持久未完成任务占用期间保持手动控制锁；Stop 和状态读取始终可用。放弃任务必须完成清理并保留全部文件。
- 所有开束入口经过 Moxtek adapter；确认关束后至少 2 秒才允许再次开束，重连不得绕过。正常投影连续出束，末张及暂停依据关联 ExposureEnded 关束；完整文件/hash 和匹配 CAPTURE_DONE 仍是提交条件。
- 达到 maxXraySec 立即关束确认并按既定 300 秒关束间隔处理未开始投影；空场无法在单组上限内完成时关束报错，不短间隔循环开束。Stop、联锁、掉线和未知状态优先立即关束。
- 原始 NEF 只读；旧 schema 2 及旧采集 profile 保留读取、重构和合法缓存能力，不伪造证据或改写旧数据。取消受理与硬件清理完成分别表达，迟到事件不得恢复动作或提交旧结果。
- 软件安全不能替代屏蔽、门锁、急停、安全继电器和现场规程。

## 目录和变更纪律

目录职责见 directory-map.md。结构化 modules.json、edges.json 和 directory-policy.json 是自动检查依据，必须与新增、移动和接口变化同步更新。新的顶层或领域职责目录需要人工明确批准；在已有职责内拆分实现也须更新相邻说明、路径归属和引用。不得预建无实现目录。

生产源码分别位于 frontend/、backend/、firmware/；backend/ 包含 ct-engine/ 领域引擎、src-tauri/ 桌面壳和唯一 Cargo workspace 清单及锁文件。维护资源集中 assets/；工具在 tools/；外部测试在 tests/；详细资料在 docs/。禁止把临时输出混入这些维护目录。

## 生成物和 Git

- .cache/cargo/：Rust 编译；.cache/frontend/：Vite 与网页输出。
- .cargo/config.toml 是全仓共享输出配置；从根运行 Cargo 时指定 --manifest-path backend/Cargo.toml，npm 入口统一处理桌面路径。
- .cache/tests/<module>/<run-id>/：命令、结果和有用日志；.cache/screenshots/、debug/、sessions/：临时输出。
- backend/src-tauri/binaries/ 和 gen/ 是明确的 Tauri 生成位置，均忽略 Git。
- release/ 只保存最新 Windows MSI 安装包；构建来源、签名、许可证和身份信息归 .cache/tests/release/<version>/，许可证同时进入 MSI。历史验收记录保留可核验来源，退出交付目录。
- tests/ 与纯测试工具本机保留且不跟踪；构建、发布、模块检查工具、JSON 约束、锁文件、版本账本和公钥继续跟踪。
- 删除缓存前区分可再生成文件、原始扫描数据与唯一有效验收证据；原始数据和现有生产成果必须保留。

## 验证要求

按 modules.json 运行影响模块门禁。基本检查：npm.cmd run module-map:check、npm.cmd run typecheck、npm.cmd run build、cargo test --manifest-path backend/Cargo.toml --workspace。本机完整验证可用 npm.cmd run verify；外部测试套件仅在本机存在时运行。

设备、安全、IPC 和发布变更覆盖相应控制链。目录迁移核对源码内容、资源哈希、依赖引用、桌面构建和正常启动/退出；运行时不连接真实设备。交付列出 PASS、FAIL、BLOCKED、命令及未测边界。无本次硬件证据时明确写“仅离线验证”。
