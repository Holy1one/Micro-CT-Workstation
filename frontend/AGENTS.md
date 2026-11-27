# 前端约束

继承根级 AGENTS.md。src/ 是生产表现层；preview/ 是离线开发模拟。生产错误不触发模拟回退，scene/ 不发送设备命令。

index.html、tsconfig.json、vite.config.mjs 位于本目录；从仓库根运行 npm 脚本，显式使用此处的配置。Vite root 为 frontend/，依赖从根 node_modules/ 解析，输出与缓存仍归根 .cache/frontend/。

- 产品界面默认英文，可在 Tools → Interface Preferences 切换简体中文并保存在本机。所有产品文案通过 i18n.ts / zh-CN.ts 在呈现边界翻译，含菜单、提示、日志说明和错误建议；不得翻译命令、枚举值、用户草稿、路径、型号、错误码、诊断 JSON 或导出证据。英文模式继续由 engine/native-labels.ts 映射原生中文；中文模式保留原生中文。切换语言不得重建组件、重置任务/草稿或发送设备命令。
- 主界面铺满客户区四边；1920×1080 是设计基准，统一控件缩放，固定栏目不引入滚动。仅日志、图像条及浮层允许内部滚动。
- 自定义标题栏设计高度 39px；相机、设备视图、射线控制器共用上排，转台、日志、运行状态共用下排，上下排比例 660:313、间距 12px。1920×1080 基准下下排高度 313px，随客户区高度同步缩放；日志只占中央列，Camera/Turntable 常驻左列。
- Total projections 为 1–3600；Exposure 使用 ms、按原生档位与读回确认；Max X-ray time 在界面使用 mm:ss（上限 16:40 = 1000 秒整数，也接受十进制分钟），IPC、清单与冻结值一律使用整数秒。无效草稿不得沿用旧值启动。
- 三维姿态只跟随确认位置，未知、暂停、停止、失联和完成不继续预测运动；无 WebGL 时显示文字状态，不使用静态降级图片。
- 颜色和样式由 src/tokens.css、src/styles.css 维护；桌面能力由 src/platform/ 封装。静态资源从根 assets/ 暂存，测试输出进入 .cache/。
- 产品名 Micro-CT Workstation 属于品牌，标题栏、窗口标题和关于页在两种语言下都保留英文。所有文字共用双语字体栈与字号/字重令牌；英文和数字用 Inter，中文优先系统 PingFang SC、否则用内置 Noto Sans SC 可变字体，不为单个控件另选字体或模拟粗体。
- 自动图像处理与重构沿用现有操作入口；默认由引擎选取适用处理并显示结果、质量提示和失败原因。只有帮助操作员作出实际决定的信息才进入主操作流程。

验证：前端行为修改运行 npm.cmd run typecheck、npm.cmd run build 及相关 test:frontend 用例；涉及生产契约时覆盖对应桌面及 Rust 测试。纯文档按根规则检查。
