# ADR 0003：原生 WPD 相机与设备自主控制

状态：已实现，存在按版本记录的设备分项证据；完整能力认证仍按 [验证索引](../validation/README.md) 的边界报告。

## 原生通信

- Windows WPD/PTP 在 Rust ct-engine 内实现，COM 对象由专用线程创建、使用和释放，只暴露消息代理，不为真实 COM 对象添加 unsafe Send。
- 必要 Nikon 协议保留经核验的 MIT 来源、版权和许可；不并入 LGPL PortableDeviceLib 或闭源 DLL，不替换 USB 驱动，不回退外部相机工具或模拟成功。
- 明确选择一个相机身份，并协商其实际描述与能力。完整厂商适配和正式 NEF 扫描首期覆盖 D7100；其他型号只开放有证据的标准能力，不试发未知厂商操作。
- 扫描存卡与主机 RAM 是显式选择的不同事务；当前默认存卡契约见 [ADR 0004](0004-camera-sd-card-acquisition.md)，不静默切换目标。

## 领域所有权

ct-engine 唯一持有设备、任务、安全与扫描事务；scan.rs 编排跨设备顺序，自主 HOME 等跨设备裁决由领域入口持有。Moxtek、相机和 Nano 各有单一通信 owner，设备模块互不调用。UI 展示完整快照并发送意图，Tauri 管理 OS、IPC 与生命周期。

长操作不阻塞独立监测、状态读取和 Stop/OFF/EOF。全局 Stop 并行请求相机取消、Nano STOP、Moxtek OFF；关束不等待相机 I/O 或运动。受理、取消、清理未知和硬件完成分别表达，迟到事件不能恢复动作或提交旧结果。

## 自主控制

- 相机与转台可独立连接、自检。独立单拍、Live View 和规定方向运动不自动开束；完整扫描仍通过整套预检。
- HOME 在射线已连接时先关束并读回确认，完成后不自动开束。仅转台台架模式需现场物理隔离确认，射线保持 UNKNOWN；不能覆盖曾连接后的未知 OFF。
- 普通运动只沿既有生产方向，协议维持负符号，HOME 保留固件内部有界反向退让。到位、匹配 ticket 的 CAPTURE_DONE 和 IDLE 是运动事务证据。
- 扫描、暂停、收尾、清理和重构锁住手动控制；持久未完成任务跨重启与任务切换保持锁。明确放弃并完成清理后解锁，保留所有文件。

## 摄影参数与像素

- 只有明确 Apply 写入参数并读回；预检、开始和恢复仅鲜读比较冻结值。属性、选项依赖及拒绝原因由引擎发布，部分拒绝不伪造回滚或设备成功。
- 预设只保存已确认摄影参数与来源；加载到草稿，Apply 才写入，不存射线、转台动作或未知焦点坐标。
- Live View 期间拒绝拍摄，不自动停预览拍摄再恢复。其专用控制绑定独立会话，普通摄影 Apply 不获得额外权限。
- Live View 经引擎拥有、当前用户 ACL 与随机会话端点的 named pipe 传递，验证版本、序号、长度、尺寸和 JPEG；只保留最新帧，不把实时像素写入 JSONL 或文件。
- 独立 SD 单拍只确认存卡回执；显式授权主机传输的 RAM 单拍保存至当前图片根目录的 CameraCaptures，与扫描目录和帧数隔离。照片预览按 capture ID 解析登记与哈希，不接受 renderer 任意路径。

## 数据与交付

拍摄证据关联 session/operation，可靠曝光结束、卡回执、主机文件完整性及哈希分别记录。曝光计时、临时文件出现或进程退出不能代替设备证据。schema 3/4 保存各自冻结采集资料；旧 schema 2 可只读重构与合法缓存复用，新后端不追加投影。算法允许按当前目标改进，输出独立版本缓存，原始 NEF 与旧成果保持只读。

设备迁移的离线通过不等于硬件认证。用户明确要求安装时按授权执行并分别核验身份和验证边界；真实设备动作仍依上级 AGENTS.md 核对本次范围。当前 UI 契约见 [前端规范](../architecture/frontend-functional-spec.md)，安装过程见 [更新维护](../guides/desktop-update-distribution.md)。

## 接口依据

- [Microsoft Supporting MTP Extensions](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/supporting-mtp-extensions)
- [windows-rs IPortableDevice](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Devices/PortableDevices/struct.IPortableDevice.html)
