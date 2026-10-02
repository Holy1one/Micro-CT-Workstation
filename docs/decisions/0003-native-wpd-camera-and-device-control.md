# ADR 0003：原生 WPD 相机与设备自主控制

状态：已接受，待实现。

本决定替代 [ADR 0002](0002-no-out-of-process-camera-bridge-or-algorithm-plugins.md)
中的相机传输技术选择；禁止额外相机桥进程和算法插件的决定继续有效。本文记录
已确认的目标契约，不表示原生后端已经上线或设备已经获得硬件认证。实施前的
活动相机实现仍为 `digicam_control.rs` 调用外部 `CameraControlRemoteCmd.exe`。

## 背景

相机自主控制需要设备身份、可读写属性、Live View、可靠拍摄事件与主机落盘证据。
现有外部命令调用不能提供所需的统一会话、取消和事件关联契约。设备长操作也不能
阻塞全局 Stop、关束或持续安全监测。相机与转台应能独立连接并检查自身，完整扫描
仍由 `ct-engine` 执行整套预检和事务。

## 技术与能力边界

- Windows 相机传输在 Rust `ct-engine` 内使用 WPD/PTP，按 Microsoft 接口独立实现。
  WPD COM 对象在专用线程创建、使用和释放，对外仅暴露 Send 消息代理，不对真实
  COM 对象添加 `unsafe Send`，不增加相机桥进程。
- 必要 Nikon 协议仅移植经核验的 MIT 来源，保留版权、许可与来源。不得直接翻译或
  并入 LGPL PortableDeviceLib、复制完整 CameraControl.Devices 工程或闭源 DLL。
  不安装旧 INF、不替换 USB 驱动。
- 多品牌统一发现、标准属性和能力协商；明确选择一个活动相机，保存身份、设备 ID、
  会话和新鲜度，不隐式选择列表末项。完整厂商适配与正式扫描首期仅面向 D7100 NEF。
  其他型号仅开放设备实际描述且安全语义有证据的标准能力，无法证明 host-only 时
  禁用通用捕获，不向未知设备试发 Nikon 操作，不承诺 Canon/Sony 全功能或扫描兼容。
- 不自动回退到 DigiCamControl、存储卡采集或模拟成功。离线候选不能声称真实相机
  事件语义或硬件能力已经验收。

## 领域控制与安全

- `ct-engine` 保持设备、任务、安全和事务的唯一权威。扫描顺序由扫描协调器持有；
  自主 HOME 等跨设备安全裁决由领域入口持有；设备模块互不调用。UI 展示快照并发送
  命令，Tauri 只负责桌面能力、生命周期与转发。
- 每个设备由单一 worker 持有通信状态。主循环定时处理设备事件、超时和退出，安全
  监测不依赖 UI 请求。连接、参数应用、HOME/MOVE、捕获和断点检查异步执行；命令
  受理、取消请求、硬件完成和清理未确认分别表达。
- Stop/OFF/EOF 优先，取消相机、Nano STOP 与 Moxtek OFF 并行发起，OFF 不等待
  相机或运动。停止使旧代次失效，迟到事件不能提交、开束、恢复预览或解锁。关束
  失败仍为 UNKNOWN 并锁故障，未确认清理的相机会话隔离，不假称完成。
- 独立单拍、Live View 和规定方向旋转不自动开束，可以配合操作员手动出束；真实
  故障仍由引擎安全处置。相机、转台连接不依赖完整扫描参数或其他设备连接。
- HOME 在连接射线源时先关束读回确认，完成后不自动开束。仅转台台架模式允许
  操作员确认射线物理隔离后 HOME，射线仍显示 UNKNOWN/未监测并禁止开束。连接
  射线后重新验证；曾连接而 OFF 未知时，台架确认不得覆盖未知。
- 普通运动只沿既有生产方向，UI 正数表示沿该方向前进，协议维持负符号，前后端
  均拒绝逆向。HOME 保留固件内部有界反向退让例外，不修改已调回零和脉冲时序。
  首版相对运动为 `(0,360]` 度，绝对目标为 `[0,360)` 度；按已确认展开位置规划
  `MOVE_REL`，已到目标不额外绕圈。到位、同 ticket 的 `CAPTURE_DONE`、IDLE 都
  确认后才完成动作；不开放任意多圈、速度、微步或 ZERO 代替 HOME。
- 扫描、暂停、收尾、停止清理及重构期间锁定手动控制，Stop 和状态读取始终可用。
  持久未完成任务跨重启和 taskId 切换锁定参数、对焦和手动运动，受控恢复 HOME
  是例外。放弃必须先清理与确认必要 OFF，再持久标记解锁；保留文件，不标记
  Completed、不覆盖原目录。

## 相机参数、预设与 Live View

- 属性呈现枚举、可读写能力、目标、读回和失败原因。参数从草稿经明确 Apply 串行
  写入并读回，提前拒绝不支持值；部分失败展示实际/未知状态并失效预检，不伪造回滚。
