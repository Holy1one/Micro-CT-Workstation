# 桌面壳约束

继承根级 AGENTS.md。只拥有原生窗口/路径、进程生命周期、IPC 和更新传输；设备和扫描状态属于 ct-engine。

- setup 不同步等待长操作；IPC 校验版本、请求与会话，不回退到模拟。
- 退出与通信中断保留原有安全清理；更新必须取得本会话/操作关联的领域清理报告。
- build.rs 记录引擎 SHA-256；binaries/ 的打包副本、测试副本和安装目录中的 ct-engine 必须来自同一次当前源码构建。生产启动只校验并运行安装目录中的引擎，不向用户缓存释放可执行文件。
- gen/、binaries/ 为明确生成例外，其他日志、截图与测试输出进入 .cache/。
- 新权限、插件、后台进程或领域职责需要明确批准。

验证：cargo test --manifest-path backend/Cargo.toml --workspace；本机桌面契约测试使用 npm.cmd run test:desktop。桌面交付必须验证本机构建 EXE 的启动和正常退出。
