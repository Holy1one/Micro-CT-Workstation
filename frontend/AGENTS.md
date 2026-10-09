# 前端约束

继承根级 AGENTS.md。src/ 是生产表现层；preview/ 是离线开发模拟。生产错误不触发模拟回退，scene/ 不发送设备命令。

index.html、tsconfig.json、vite.config.mjs 位于本目录；从仓库根运行 npm 脚本，显式使用此处的配置。Vite root 为 frontend/，依赖从根 node_modules/ 解析，输出与缓存仍归根 .cache/frontend/。

- 产品界面全部英文，包含日志、错误详情和只读诊断。通过 engine/native-labels.ts 和 deviceControls.ts 映射界面文案；未映射诊断和非英文标识以可还原的 Unicode 转义显示。引擎快照、错误码、原始值与导出记录保持完整。
- 主界面铺满客户区四边；1920×1080 是设计基准，统一控件缩放，固定栏目不引入滚动。仅日志、图像条及浮层允许内部滚动。
- 自定义标题栏设计高度 39px；日志与运行状态底栏设计高度 209.375px，上方间距 12px，日志只占中央列，Camera/Turntable 常驻左列。
- Total projections 为 1–3600；Exposure 使用 ms、按原生档位与读回确认；Max X-ray time 使用 min，IPC 使用秒。无效草稿不得沿用旧值启动。
- 三维姿态只跟随确认位置，未知、暂停、停止、失联和完成不继续预测运动；无 WebGL 时显示文字状态，不使用静态降级图片。
- 颜色和样式由 src/tokens.css、src/styles.css 维护；桌面能力由 src/platform/ 封装。静态资源从根 assets/ 暂存，测试输出进入 .cache/。

验证：npm.cmd run typecheck、npm.cmd run build；本机套件使用 npm.cmd run test:frontend。涉及生产契约时再运行桌面及 Rust 测试。
