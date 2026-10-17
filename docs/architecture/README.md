# Micro-CT 软件架构

本文维护当前模块职责、调用方向和安全边界。源码、配置和测试是事实源；版本来自 package.json 与 backend 的配置，验收身份见 [验证索引](../validation/README.md)。路径归属与语义依赖分别由 [modules.json](modules.json)、[edges.json](edges.json) 检查。

## 当前生产链路

~~~text
React UI → Tauri invoke → EngineClient → JSONL stdio → ct-engine
                                                      ├─ scan coordinator
                                                      ├─ offline reconstruction
                                                      └─ devices
                                                         ├─ Moxtek serial owner
                                                         ├─ WPD/PTP camera owner
                                                         └─ Nano serial owner
~~~

PC 直接控制三类设备，ct-engine 是任务、设备、安全状态和扫描事务的唯一权威；设备模块互不调用。扫描协调器编排跨设备顺序，独立 HOME 的安全裁决留在领域入口。当前不存在已实施的 Arduino 中心控制 backend。

## 模块边界

| 路径 | 职责 |
|---|---|
| backend/ct-engine/src/lib.rs、device_controls.rs | 领域命令、安全门控、完整快照与操作结果 |
| backend/ct-engine/src/scan.rs、scan_project.rs、task_registry.rs | 采集编排、持久事务、项目核验与任务登记 |
| backend/ct-engine/src/devices/xray/ | Moxtek I/O、设定与实测、重开间隔及失效安全关束 |
| backend/ct-engine/src/devices/camera/ | 原生 WPD/PTP、身份与能力、参数、Live View、存卡与主机文件证据 |
| backend/ct-engine/src/devices/turntable/ | Nano 身份、协议、位置与 capture-hold 确认、STOP |
| backend/ct-engine/src/reconstruction*.rs | 只读已验证扫描、自动处理、科学体和版本缓存，不调用设备 |
| backend/ct-engine/src/software_update.rs | 维护占用与关联设备清理报告 |
| backend/src-tauri/ | OS、IPC、受控媒体读取、进程生命周期及更新传输 |
| frontend/src/engine/、frontend/src/platform/ | UI 契约与桌面入口；生产错误不回退模拟 |
| frontend/src/scene/ | 只读设备与重构显示，不发送领域命令 |
| frontend/preview/ | 离线开发模拟，不接触真实设备，不生成生产证据 |

## 命令与设备证据

命令经过版本、会话、请求 ID、序号、占用及安全校验，长操作异步执行，结果由唯一引擎提交。监测不依赖 UI 轮询；Stop/OFF/EOF 优先。UI 分别显示受理、拒绝、清理未知和完成；设定值、目标角度、未知状态不能冒充实测或安全证据。

相机和转台可独立连接、自检；完整扫描须整套预检。参数只在明确 Apply 后写入并读回；预检、开始和恢复鲜读比较，不自动停止 Live View 或改写摄影参数。完整厂商适配首期为 D7100 NEF，其他型号只开放有证据的标准能力。

Live View 使用有界内存、随机会话 named pipe 和最新帧策略，验证 ACL、版本、序号、长度、尺寸及 JPEG，不把实时像素写入 JSONL 或临时文件。LV 期间拒绝拍摄；专用控制仅接受当前会话的已广告能力。摄影目录、属性依赖与通道限制见 [相机矩阵](native-camera-capability-matrix.md)。

## 采集与精确清卡

prepare_scan 原子提交任务、投影数、出束上限与实测几何；路径取唯一图片根目录，曝光取已确认原生快门。取消准备不创建任务锁或设备动作。前暗场 10 张、前空场 10 张，空场连续一次开束，关束后提示装样；跨投影连续出束，结束不拍后参考。

默认存卡采用 schema 4 / profile 7，扫描中零图像载荷；显式选择主机 RAM 才采用 schema 3 / profile 6。存卡早释放默认开启，每张最多一个未完成拍摄；可靠曝光结束可提前释放并预取下一位点，下一张触发仍等前张回执持久化及当前到位。末张保持确认角度，无额外整圈 MOVE 或 HOME。

