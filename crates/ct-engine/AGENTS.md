# ct-engine 范围与约束

本 crate 是生产领域内核和唯一安全权威。修改时先读 `docs/architecture/README.md`；目录职责和设备实现说明见本 crate 及 `src/devices/` 下的 README。

- `src/lib.rs` 负责命令、安全状态、完整快照和 JSONL DTO；不要把厂商协议细节继续堆入此文件。
- `src/scan.rs` 只负责编排投影事务；设备 I/O 细节留在 `src/devices/`。
- `src/main.rs` 只负责 JSONL 循环和进程退出关束，不承载业务状态。
- `src/devices/xray/` 只处理 Moxtek I/O、实测回读和 fail-closed 关束；设定值不得冒充实测值。
- `src/devices/camera/` 负责原生传输、身份和能力发现、参数、Live View、拍摄事件与主机文件确认；不得静默回退到存储卡、旧外部相机工具或模拟。实际迁移状态以源码为准，技术选择见 `docs/decisions/0003-native-wpd-camera-and-device-control.md`。
- `src/devices/turntable/` 只处理 Nano 协议、运动确认、警告和原生 STOP；协议不得靠猜测改变。硬件急停保护独立于界面普通 Stop。
- 三类设备模块不得互相调用。扫描顺序由扫描协调器持有；独立 HOME 等跨设备安全裁决由领域入口持有，不把这类安全规则移入 UI 或设备模块。
- 不得让 Tauri、React 或设备 worker 推进第二份生产扫描状态。
- 任何错误、超时、掉线或未知设备状态都 fail closed；关束未确认不得报告安全完成。
- Moxtek 设备层必须在每次已确认关束后阻止 2 秒内再次开束，手动控制、扫描和重连共用此门控。扫描前空场在整组拍摄期间持续出束，最后一张文件确认后才正常关束；不得逐张循环开关。扫描结束后不采集后空场或后暗场。投影之间持续出束，正常末张投影及暂停按本次会话和操作关联的可靠 `ExposureEnded` 关束，继续完成完整文件/哈希确认、同运动 ID 的 `CAPTURE_DONE` 和提交后才报告 Completed/Paused。曝光结束不得由计时、临时文件或工具退出推定。Stop、故障、失联、未知和连续出束硬上限优先立即关束；达到 `maxXraySec` 后保持既定 300 秒关束休息策略。空场无法在单个连续出束上限内完成时关束报错，不自动重新开束。
- 相机、转台独立连接并校验自身，不依赖完整扫描参数或其他设备。独立单拍与扫描落盘和事务隔离，不自动开束；Live View 开启期间必须拒绝捕获，不自动停止后拍摄再恢复。正式扫描首期仅接受 D7100 主机 RAM NEF，其他型号只开放实际描述且安全语义有证据的能力，不向未知设备发送 Nikon 私有操作。
- 相机 COM 对象由专用线程创建、使用和释放，只暴露 Send 消息代理，不为真实 COM 对象添加 `unsafe Send`。Moxtek 由单一常驻 worker 持有连接、last_off、设定和 health；Nano 复用单一串口 worker。长操作不得阻塞引擎监测、状态读取或 Stop/OFF；安全监测不依赖 UI snapshot 请求。
- 全局 Stop 覆盖扫描、手动出束、HOME/MOVE、拍摄与 Live View；并行发起相机取消、Nano STOP 和 Moxtek OFF，OFF 不等待相机 COM/传输或运动。区分命令受理、取消已请求和硬件完成；未确认清理的会话必须隔离，关束失败为 UNKNOWN 并锁故障。旧代次和迟到事件不得继续提交、开束、恢复 Live View 或解锁。
- 普通运动只沿既有生产方向，保持协议负符号并在领域入口及 adapter 拒绝逆向；HOME 保留固件内部有界反向退让例外。运动完成必须有到位、匹配 ticket 的 `CAPTURE_DONE` 及 IDLE 证据，不能只把 ACK 当完成。
- 自主 HOME 在连接射线时先关束读回确认，完成后不自动开束。仅转台台架模式要求物理隔离确认，仍显示射线 UNKNOWN/未监测并禁开束；连接射线后重新验证。曾连接而 OFF 未知不得用台架确认覆盖。
- 扫描、暂停、收尾、停止清理、重构占用期间锁定手动控制，Stop 和状态读取始终可用。持久未完成任务锁跨重启和 taskId 切换生效，锁定参数、对焦和手动运动；扫描恢复必要的受控 HOME 是例外。放弃任务先完成 worker 清理及必要 OFF 确认，再持久标记和解锁，保留全部文件，不置 Completed、不允许原目录覆盖。
- schema 3 冻结设备身份/backend、采集参数、实际读回及规范化 profile hash；恢复比较实际内容。旧 schema 2 仅可读取、离线重构与合法缓存复用，新后端拒绝追加投影；不重写旧 manifest/preprocessing/cache，不改变数值重构算法或原始 NEF。
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
