# PC 直控与 Arduino 主控双线架构

## 当前事实

当前生产拓扑是 `pc-direct`：Windows PC 分别连接 Moxtek 射线源、Nikon D7100 和 RTS9060 Nano。三个驱动位于 `ct-engine/src/devices/{xray,camera,turntable}`，扫描协调器直接组合它们。

当前没有活动 Arduino 中心固件源码，也没有经过确认的统一控制器协议。因此本仓库不会创建假实现、猜测命令或宣称第二条生产链路可用。

## 稳定边界

未来切换通信拓扑时，下列部分保持稳定：

- React 的 `EngineCommand` / `EngineSnapshot` 语义；
- Tauri 的 `engine_command` / `engine_snapshot` IPC；
- versioned JSONL envelope；
- `ct-engine` 对安全门控、状态和扫描事务的最终权威；
- 投影提交条件和 fail-closed 规则；
- 图像必须在主机侧确认的要求，除非未来硬件能力与数据链经过正式决策变更。

变化只发生在设备通信实现与能力协商层。

## 目标形态

```text
                       ┌─ pc-direct backend (current)
ct-engine domain ports ┤    Moxtek + D7100 + Nano
                       │
                       └─ controller-managed backend (future)
                            versioned controller protocol
                            capability discovery
                            Arduino-side device fan-out
```

两条线是可选择的 backend，不是两个领域内核。禁止在 Arduino 和 PC 中各维护一份可独立推进的扫描状态。若未来控制器需要本地实时状态机，PC 仍必须通过明确的事务 ID、阶段证据和故障状态与其对账。

## 未来接入顺序

1. 先在活动硬件仓建立可编译、可追溯的 Arduino 固件来源。
2. 定义带版本、身份、能力、事务 ID、超时和故障码的控制器协议。
3. 为现有 PC 直控行为建立设备端口契约和协议一致性测试。
4. 新增 `controller-managed` backend，实现相同端口，不改 UI 命令。
5. 运行相同的扫描事务、安全故障和恢复测试；真实硬件结论必须单独授权取得。
6. 两条 backend 均显式选择；连接失败时不得在两者之间静默回退。

## 需要正式决策的问题

- D7100 是否仍由 PC 直接保存，还是由控制器只提供触发同步；
- X 射线关束的独立硬件链路和 watchdog 放置位置；
- PC 与控制器失联时谁执行最终停机，以及如何证明已停机；
- 扫描 manifest 中记录哪些控制器固件、能力和校准版本；
- PC 直控与控制器主控是否长期并存，或只在迁移期并存。

这些问题在有活动固件和硬件证据之前只作为待决项，不写入当前设备协议。