安全收尾后自动导入，再次确认 OFF，逐文件校验并封存。仅对本次回执唯一匹配且本地副本已验证的卡对象记录删除意图、精确删除并鲜读确认；其他照片保留。失败保留本地成果、剩余对象和恢复记录，全部清卡及终态登记完成后才报告完整流程完成。细节见 [存卡契约](../decisions/0004-camera-sd-card-acquisition.md)。

## 安全、停止与恢复

所有开束共用 Moxtek adapter；已确认关束后至少 2 秒才再开束。maxXraySec 是连续出束硬上限；300 秒关束间隔是项目策略。空场无法整组完成时关束报错，不自动循环。Stop、联锁、失联、超时和未知状态优先尝试关束与停止运动，未知 OFF 不报告安全完成。

首次生产扫描建立 HOME 参考；后续是否可用依当前设备证据，STOP、复位、失联或故障失效参考。恢复核验冻结参数、卡回执或本地 hash，以及连续位置证据，必要时受控 HOME；文件一致不证明跨 HOME 的物理配准。

Pause 在完整事务边界关束并保留任务。Resume 重新比较当前证据，受理不等于已恢复。操作员 Stop 完成清理和登记后结束当前采集并保留所有文件；退出、断开或自动故障保留中断任务以便恢复。清理或持久写入失败保持锁。已结束目录仍在时不可复用；仅保存根可读且任务目录明确不存在才撤销旧登记，不能把读错误或断盘当删除证据。

## 内置自动重构与显示

默认单色/多色入口使用 spectral/v3-<input-hash> 独立缓存：参考驱动、颜色无关的亮暗及色彩缺陷检测，同通道局部增益修补，暗空场平衡、带质量标记的负对数、小型缺失射线估计、归一化去噪、轴校准与锥束 FDK。真实边缘和支架受保护，插补区域记录为估计，不作为定量颜色测量。

单色非材料模式另尝试 64³、八轮 PWLS-Huber 粗网格锥束修正，拟合角度与独立验证角度均改善且幅度受限才采用；不通过则保留 FDK。自动预览使用有符号 P1–P99.5 拉伸，科学体保留 float32。旧无 options 调用、旧方法缓存与 v2 保持各自数值和显示约定，不能把旧版的零下限或固定轴搜索范围当作 v3 规则。

FDK 的 CPU/GPU 路径经首帧数值对照和实测加速门槛选路，GPU 失败全量 CPU 重算。旧通用方法保留预算限制的 CPU SIRT，CGLS 没有后端并保持禁用；当前颜色入口只使用 FDK。屏幕 ROI、几何和参数来自记录及可核验参考，不改写原始 NEF 或 manifest。新算法与显示增强不证明材料身份、实际空间分辨率或 CT 定量精度。细节见 [自动重构记录](../validation/automatic-cbct-2026-10-15.md)、[叠层成像决策](../decisions/stacked-scintillator-material-imaging.md)。

底部 Reconstruction 区启动计算，完成且预览校验成功后显示结果；Equipment / Result 仅切换显示，普通轮询不抢走用户选择。3D View 不重复提供重构入口。

## 更新维护与验证边界

更新先取得本会话/操作的维护锁与设备清理报告，UNKNOWN 或未确认清理不能授权安装。Tauri 只运行安装目录中哈希匹配的引擎；更新 helper 等待原应用及 owned engine 正常退出。退出码、签名、下载、安装结果与硬件安全分别核验。用户明确要求安装可按授权完成；安装不自动连接、运动或开束。

当前设备分项证据与已知质量限制见 [验证索引](../validation/README.md)，不把旧版分项验收提升为当前整机认证。构建与安装规范见 [更新维护](../guides/desktop-update-distribution.md)。

## 阅读顺序

1. [目录职责](directory-map.md) 与 [模块图](module-graph.md)。
2. [执行流程](execution-flows.md)。
3. [前端功能及接口](frontend-functional-spec.md)、[错误码](frontend-error-codes.md)。
4. [原生控制决策](../decisions/0003-native-wpd-camera-and-device-control.md) 与 [存卡契约](../decisions/0004-camera-sd-card-acquisition.md)。
