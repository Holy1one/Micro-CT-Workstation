# 前端模块范围与约束

`src/` 是 React/TypeScript 表现层，同时包含严格隔离的浏览器开发预览。目录结构和文件职责见 `src/README.md`。

- React 只消费完整 `EngineSnapshot` 并发送 `EngineCommand`，不得自行推进生产扫描状态。
- Tauri runtime 中 IPC 失败必须显式报错，禁止回退到 `WorkstationAdapter`。
- `engine/types.ts` 是前端公共契约；改变字段或命令时同步 Rust DTO、生产/预览 adapter 与合同测试。
- `engine/rts9060/` 仅为离线浏览器预览，不得访问串口、相机、射线源或被当作硬件证据。
- `scene/` 只读展示，不得调用 Tauri、import EngineAdapter 或发送设备命令。
- 颜色使用 `tokens.css`，平台路径和对话框通过 `platform/` 封装。

## 目录修改与新增

前端持久功能分区固定为 `engine/`、`scene/`、`platform/`，以及 `src/` 顶层的应用组合、样式和菜单文件。

- 在现有分区内新增文件或实现子目录，仅允许用于拆分该分区已经拥有的职责，并同步 `src/README.md`、模块图谱和 import。
- 在 `src/` 下新增直接子目录通常代表新的前端功能模块，必须先按根规则提交目录申请。
- 在 `engine/` 下新增生产 adapter、transport 或新的预览体系，属于控制边界变化，必须申请；不得用新目录规避 Tauri/preview 隔离。
- 在 `scene/` 下新增纯几何/材质子目录可视为同一展示职责，但如果引入设备命令、校准数据、文件格式或独立渲染进程，必须申请。
- 自动化测试源码继续放在根 `tests/` 或现有源码文件的测试代码中；新建 `src/**/__tests__/`、快照、fixture 目录前必须申请并说明为何根测试模块不足。
- `public/assets/` 只保存产品需要的静态资源；调试截图、对比图和临时模型不得放入其中。

## 本模块生成物位置

| 产物 | 固定位置 |
|---|---|
| npm 依赖 | `node_modules/` |
| Vite 缓存 | `.vite/` |
| 前端和 Sites 构建输出 | `dist/` |
| 前端合同/布局过程测试 | `tmp/tests/frontend/<run-id>/` |
| 浏览器预览扫描会话 | `tmp/sessions/preview/<run-id>/` |
| React 状态、DOM、WebGL 调试输出 | `tmp/debug/frontend/<run-id>/` 或 `tmp/debug/scene/<run-id>/` |
| 未筛选页面/3D 截图 | `tmp/screenshots/<frontend|scene>/<run-id>/` |
| 覆盖率报告 | `coverage/` |
| 经人工确认的布局/3D 长期证据 | `docs/shots/<YYYY-MM-DD>-<ui-or-scene-topic>/` |

简单 UI 过程测试产生的 DOM JSON、控制台日志和截图必须进入同一个 run-id 目录，禁止散落在 `src/`、`public/assets/` 或仓库根。

验证：运行 `npm.cmd run typecheck` 与 `npm.cmd run test:sites`；涉及 Rust DTO 或 IPC 时再运行 `cargo test --workspace`。
