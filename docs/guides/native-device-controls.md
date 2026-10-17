# 原生设备控制维护

生产设备由 ct-engine 原生控制，当前能力和接口见 [ADR 0003](../decisions/0003-native-wpd-camera-and-device-control.md)、[相机矩阵](../architecture/native-camera-capability-matrix.md) 与 [执行流程](../architecture/execution-flows.md)。本指南不固定某次候选的源码哈希、测试数量或机器端口。

## 控制与数据

- 相机、Nano、Moxtek 各由单一 worker 持有通信；异步长操作不阻塞状态读取和优先 Stop/OFF。受理、设备完成和清理未知分开显示。
- 相机按明确身份与描述能力工作。D7100 扫描默认存卡，显式 Apply 主机目标才允许 RAM 传输，不回退 DigiCamControl 或模拟。存卡导入与精确清卡见 [ADR 0004](../decisions/0004-camera-sd-card-acquisition.md)。
- Live View 用受控会话的 named pipe 与有界内存，不保存实时像素文件；预览开启时拒绝捕获。独立单拍不增加扫描帧数，不自动开束。
- 普通 Nano 运动固定生产方向；HOME 有界反向退让由固件持有。到位、同 ticket 释放及 IDLE 才完成事务。位置必须有设备证据。
- 所有开束经过 Moxtek adapter，OFF 确认后至少等 2 秒；前空场整组连续出束，跨角度连续出束，Stop、故障及硬上限优先关束。拍后无后参考。
- 未完成任务跨重启保持手动锁；恢复核对冻结资料与当前证据，放弃需完整清理并保留全部文件。旧数据只读，算法改进写独立版本缓存。

## 维护和验证

离线检查不连接真实设备；真实连接、拍摄、运动、出束或固件上传按上级 AGENTS.md 核对当前授权范围。已有授权在原范围内持续有效。

按修改覆盖受影响的协议、解析、领域状态和生产消费方；不以模拟 transport 或旧版硬件证据宣布当前整机认证。实测报告分别记录构建身份、设备身份、实际动作、数据完整性及未测范围。

已有设备与清卡分项实测见 [验证索引](../validation/README.md)。负载侧角度精度、有效辐射成像与定量材料识别仍需独立证据；不能笼统称所有设备“尚未上机”，也不能笼统称全部通过。
