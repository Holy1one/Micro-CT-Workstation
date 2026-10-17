# D7100 原生事件与转台台架关键证据（2026-10-11）

历史实测摘要。保留第 7/9/10 节编号，供源码与回归用例追溯事件判据；过期任务清单、当时设备状态和后续授权请求已移除。当前存卡、自动导入及精确清卡见 [ADR 0004](../decisions/0004-camera-sd-card-acquisition.md)。

本轮为指定 D7100 与 Nano 的相机/转台台架，未连接射线；不证明负载侧角度准确性或整机辐射成像。原始完整记录另保存在 [历史摘要](history.md) 所列保护快照中。

## 7 主机 RAM 早期失败与未关联事件

早期捕获错误地等待完成/曝光结束事件，截止时未取得 NEF，操作为 cleanup_unconfirmed，相机会话隔离，退出码 1。诊断收到 0x4006 PropertyChanged、参数 0xD1B4；该属性通知不构成曝光结束或图像提交证据，不能据其计时推定拍摄成功。

另一次被中断操作留下 0xC101、参数 0x0B000001，后续连接空闲时出现并按未关联对象退役会话。即使后续引入可选完成排空窗口，该对象事件也不能获得捕获信用。0x90C7 是 GetEvent 轮询操作，不能称为拍摄操作码。

这是当时的 FAIL；后续判据修复与复测见下文，不代表当前 RAM 路径不可用，也不是重拍或清理相机缓冲的授权。

## 8 HOME 与规定方向运动

明确授权后，HOME 用时 4796 ms；1° 两次为 1844/1868 ms，2° 为 2407 ms，5° 为 4001 ms。生产方向、参考建立、匹配 CAPTURE_HOLD 及释放均取得控制器证据。

这些总耗时包含串口与引擎往返，不能分离证明“末脉冲下降沿后精确等待 150 ms”，也不证明负载侧实际角度。原始记录：.cache/tests/hardware/20261011-home-settle/。

## 9 RAM 对象提交成功与迟到 C102

按与当前操作关联的 C101 接受图像可用、读取对象并校验落盘后，得到真实 NEF：27,308,012 字节，SHA-256 c33242ab34a9d3c8e314c48f77e15ac37c6569854965c24d89c80cda5f724bc8。操作 completed / confirmed；文件提交前 captureCompleteCount=0、exposureEndedObserved=false，清理确认后退出 0。

文件登记后实际又到达 0xC102、参数 0，旧空闲守卫以 unassociated capture or device event while idle 退役会话。因而只能说该事件在提交前可缺失或迟到，不能说真实机身“从不发送 C102”。完整文件成功与后续会话可用性是两个验收项。

原始记录：.cache/tests/hardware/20261011-capture-retest/。该次测试后切回存卡并读回确认的证据在 .cache/tests/hardware/20261011-restore-card-mode/；不作为现在机身状态。

## 10 有界迟到完成吸收与连续两张复测

提交后增加 5 秒排空窗口，只接受同一已提交操作、同一会话的一次可选 C102。仅写诊断，不产生 ExposureEnded、提交信用、传输或新拍摄；窗口外、重复完成及任何未关联 C101 继续失效安全拒绝。

实际连续两张、中间不重连，操作用时 10,164/10,117 ms；文件分别为 21,948,627/21,554,548 字节。会话 connected=true、isolated=false、cleanupConfirmed=true，lateCompletionAbsorbedCount=1，随后明确切回存卡并读回，退出 0。

原始记录：.cache/tests/hardware/20261011-capture-twice/。该有限复测支持所列可用性修复，不认证所有机身模式、USB 故障、当前构建全工况或 CT 精度。参考语义对照见 [D7100 取证记录](d7100-capture-semantics-digicamcontrol-2026-10-11.md)。
