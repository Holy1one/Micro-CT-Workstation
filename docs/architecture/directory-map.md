# 目录职责

| 目录 | 允许内容 | 边界 |
|---|---|---|
| frontend/src | React 组件、样式、engine 契约、platform 服务、scene 展示 | 不持有生产扫描状态、不直接连接设备 |
| frontend/preview | workstationAdapter、rts9060 模拟、hosting 静态服务 | 严格离线，不作为设备证据 |
| backend/ct-engine | Rust 领域内核、设备模块、扫描和重构、crate 单元测试 | 唯一生产状态及安全权威 |
| backend/src-tauri | OS 能力、IPC、生命周期、更新 | 薄桌面壳 |
| firmware/rts9060_nano | 原始固件候选、身份和构建配置、其固定 include 所需 stub | .ino 保持核验身份，构建不上传 |
| assets | 透明 Logo 母版、字体、必要打包图标、来源说明 | 一处维护，构建只收集轻量运行资源；历史截图归 .cache/screenshots |
| tests | frontend、desktop、firmware、fixtures、helpers | 本机保留，不进入 Git；私有 Rust 单测仍在 crate 内 |
| tools | build、release、verify、lib | 构建/发布/约束工具继续跟踪，不连接设备 |
| docs/architecture | 当前架构、接口、流程、目录说明及三份约束 JSON | JSON 是自动检查依据 |
| docs/guides | 当前开发、设备集成、发布和维护说明 | 不混入实施流水 |
| docs/decisions | 有效长期决策 | 不创建未实现的后端占位目录 |
| docs/validation | 最小验收摘要 | 原始本机过程证据归 .cache/tests |
| .cache | cargo、frontend、tests、screenshots、debug、sessions | 可再生成产物；清理先保护真实数据和唯一证据 |
| release | 只保存最新 Windows MSI | 全部为本地生成物；签名、身份、来源和许可证归 .cache/tests/release/<version> |

frontend/index.html、tsconfig.json、vite.config.mjs 集中维护前端启动、类型与构建配置；npm 脚本显式指定配置路径。前端 root 为 frontend/，构建和暂存资源仍在 .cache/frontend/。

Rust workspace 清单与唯一锁文件位于 backend/{Cargo.toml,Cargo.lock}，两个成员为 ct-engine/ 和 src-tauri/。从仓库根直接执行 Cargo 时指定 --manifest-path backend/Cargo.toml；在 backend/ 内可直接使用 cargo。Tauri 的 npm 开发、安装包与发行入口统一经过 tools/build/run-tauri.mjs，显式指定桌面 crate 与 npm 钩子的工作目录。

根级保留 7 个入口文件：package.json、package-lock.json、.npmrc（npm）；.gitignore、.gitattributes（Git）；AGENTS.md、README.md（全仓规则和导航）。npm 依赖保留 node_modules/；.cargo/config.toml 是全仓共享编译输出约束，从根、backend 或 crate 内启动 Cargo 都使用根 .cache/cargo/。Tauri 固定生成例外为 backend/src-tauri/gen/、binaries/。不在根级保留第二份 Cargo 清单、锁文件或桌面源码副本。

## 结构化约束

- modules.json：文件所有者、模块职责、公开 surface 和验证门禁。
- edges.json：IPC、协议与编译器不可见的语义依赖。
- directory-policy.json：重要边界下允许存在的直接子目录。
- module-graph.md：由 tools/verify/gen-module-graph.mjs 生成，禁止手工维护。

新增、移动或删除维护路径时，同步 JSON、引用和相邻说明，并运行 npm.cmd run module-map:check。当前目标所需的现有职责内整理可直接执行；新增目标之外的领域职责或系统边界变化先明确范围。
