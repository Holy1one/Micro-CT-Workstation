# ct-engine 范围与约束

本 crate 是生产领域内核和唯一安全权威。修改时先读 `docs/architecture/README.md`；目录职责和设备实现说明见本 crate 及 `src/devices/` 下的 README。

- `src/lib.rs` 负责命令、安全状态、完整快照和 JSONL DTO；不要把厂商协议细节继续堆入此文件。
- `src/scan.rs` 只负责编排投影事务；设备 I/O 细节留在 `src/devices/`。
- `src/main.rs` 只负责 JSONL 循环和进程退出关束，不承载业务状态。
- `src/devices/xray/` 只处理 Moxtek I/O、实测回读和 fail-closed 关束；设定值不得冒充实测值。
- `src/devices/camera/` 只处理 D7100/DigiCamControl 与主机文件确认；不得静默回退到存储卡。
- `src/devices/turntable/` 只处理 Nano 协议、运动确认、警告和 STOP/E-STOP；协议不得靠猜测改变。
- 三类设备模块不得互相调用；跨设备顺序只存在于 `src/scan.rs`。
- 不得让 Tauri、React 或设备 worker 推进第二份生产扫描状态。
- 任何错误、超时、掉线或未知设备状态都 fail closed；关束未确认不得报告安全完成。
- 新功能优先放入职责明确的新模块；现有超大文件只做与任务直接相关的最小改动，并逐步抽离稳定边界。

## 目录修改与新增

本 crate 的持久功能分区固定为：顶层领域文件 `src/{main.rs,lib.rs,scan.rs}`，以及 `src/devices/{xray,camera,turntable}` 三类设备目录。

- 在现有设备目录中新增 `.rs` 文件，用于拆分同一设备的 transport、protocol、解析或测试帮助代码，属于现有职责；同步更新该目录 `mod.rs` 和 `src/devices/README.md`。
- 在 `src/` 下新增目录代表新的领域子系统；在 `src/devices/` 下新增目录代表新的设备类型或新的统一控制 backend。两者都必须先按根 `AGENTS.md` 提交目录申请。
- 把 PC 直控和未来 Arduino 主控拆成新目录属于架构边界变化，即使名称已经在文档中规划，也必须重新申请并说明活动固件来源、协议版本和状态所有权。
- 不在源码旁创建 `fixtures/`、`captures/`、`logs/`、`output/` 或临时串口转储目录。若确需新增受版本控制的测试 fixture 目录，必须先申请，说明数据来源、体积、脱敏和更新方式。
- 移动现有设备目录或改变公开 `mod.rs` surface 时，同步模块图谱、扫描协调器、README 和全部调用方。

## 本模块生成物位置

| 产物 | 固定位置 |
|---|---|
| Cargo 编译、单测和增量缓存 | 仓库根 `target/` |
| ct-engine 过程测试输出 | `tmp/tests/ct-engine/<run-id>/` |
| X 射线模拟/解析测试输出 | `tmp/tests/xray/<run-id>/` |
| 相机模拟/文件确认测试输出 | `tmp/tests/camera/<run-id>/` |
| 转台协议/运动模拟测试输出 | `tmp/tests/turntable/<run-id>/` |
| 串口帧、协议 trace、故障诊断 | `tmp/debug/<xray|camera|turntable>/<run-id>/` |
| 扫描模拟会话和临时 manifest | `tmp/sessions/ct-engine/<run-id>/` |
| 值得长期保留的离线或硬件验收证据 | `docs/shots/<YYYY-MM-DD>-<device-or-scan-topic>/` |

测试不得默认写入真实扫描保存目录。真实设备调试仍需本次明确授权；授权不改变输出位置、脱敏和证据筛选要求。

验证：至少运行 `cargo test -p ct-engine`；涉及 JSONL、Tauri 或发布时运行 `cargo test --workspace` 和对应前端合同测试。
