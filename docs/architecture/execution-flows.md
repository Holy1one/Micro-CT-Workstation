# 执行逻辑

## 桌面启动

1. Tauri 创建窗口和共享 `EngineClient`。
2. 后台线程启动 `ct-engine` sidecar，避免阻塞 Tauri `setup`。
3. `EngineClient` 校验 sidecar 二进制并建立 stdin/stdout JSONL 通道。
4. React 首次调用 `engine_snapshot`；sidecar 未就绪时返回明确错误，不切换到预览。
5. 操作者显式连接真实设备并运行预检；启动应用本身不触发真实运动或出束。

## 浏览器预览启动

1. `createEngineAdapter` 检测不到 Tauri runtime。
2. 创建 `WorkstationAdapter` 和内存 RTS9060 transport。
3. 预览生成与生产相同形状的 `EngineSnapshot`，但不得打开串口、调用 DigiCamControl 或接触真实设备。
4. 一旦处于 Tauri runtime，任何 IPC 失败都直接报错；不得回退到预览。

## 命令执行

```text
operator gesture
  -> React dispatch(EngineCommand)
  -> Tauri invoke("engine_command")
  -> EngineClient request envelope
  -> ct-engine phase/safety validation
  -> device action or state transition
  -> complete EngineSnapshot
  -> React render
```

命令是意图，不是事实。例如 `send_voltage` 只修改请求设定；只有设备回读才能成为实测电压。UI 不得根据按钮点击自行宣布设备已完成动作。

## 故障与关闭

- UI 关闭或 Tauri 崩溃会导致 sidecar stdin EOF；`ct-engine` 在退出前停止扫描并强制关束。
- STOP、E-STOP、通信丢失、超时和无法解释的设备状态都按不安全处理。
- 关束未确认时只能报告故障，不能报告安全完成。
- 释放 E-STOP 不恢复旧扫描资格；必须重新连接、预检和 HOME。

## 状态所有权

| 状态 | 唯一所有者 | 消费方 |
|---|---|---|
| 设备连接与健康 | `ct-engine` | Tauri 转发、React 展示 |
| 安全门控与 X 射线实测 | `ct-engine` | UI、3D 场景、日志 |
| 扫描 phase 与进度 | `ct-engine` | UI、manifest |
| 原生路径与对话框 | Tauri | React |
| 主题、面板和相机视角 | React / scene | 用户界面 |
| 浏览器预览状态 | `WorkstationAdapter` | 非 Tauri 开发预览 |