- 快门与扫描 Exposure 共用目标，应用预设同步 Exposure；扫描前必须写入并读回
  一致，保留原生档位精确匹配，不舍入。正式扫描要求 RAW 单张、定时快门、确定
  自动模式与采集响应，不自动重新对焦；不可读物理焦点位置始终未知。
- 版本化 JSON 文件保存多个命名预设，只包含已确认的相机参数与来源标签。支持比较、
  原生对话框导入/导出，在应用用户数据目录原子存储；加载只到草稿，明确 Apply 才
  写设备。能力、版本或结构不兼容时拒绝，不跳过字段；不保存射线、路径、投影数、
  转台动作或未知焦点坐标。
- Live View 提供适应窗口、缩放、网格/十字线、单次 AF 和支持时的镜头步进对焦。
  默认最高 10fps，latest-only 丢旧帧，不承诺固定帧率；严格验证长度、尺寸与 JPEG。
  Live View 开启期间前后端均拒绝拍摄，不自动停预览拍摄再重开；关闭控制台结束
  Live View，进入扫描采集前确认停止。会话变化、失联或过热撤销旧帧实时身份。
- 图像不进入 JSONL 快照；使用引擎拥有的本地 named pipe，经 Tauri 受控转发。端点
  使用当前用户 ACL、随机会话名、版本、session/seq、长度上限及 codec/尺寸校验，
  拒绝截断/重复/旧帧，latest-frame slot 保证慢消费者不阻塞控制和 Stop。前端不接受
  任意 pipe 名或路径。JSONL 仅传状态与新鲜度。

## 拍摄、扫描与旧数据

- 拍摄证据绑定本次 session/operation：可靠 `ExposureEnded`、对象/传输阶段、完整
  文件、长度、格式与 SHA256 分开记录。重复、旧、迟到或无关联事件不得串帧；不以
  临时文件出现或曝光加固定余量推定曝光结束。下载和取消共享单调 deadline，WPD
  卡住时隔离会话，不能假称取消成功。
- D7100 使用主机 RAM；验证协议响应、对象长度、格式、完整文件与哈希后原子落盘。
  独立单拍默认用户图片目录 `MicroCT/CameraCaptures`，可明确选目录，按时间与
  operation ID 命名，与扫描目录、帧数、manifest 和断点隔离。
- 投影仍按“到位确认→持续出束证据→拍摄→完整文件与哈希→同 MOVE ID 的
  `CAPTURE_DONE`→manifest/progress commit”执行。正常末张/暂停按可靠曝光结束
  证据关束，继续文件确认、capture hold 释放与提交后才 Completed/Paused；前空场
  末张按文件确认正常关束。Stop、故障、失联、未知和连续出束硬上限立即关束优先。
- 保持十张前暗场/十张前空场、空场整组一次连续出束、跨投影连续出束、两秒重开
  门控、`maxXraySec` 与既定 300 秒休息、扫描后无后参考和末张确认角度。保持现有
  数值重构算法和原始 NEF 只读，不用软件缓存证明跨 HOME 的物理角度配准。
- 新 manifest/preparation checkpoint schema 3 冻结设备身份/backend、采集参数、
  自动模式、实际读回、规范化 profile hash 与 preset 标签，恢复比较实际内容。旧
  schema 2 保留原文件，可读取、离线重构及按原缓存版本 5/hash 复用，但新后端不
  追加投影。旧任务继续保留旧版本处理，不重写旧 manifest/preprocessing/cache。

## 界面与交付

- 顶部增加 Devices 菜单，分别打开可滚动 Camera、Turntable 浮层；保留 File/Edit/
  Tools/Help、39px 自定义标题栏、品牌与 306px 底栏及主面板排布。主界面固定栏目
  不新增滚动，控制台始终提供安全 Stop，所有门控由后端再次执行。
- 设备重大迁移先交付 `release/native-device-controls-offline/` 离线候选，包含 EXE、
  source/build info/hash、第三方许可与离线报告，保留现用正式 EXE、来源与哈希。
  源码、模拟 transport/fake 及同生产协调器离线检查通过不能标记硬件认证完成。
- 真实固件逐字节一致性、D7100 事件关联、运动方向、负载角度/回零精度和联动扫描
  必须另行明确授权实机验收。此门槛前不得连接设备、读取 flash、烧录、拍摄、
  运动、出束或修改电路。

## 依据

- [Microsoft：Supporting MTP Extensions](https://learn.microsoft.com/en-us/windows/win32/wpd_sdk/supporting-mtp-extensions)
  描述通过 `IPortableDevice::SendCommand` 使用 WPD MTP 扩展及传输阶段。
- [Microsoft windows-rs：IPortableDevice](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Devices/PortableDevices/struct.IPortableDevice.html)
  提供接口方法与 COM 类型约束；接口存在不代表 D7100 的厂商事件语义已验证。
