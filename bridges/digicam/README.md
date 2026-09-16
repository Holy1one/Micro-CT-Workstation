# digiCamControl Bridge 契约

这里是 Windows 专用 .NET 相机桥的预留入口。当前没有 bridge 可执行文件或模拟实现，也没有连接、枚举或触发相机。

## 最小进程边界

- `ct-engine` 在显式启用 Nikon adapter 后启动并独占 `digicam-bridge.exe`；UI 不直接调用 .NET 或 digiCamControl SDK。
- V1 使用 UTF-8 JSONL stdio。每行最多 64 KiB；stdin EOF 表示父进程退出，bridge 必须释放相机后退出。无需提前引入 Named Pipe。
- bridge 只做 SDK 兼容、相机发现/连接、设置、单次拍摄、文件落盘与停止。扫描任务、投影序号、重试策略、安全门控和状态推进全部属于 `ct-engine`。
- SDK 调用放入单一相机 actor；stdio 读取与 SDK 调用解耦，使幂等 `camera.stop` 能在捕获超时期间被处理。
- engine 预留采集临时文件的完整路径；bridge 只能写该路径。engine 复核大小和 SHA-256 后才原子提交科学产物并推进任务。

## 协议 v1

协议标识为 `digicam.bridge.v1`。请求和响应复用统一 envelope：

```json
{
  "protocol_version": 1,
  "request_id": "camera-42",
  "command": "camera.capture",
  "payload": {},
  "timestamp": "2026-09-16T07:30:00.000Z",
  "sequence": 42,
  "error_code": null
}
```

约束：

- `request_id` 在一次 engine 生命周期内唯一；响应必须回显 `request_id` 和 `command`。
- `sequence` 在单进程连接内严格递增；重放或乱序返回 `STALE_REQUEST`。
- `timestamp` 使用 UTC RFC 3339；`error_code` 成功时为 `null`，失败时为稳定机器码，细节放在 `payload.error`。
- v1 只有请求/响应，不发送会自行改变扫描状态的事件。未知版本、未知命令和超长行 fail-closed。

最小命令集：

| 命令 | 作用 | 关键返回值 |
|---|---|---|
| `bridge.hello` | 协商协议与能力 | bridge/SDK 版本、支持的协议版本、进程位数 |
| `camera.discover` | 只读枚举 | 稳定相机 ID、型号、序列号；不连接 |
| `camera.connect` | engine 授权后取得相机所有权 | 相机身份、连接状态 |
| `camera.health` | 读取状态 | busy、电池、存储/传输能力、最后错误 |
| `camera.configure` | 应用一次设置快照 | 请求值与 SDK 实际接受值 |
| `camera.capture` | 执行一个 engine 指定的 capture | 文件证据与远端删除结果 |
| `camera.stop` | 幂等取消/停止当前 SDK 操作 | `stopped: true`；不推进扫描 |
| `camera.disconnect` | 释放相机 | `disconnected: true` |

`camera.capture` 请求必须由 engine 给出不可复用的 `capture_id`、`task_id`、`projection_index`、相机 ID、设置快照和 `host_temp_path`：

```json
{
  "capture_id": "cap-000042",
  "task_id": "CT-2026-001",
  "projection_index": 41,
  "camera_id": "nikon:<serial>",
  "settings": {
    "exposure_ms": 180,
    "iso": 100,
    "format": "raw"
  },
  "host_temp_path": "D:\\MicroCT\\.staging\\cap-000042.nef",
  "delete_remote_after_verified_transfer": true
}
```

成功响应的 `payload` 至少包含：

```json
{
  "capture_id": "cap-000042",
  "camera": {
    "id": "nikon:<serial>",
    "model": "Nikon D7100",
    "serial_number": "<serial>"
  },
  "effective_settings": {
    "exposure_ms": 180,
    "iso": 100,
    "format": "raw"
  },
  "file": {
    "path": "D:\\MicroCT\\.staging\\cap-000042.nef",
    "size_bytes": 12345678,
    "sha256": "<64 lowercase hex chars>"
  },
  "remote_delete": {
    "requested": true,
    "confirmed": true
  },
  "capture_started_at": "2026-09-16T07:30:00.100Z",
  "capture_completed_at": "2026-09-16T07:30:02.500Z"
}
```

`remote_delete.confirmed` 只能在主机文件关闭并计算 SHA-256 后为 true。它不等于任务完成；engine 还要独立校验文件并提交 manifest。建议的稳定错误码包括 `PROTOCOL_VERSION_MISMATCH`、`INVALID_ENVELOPE`、`CAMERA_NOT_FOUND`、`CAMERA_BUSY`、`SDK_UNAVAILABLE`、`SETTINGS_REJECTED`、`CAPTURE_TIMEOUT`、`TRANSFER_FAILED`、`HASH_FAILED`、`REMOTE_DELETE_UNCONFIRMED`、`STOPPED` 和 `INTERNAL_ERROR`。

## 实现前置条件

必须先获得实际 digiCamControl/厂商 SDK 程序集、许可和目标框架证据，再选择 `net48` 或 `net8.0-windows`；不得仅凭已安装 runtime 猜测兼容性。首个实现应只覆盖上述契约，并用假 SDK 做 bridge 单元测试；真实相机拍摄需要另行明确授权。
