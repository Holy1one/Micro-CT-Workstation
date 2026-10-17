# Windows 集成边界

安装目标为 Windows 10 / 11 x64；现有实际安装和桌面证据来自 Windows 11，Windows 10 尚需独立运行验收。设备分项记录见 [验证索引](../validation/README.md)，不等于当前整机能力认证。

## 进程与传输

- ct-workstation.exe 为 Tauri 2 + WebView2 桌面应用。MSI 默认安装至当前用户 Programs/Micro-CT Workstation，也允许明确选择其他可写位置。
- ct-engine.exe 与应用同目录安装，桌面启动先校验编译时记录的 SHA-256。引擎唯一拥有设备、安全、任务和离线重构，不向 C 盘缓存释放另一份引擎。
- 设备控制只走 Tauri → JSONL stdio → ct-engine，不开放 localhost 设备控制服务，生产 IPC 错误不回退预览。
- 相机 WPD COM 在线程内创建、使用和释放。存卡与 RAM 路径分别保持回执或文件完整性语义；不调用 DigiCamControl、不增加相机桥进程。
- Live View 经当前用户 ACL、随机会话端点的 named pipe 传输有界 JPEG；JSONL 只传状态。专用会话、序号、尺寸、长度和 codec 校验不可绕过。
- 重构算法内置 Rust；默认颜色入口自动处理后用 FDK，适用时尝试有验证门槛的 PWLS-Huber 修正。旧通用入口保留受预算限制的 CPU SIRT，CGLS 不可用，不引入算法插件运行时。

## 设备与状态所有权

生产落点为 backend/ct-engine/src/devices/{camera,turntable,xray}/。设备模块处理自身协议和证据，互不调用，不推进扫描。scan.rs 持有跨设备顺序，领域入口持有自主 HOME 等安全裁决。当前唯一生产拓扑为 PC 直控。

HOME、连续出束、两秒重开、Stop/EOF 清理与未完成任务锁见 [原生控制](native-device-controls.md)。当前默认存卡的容量、早释放、导入和只删本次图片见 [存卡契约](../decisions/0004-camera-sd-card-acquisition.md)。旧 manifest 和原始 NEF 只读，当前算法输出独立版本缓存。

## 构建、安装与验收

按改动影响执行适用的 Rust、TypeScript、IPC、桌面或固件门禁；文档整理无需无关的全量构建。交付 MSI 必须核对包内应用与引擎同源身份、启动及正常退出，构建不自动安装。

用户明确要求安装/更新后，可在授权范围内完成维护停机和安装；维护报告、进程退出、安装结果与二进制身份分别核验。保留旧包及可核验来源，不回退旧设备工具。release/ 只保留最新 MSI，来源、签名、许可和历史包归本机证据缓存，详情见 [安装与更新](desktop-update-distribution.md)。

真实固件、相机事件、Moxtek 实测/联锁、联动扫描及物理精度按具体版本和工况验收。VERSION/BUILD 字符串、模拟结果、成功安装或更清晰的预览均不能证明硬件安全或成像定量准确。没有本次设备动作证据时报告“仅离线验证”。
