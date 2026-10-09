# 0.9.13 相机预检与高级设置

## 范围

按本次用户要求删除运行时多重曝光准入、人工确认命令、会话记录、相机健康字段、前端类型、预览分支与提示。相机原生属性目录中没有为此新增或试发任何操作。当前采集 profile 升为 6，固定策略 hash 版本为 3；新 profile 不包含退役字段。旧 profile 1–5 保留只读文件及原 hash 校验，不追加投影。profile 5 的旧字段仅用于原格式解码和 hash 重现，没有状态裁决或人工确认行为。

长曝光降噪 OFF、包围 OFF、ADL OFF4、HDR OFF/未广告时 NEF 互斥、固定 WB、M、Auto ISO OFF、MF、单个 RAM NEF、会话及事件关联、完整文件/hash、Live View 拍摄禁令、停止清理、维护锁继续执行。

高级设置正文采用有界 flex 布局，左右两栏共同填满标题和固定底部按钮之间的高度。右栏删除 560px/60vh 高度上限；左栏删除顶端自适应高度限制，两栏内部可滚动。预检相机错误按英文参数标签、实际值、要求和操作建议显示；原始引擎文本保留在日志与 Copy details。无错误码的已失败操作不再附加控制服务未响应的猜测。

## 离线验证

| 结果 | 检查 |
|---|---|
| PASS | 修改前回归复现：完整相机参数读回仅因旧准入项被阻塞；修改后同一要求通过，并继续拒绝长曝光降噪 ON |
| PASS | profile 6 跨会话同参数匹配，参数漂移拒绝；修改前保存的两种 profile 5 记录保持原 hash、可往返解码、拒绝续扫、篡改 hash 元数据拒绝 |
| PASS | `cargo test --manifest-path backend/Cargo.toml --workspace`：295 引擎库 + 2 引擎进程 + 54 桌面 Rust 项，7 项原有忽略 |
| PASS | `npm.cmd run test:frontend`：171 项，包括相机参数问题展示、原始错误保留、真正 transport 错误仍分类为 transport |
| PASS | `npm.cmd run test:desktop`：27 项；`npm.cmd run typecheck`、`npm.cmd run build`、`npm.cmd run module-map:check` |
| PASS | 实际 React/App DOM 浏览器检查：1366×768、1920×1080、3840×2160 下 Exposure/Presets/Diagnostics 左右栏等高，底端使用正文可用高度，底部按钮保持可见，无正文溢出、无浏览器异常 |
| PASS | 实际 MSI 21 对话框/200 控件、字体/目录选择及提升标志检查；提取的 app/engine 与同次构建逐字节核对；发行签名和内部更新容器验证 |
| PASS | MSI 提取应用使用隔离数据正常启动，控制服务 Ready，无启动失败通知；app PID 27472 与其 engine PID 60092 均正常退出 0 |

过程日志在 [相机验证目录](../../.cache/tests/camera/20261008-policy-cleanup/)，浏览器结果与截图在 [布局验证目录](../../.cache/tests/frontend/20261008-policy-cleanup/)。原失败和修复期间遇到的过时测试期望已分别修正，最终门禁均通过。

## 包身份

- 交付：[Windows x64 0.9.13 MSI](../../release/micro-ct-workstation_0.9.13_windows_x86_64.msi)，17,007,031 bytes。
- MSI SHA-256：`2e9514a80ebc9286a82388979bd437a7643656e7e11d969608d56ca0a852a5c9`。
- 包内应用 SHA-256：`e45d71dd78f41ea7e2962c5999055b7f7d8b5bb34d67d43fbabd8ed877cae1b0`。
- 包内引擎 SHA-256：`11308abdce441e9e51984bdf66f227dc3ddc25c2498ac1a74b5bea647abfdd78`。
- 同次来源、签名、原包、许可证及身份文件保存在 [.cache/tests/release/0.9.13](../../.cache/tests/release/0.9.13/)。release/ 仅保留当前 MSI，历史安装包退出交付目录。

## 边界

仅离线验证。本次没有打开真实设备或串口，没有安装、Repair 或发布 GitHub Release。删除准入项不生成“机器读回 OFF”的证据；C101/C102 与实际曝光次数的核心假设、真实联动和成像精度仍需要实机验证。已安装版本和已有扫描数据未被本次验收修改。
