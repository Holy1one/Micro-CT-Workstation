# D7100 曝光结束信号台架证据（2026-10-12）

这是指定设备与工况的历史测量，不是当前操作清单。后续已实施的早释放契约见 [ADR 0004](../decisions/0004-camera-sd-card-acquisition.md)，实际安装版联动证据见 [联动记录](installed-camera-validation-2026-10-13.md)。

## 测量范围

Nikon D7100 8322769，NEF 14 位无损、M、ISO 100、物理 MF、单张存卡。该轮仅相机台架，未连接射线和转台。31 张初始测量使用 0.25 / 1 / 4 秒档，随后 14 张值级复核使用 1 秒 8 张、4 秒 6 张。

| 曝光 | D1A4 通知距触发返回 | ObjectAdded + CaptureComplete |
|---|---|---|
| 0.25 s | 588–593 ms，10/10 | 2852–3462 ms |
| 1 s | 1336–1345 ms，10/10 | 3664–4425 ms |
| 4 s | 4339–4348 ms，10/10 | 6925–7343 ms |

14/14 值级复核观测到 D1A4 UINT32 的 bit15 先置位、后清零，清零约在曝光时长加 0.34 秒，早于存卡完成。120 秒空闲窗口的 600 次采样中 D1A4 全为 0，未观察到对应假转换；这只限制已测窗口，不能证明所有工况无假阳性。单张 DeviceReady 首次 OK 约在 1415 ms，完整对象/完成约在 3914/3974 ms。

bit15 与曝光过程的对应属于受测数据支持的推断；设备事件没有本项目 operation ID，生产关联依赖同一 owner/session 且最多一张未完成拍摄、先置位再清零等约束。早事件不代替存卡回执和文件 hash，也不能据该表推广所有相机或参数。

## 原始证据

相对 .cache/tests/camera/：

- 初始档位：20261012-143432-card-interval-serial/、20261012-143530-card-interval-serial/、20261012-143631-card-interval-serial/。
- DeviceReady：20261012-143808-card-interval-monitor/。
- 值级复核：20261012-164450-card-interval-serial/、20261012-164543-card-interval-serial/。
- 空闲采样：20261012-171140-card-interval-idlesample/。
- 精确清理台账：20261012-signal-ledger/ledger.json、20261012-signal2-ledger/ledger.json。第一批 31 张与第二批 14 张均按存储、名称、长度、拍摄时间核对并确认消失，其他照片保留。

旧报告中“尚不可生产放行”、默认关闭的诊断环境开关和未执行的 C′/L1 任务描述已退出现行说明；完整历史原文保存在 [历史摘要](history.md) 所列快照中。当前实现与当前验收仍各自核对，不能用后续实现改写本次实验当时的范围。
