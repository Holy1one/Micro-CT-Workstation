# 目录功能定位

本表覆盖仓库中的活动目录及重要子目录。模块路径归属以 `module-map/modules.json` 为机器事实源；本文件解释人类应如何理解和使用这些目录。

## 智能体规则层级

仓库只保留四份会被智能体按目录继承的规则文件，避免重复占用上下文：

- `AGENTS.md`：全仓架构、安全不变量和验证要求。
- `crates/ct-engine/AGENTS.md`：Rust 领域内核与三类生产设备的共同约束。
- `src/AGENTS.md`：React、前端 engine、浏览器预览、3D 和平台服务的共同约束。
- `src-tauri/AGENTS.md`：桌面壳、sidecar 和 IPC 约束。

其他目录和子目录的功能说明使用 README，需要时再读取，不作为常驻规则上下文。

## 根目录

| 路径 | 功能定位 | 执行关系 / 约束 |
|---|---|---|
| `algorithm-plugins/` | 可选算法扩展的契约与未来实现入口 | 只能消费已导出的数据，不进入设备安全控制环 |
| `bridges/` | 外部工具或厂商软件桥接说明 | 当前 `digicam/` 记录 DigiCamControl 集成边界；生产调用实现在 Rust 相机模块 |
| `crates/` | Rust workspace 的领域 crate | 当前只有 `ct-engine`；新增 crate 必须保持 Tauri 壳轻量 |
| `docs/` | 当前架构、使用指南、历史方案和验收证据 | 当前事实必须与源码同步；历史内容必须明确标记 |
| `module-map/` | 模块归属与隐藏依赖的声明式事实源 | 由 `scripts/` 生成图谱并计算影响面 |
| `portable-release/` | 可重新生成的便携版二进制 | 不是源码或设计事实源，不在普通开发中手工修改 |
| `public/` | Vite 原样复制的静态资源 | `assets/` 只保存 UI 图片和 SVG，不包含运行逻辑 |
| `scripts/` | 构建、打包、图谱和验证脚本 | 只能编排确定性工具，不得访问真实设备 |
| `src/` | React/TypeScript 表现层和浏览器预览 | 生产设备命令只能通过 Tauri 进入 `ct-engine` |
| `src-tauri/` | Windows 桌面壳 | 只处理 OS 能力、sidecar 生命周期和 IPC 转发 |
| `tests/` | Node 合同测试与布局测试 | 离线运行，不接触真实设备 |
| `worker/` | Sites/静态部署兼容 worker | 只服务构建产物，不参与桌面设备控制 |

根级 `Cargo.toml`、`package.json`、`tsconfig.json` 和 `vite.config.mjs` 是工作区、前端和构建入口。`Cargo.lock` 与 `package-lock.json` 是生成的依赖锁文件，不手工注释或编辑。

## Rust 生产引擎

```text
crates/ct-engine/
├── Cargo.toml                 crate dependency declaration
└── src/
    ├── main.rs                JSONL sidecar process entry and EOF shutdown
    ├── lib.rs                 domain commands, safety state, snapshots, API DTOs
    ├── scan.rs                transactional projection coordinator
    └── devices/
        ├── mod.rs             device namespace and topology boundary
        ├── xray/
        │   ├── mod.rs         stable X-ray module surface
        │   └── moxtek.rs      current Moxtek PC-direct driver
        ├── camera/
        │   ├── mod.rs         stable camera module surface
        │   └── digicam_control.rs  current D7100 host-control driver
        └── turntable/
            ├── mod.rs         stable turntable module surface
            └── nano.rs        current RTS9060 Nano serial driver
```

三类设备目录可以独立分支深入开发，但公开 surface、扫描协调器消费方式和跨设备安全事务发生变化时，必须同步检查 `engine-core` 及所有强依赖边。设备模块不互相调用；跨设备顺序只存在于 `scan.rs` 和领域引擎。

## 前端

```text
src/
├── main.tsx                   React bootstrap only
├── App.tsx                    workstation panels and user interaction
├── tokens.css                 design-token source of truth
├── styles.css                 component layout and visual rules
├── canvas-layout.ts           fixed workstation scaling calculation
├── menuActions.ts             menu vocabulary and availability
├── engine/
│   ├── types.ts               shared EngineCommand/EngineSnapshot contract
│   ├── adapter.ts             runtime adapter selection
│   ├── tauriAdapter.ts        production Tauri invoke implementation
│   ├── useEngine.ts           React state bridge
│   ├── workstationAdapter.ts  browser-preview adapter
│   └── rts9060/               preview-only simulated workflow
│       ├── protocol.ts        Nano command parsing helpers
│       ├── transport.ts       in-memory firmware transport
│       ├── devices.ts         simulated X-ray and camera
│       └── workflow.ts        simulated scan workflow
├── platform/
│   └── desktopPaths.ts        desktop dialogs and path services
└── scene/
    ├── LiveSceneCanvas.tsx    Three.js canvas and camera controller
    ├── EquipmentScene.tsx     equipment geometry and beam visualization
    ├── scene-config.ts        optical-axis geometry constants
    ├── stage-surroundings.tsx environment geometry and textures
    ├── theme-three.ts         CSS-token to Three.js theme conversion
    ├── StaticSceneFallback.tsx non-WebGL fallback
    ├── useSceneFallback.ts    WebGL capability/context-loss hook
    └── types.ts               read-only scene view-model types
```

`src/engine/rts9060/` 是浏览器开发预览，不是活动 Nano 固件或生产设备驱动。它必须保持离线，并与 `types.ts` 的生产契约一致。

## Tauri 桌面壳

```text
src-tauri/
├── src/main.rs                Tauri commands, native paths, window lifecycle
├── src/engine_client.rs       sidecar verification and JSONL request client
├── build.rs                   build-time metadata and Windows resources
├── Cargo.toml                 shell dependencies
├── tauri.conf.json            bundle, window, and sidecar configuration
├── capabilities/              explicit Tauri permission declarations
├── icons/                     generated platform icon assets
├── gen/schemas/               generated Tauri schemas; do not hand-edit
└── binaries/                  generated sidecar copies; ignored by Git
```

## 工具、测试与文档

```text
scripts/
├── lib/module-map.mjs         shared glob, JSON, and file-walk helpers
├── gen-module-graph.mjs       declarations -> generated Markdown graph
├── verify-module-map.mjs      ownership and edge consistency gate
├── impact.mjs                 Git changes -> affected modules and gates
├── prepare-engine-sidecar.mjs Rust sidecar build/copy step
├── prepare-sites-build.mjs    Sites-compatible output assembly
└── process-logo.mjs           icon source processing

tests/
├── canvas-layout.test.mjs     fixed-canvas sizing invariants
├── desktop-path-contract.test.mjs native path and dialog contracts
├── menu-wiring-contract.test.mjs menu-to-command wiring
└── sites-worker.test.mjs      static worker/build output contract

docs/
├── architecture/              current architecture and generated graph
├── decisions/                 lasting architecture decisions and alternatives
├── shots/                     QA evidence and one-off probes, not product code
├── *.mermaid                  diagrams tied to current or historical docs
└── remaining *.md             integration guides, design notes, or marked plans
```

## 生成物与不可注释文件

下列文件不能安全内嵌英文注释：JSON（标准不允许注释）、依赖锁文件、PNG/ICO/ICNS、可执行文件和自动生成 schema。它们通过本目录图、相邻 README、字段名和生成脚本说明职责。手写 Rust、TypeScript、TSX、CSS、JavaScript 和构建脚本使用英文模块注释解释 purpose、inputs、outputs 和 safety boundary。
