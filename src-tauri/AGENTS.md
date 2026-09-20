# Tauri 桌面壳范围

Tauri 是轻量宿主，不是第二个领域引擎。它只拥有窗口、原生路径/对话框、sidecar 生命周期和 IPC 转发。

- `setup` 不得同步执行长耗时工作；sidecar 连接和监控必须离开 UI 启动路径。
- `engine_command` 与 `engine_snapshot` 是字符串 IPC 契约，改变时同步前端 adapter 和测试。
- `EngineClient` 必须校验 sidecar、协议版本、请求 ID 和响应序号；失败不得启动浏览器预览替代。
- 窗口关闭、sidecar 异常或 stdin EOF 必须触发可验证的安全停机路径。
- `gen/schemas/`、`icons/` 和 `binaries/` 是生成物，不手工编辑或添加逐文件注释。

## 目录修改与新增

本模块持久分区固定为 `src/`、`capabilities/`、`icons/`、`gen/` 和 `binaries/`；配置与构建入口留在 `src-tauri/` 顶层。

- `src/` 内新增 `.rs` 文件可以拆分窗口、路径、IPC 或 sidecar 的既有职责；新增子目录或新的后台服务/进程层必须先提交目录申请。
- 新增 Tauri plugin、权限类别或 capability 分区可能改变安全面，创建目录或清单前必须申请并列出新增权限。
- `gen/schemas/` 只能由 Tauri 工具更新；`icons/` 只能由图标生成流程更新；`binaries/` 只能由 sidecar 准备脚本写入。
- 不在 `src-tauri/` 下新增日志、崩溃转储、窗口截图、临时配置或测试输出目录。
- 新增平台专用实现如果仍属于薄壳职责，可以在审批后的结构内拆分；若包含设备协议、扫描状态或长期后台服务，必须留在/转交正确模块并先申请新的架构边界。

## 本模块生成物位置

| 产物 | 固定位置 |
|---|---|
| Rust/Tauri 编译缓存 | 仓库根 `target/` |
| 准备后的 sidecar | `src-tauri/binaries/` |
| Tauri schema | `src-tauri/gen/schemas/` |
| 桌面 IPC/路径合同过程测试 | `tmp/tests/desktop/<run-id>/` |
| sidecar、窗口和 IPC 调试日志 | `tmp/debug/desktop/<run-id>/` |
| 临时桌面运行会话 | `tmp/sessions/desktop/<run-id>/` |
| 未筛选窗口截图 | `tmp/screenshots/desktop/<run-id>/` |
| 经人工确认的 Windows 验收证据 | `docs/shots/<YYYY-MM-DD>-<desktop-topic>/` |

运行桌面测试前生成 `binaries/` 不需要目录申请，因为它是预先声明的生成物；新增另一种 sidecar 或另一套二进制目录则必须申请。

验证：`cargo test --workspace`、`npm.cmd run test:sites`；宣称桌面窗口正常还需真实 Windows 窗口证据。
