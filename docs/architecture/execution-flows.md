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

## 采集与离线重构

1. 预检成功后，引擎给出几何参数提示。操作员填写实测的焦点至转轴、转轴至屏幕、有效屏幕宽度、屏幕中心偏移（mm）及水平镜像。转台旋转方向固定为逆时针，数值来源固定为实测；几何随任务写入清单。
2. Start 后，引擎弹一次前参考提示；确认后顺序拍前暗场 10 张、只开束一次连续拍前空场 10 张，最后一张文件确认后关束。空场没有第二个确认弹窗；若单次出束上限不足以拍完整组，关束报故障，不逐张循环开束。
3. 引擎确认关束后弹装样提示；确认后执行投影事务。每张投影均须确认已传输的 NEF 最终文件，并提交其索引、大小及哈希。
4. 最后一张投影提交后关束，确认警告输出关闭并封存 manifest。收尾不重复复核全部影像，不再拍后空场或后暗场。
5. 采集提示的取消、Esc 或关闭均发送 `cancel_scan_stage`。引擎负责安全结束和阶段断点；React 不自行推进阶段。恢复前仍需重新预检、必要的 HOME 与文件核对。
6. 封存后只能从底部 Reconstruction 区选择方法并打开重构预览；3D View 内不设重构入口。首次选择后引擎后台读取已校验的 NEF 和前暗场、前空场，执行可用方法并保存科学体、预览体和哈希。再次选择相同方法先核对缓存。Tauri `reconstruction_preview` 仅以方法 ID 读取引擎登记的预览缓存。

FDK 有 CPU 与真实 GPU 计算路径，首帧实测和数值比对通过且预计至少快 15% 才选 GPU；失败时全量 CPU 重算。小规模扫描可选 64³、五轮 CPU SIRT；超出计算预算时置灰。CGLS 无后端，保持不可选。体素间距是计算采样间距，不代表实测分辨率。0.8.0 已通过离线编译与自动测试，尚无 CT 实机和真实 D7100 NEF 验证。

## 故障与关闭

- UI 关闭或 Tauri 崩溃会导致 sidecar stdin EOF；`ct-engine` 在退出前停止扫描并强制关束。
- 界面 Stop 只请求结束活动扫描；工作线程完成关束读回与运动停止后才报告 Stopped。扫描正常收尾经过 Finishing；最后一张投影提交后保持转台在最后确认角度，确认关束和警告输出关闭、写入最终 manifest 后报告 Completed。收尾不重复读取最终 Nano STATUS，不发送额外 MOVE_ABS 或 HOME。
- 每轮真实扫描在任务图像目录保存 `scan.log`，记录扫描线程事件时间、来源、级别和最终 Completed/Stopped/Fault 结果；故障退出时仍写入已发生的事件与错误，便于区分投影提交后的关束、回零和文件收尾失败。
- 真实设备故障、通信丢失、超时和无法解释的状态必须尝试关束与停止运动，并进入失效安全路径；Nano 原生 STOP 和硬件急停保护保持有效。
- 关束未确认时只能报告故障，不能报告安全完成。
- 每次应用启动后的首次生产扫描必须 HOME。普通停止或故障导致 Nano STOP、失联或参考失效时，必须重新预检并 HOME。同一有效 Nano 会话中，预检读取实时 STATUS；仅 IDLE、无待提交投影、参考有效、已 HOME、已 REARM 且目标等于当前位置时可复用此前 HOME 基准，启动前再次核验。应用关闭仍发送 Nano STOP，不为了跨进程保留参考而削弱停机。
- Restore 验证 manifest、前暗场、前空场与投影图像哈希、几何和扫描参数，从记录的阶段或下一未提交帧继续。历史任务中已有的后参考帧仍保留在清单中，恢复时核对其记录，但不再拍摄或用于重构。故障、STOP 或参考失效后的 HOME 不删除已提交图像；跨 HOME 的物理角度配准并非文件校验可证明，应在重构验收中单独核对。
- 生产图像预览按已提交帧索引请求，Tauri 只读取引擎快照登记且哈希一致的 NEF 文件，并提取其中的 JPEG 供界面显示；预览不参与投影提交。

## 状态所有权

| 状态 | 唯一所有者 | 消费方 |
|---|---|---|
| 设备连接与健康 | `ct-engine` | Tauri 转发、React 展示 |
| 安全门控与 X 射线实测 | `ct-engine` | UI、3D 场景、日志 |
| 扫描 phase 与进度 | `ct-engine` | UI、manifest |
| 采集阶段、操作员确认与重构任务 | `ct-engine` | UI、Tauri 预览读取 |
| 原生路径与对话框 | Tauri | React |
| 主题、面板和相机视角 | React / scene | 用户界面 |
| 浏览器预览状态 | `WorkstationAdapter` | 非 Tauri 开发预览 |
