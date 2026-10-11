# digiCamControl 拍摄语义对照（2026-10-11，D7100 主机 RAM 单拍无 NEF 的参考取证）

本文件只做**只读**外部参考阅读，目的是给实机缺陷（`docs/validation/card-sd-migration-hardware-2026-10-11.md` 第 7 节：
`InitiateCapture` 被接受、机身确实曝光、但引擎在截止时间内未见曝光结束/完成事件、无 NEF）提供可引用的
第二方证据。**不触碰硬件、不修改任何源码、不提交**。本文件本身不构成硬件验收。

引用根目录为 `Related_files/digiCamControl-master/`，下文所有 `path:line` 均相对于该根目录。
参考实现为 C#（MIT，`CameraControl.Devices` 等）。

一处前提更正（避免按错误的类名检索）：**该树里不存在 `WpdProtocol` 类**。WPD 通道由
`PortableDeviceLib/`（Windows Portable Device API 封装）实现，设备层经 `TransferProtocol/MtpProtocol.cs:9`
（`class MtpProtocol : StillImageDevice`）接入，实例化点在 `CameraControl.Devices/CameraDeviceManager.cs:608`
与 `:634`；机型到驱动类的映射为 `CameraDeviceManager.cs:181`（`"D7100" → typeof(NikonD7100)`）。

## 0 取证范围（只读）

读过的文件（相对 `Related_files/digiCamControl-master/`）：

- `CameraControl.Devices/Nikon/NikonBase.cs`：opcode/事件/属性常量、拍摄发起、事件循环、属性回读。
- `CameraControl.Devices/Nikon/{NikonD7100,NikonD600Base,NikonD800,NikonD300,NikonD700}.cs`：D7100 继承链与机型例外。
- `CameraControl.Devices/{BaseMTPCamera,BaseCameraDevice,CameraDeviceManager}.cs`：标准 opcode、事件常量、传输、机型映射与 WPD 接入。
- `PortableDeviceLib/{StillImageDevice,PortableDeviceEventCallback,PortableDeviceGuids}.cs`：WPD 侧 MTP 分块读与事件回调。
- `CameraControl.Core/Classes/{CameraHelper,PhotoSession,PhotoUtils,FileItem,BraketingClass}.cs`：等待时限、命名、扩展名判据、包围序列。
- `Camera/{MainWindow.xaml.cs,Controls/Controler.xaml.cs,App.xaml.cs}`：照片流水线、传输三档 UI、属性恢复。
- `CameraControlCmd/Program.cs`、`CameraControl.Core/Scripting/CommandLineProcessor.cs`：非 GUI 的拍摄与等待用例。

检索命令（用于确认"不存在"类结论）：全树 grep `InitiateCapture|0x90C0|CaptureComplete|0x400D`、
`0xD1B4|0xD1B2`、`0x101b|PartialObject`、`CaptureInSdRam`、`RecordingMedia`。未做任何编译、运行或设备交互。

## 1 拍摄发起的位置

- 设备层通用入口：`CameraControl.Devices/BaseMTPCamera.cs:123-145` 的 `CapturePhoto()`，体内只有一次
  `ExecuteWithNoData(CONST_CMD_InitiateCapture)`；Nikon 覆盖在 `Nikon/NikonBase.cs:1862-1888`（`CapturePhoto`）
  与 `NikonBase.cs:1890-1941`（`CapturePhotoNoAf`）。
- 发起前准备：`SpeedUpEventTimer()`（`NikonBase.cs:1864`）与 `DeviceReady()`（`NikonBase.cs:1869`，实现见
  `NikonBase.cs:2404-2438`，opcode `0x90C8`）。发起后只置 `IsBusy = true`，**不同步等待图像**。
- 应用层调用：`CameraControl.Core/Classes/CameraHelper.cs:49-76`（`camera.CapturePhoto()`）、
  `CameraControlCmd/Program.cs:461-471`。
- D7100 自身的继承链：`Nikon/NikonD7100.cs:41`（`NikonD7100 : NikonD600Base`）→
  `Nikon/NikonD600Base.cs:45`（`NikonD600Base : NikonD800`）→ `Nikon/NikonD800.cs:187-197`（只重排能力）。
  整个 `Nikon/` 目录里只有 `NikonD40.cs:59`、`NikonD3200.cs:54` 覆盖 `CapturePhoto`，只有
  `NikonD80.cs:97`、`NikonD90.cs:84`、`NikonD300.cs:66`、`NikonD700.cs:75` 覆盖 `CapturePhotoNoAf`；
  **D7100 链上没有任何拍摄覆盖**，因此 D7100 走的是 `NikonBase` 的通用实现。

## 2 各路径使用的 opcode / 参数

标准 opcode 表在 `BaseMTPCamera.cs:74-87`，Nikon 私有码表在 `NikonBase.cs:55-71`。

| 目的 | 选择依据 | 非 Live View | Live View 下 |
| --- | --- | --- | --- |
| 机身自己写卡（宿主可选再传） | `CaptureInSdRam == false` | `CONST_CMD_InitiateCapture = 0x100E`（`NikonBase.cs:1872`；常量 `BaseMTPCamera.cs:82`） | `CONST_CMD_InitiateCaptureRecInMedia = 0x9207`，参数 `0xFFFFFFFF, 0x0000`（`NikonBase.cs:59`、`:1913`） |
| 图像留在机身 SDRAM 待主机取回（主机 RAM） | `CaptureInSdRam == true` | `CONST_CMD_AfAndCaptureRecInSdram = 0x90CB`（`NikonBase.cs:60`、`:1871`） | `CONST_CMD_InitiateCaptureRecInSdram = 0x90C0`，参数 `0xFFFFFFFF`（`NikonBase.cs:61`、`:1908`；非 LV 分支 `:1925`） |

- 即：**"先自动对焦再拍"用 `0x90CB`，"不 AF 直接拍进 SDRAM"用 `0x90C0`；两者的图像都在机身 SDRAM 里**，
  与"写卡"是两条不同的 opcode 路径。
- `0x90C7` 在参考实现里是 `CONST_CMD_GetEvent`（`NikonBase.cs:66`），**不是拍摄操作**；`0x90C8` 是
  `CONST_CMD_DeviceReady`（`NikonBase.cs:68`）。本项目实机记录中的 `operationCode=0x90C7` 属于事件轮询操作
  （对照本项目 `backend/ct-engine/src/devices/camera/nikon.rs:28` 与 `:1240-1273`），不应读作"拍摄操作被接受"。
- `0x920C TerminateCapture` 只用于 bulb 结束（`NikonD800.cs:185`、`:222`），与单张静态拍摄无关。
- 参考实现**从不发送 `0x101B GetPartialObject`**：全树 grep 无 `0x101b`/`PartialObject` 命中。

## 3 完成检测：等哪些事件、超时、各事件的地位

事件轮询与解析（`NikonBase.cs:2298-2402`）：

- 轮询节奏 100 ms：`_timer` 初值 `1000 / 10`（`BaseMTPCamera.cs:100`），`SlowDownEventTimer()` 与
  `SpeedUpEventTimer()` **都把 Interval 设成同一值**（`NikonBase.cs:346-354`），定时器到点把 `GetEvent`
  丢进线程池（`NikonBase.cs:356-370`）。另外 WPD 设备事件也直接触发一次 `GetEvent`
  （`NikonBase.cs:1109-1124`）；WPD 层只在 `WPD_EVENT_OBJECT_ADDED` 时附带对象信息
  （`PortableDeviceLib/PortableDeviceEventCallback.cs:46-54`）。
- 读取：`ExecuteReadDataEx(CONST_CMD_GetEvent)`（`NikonBase.cs:2306`）；`eventCount` 在偏移 0（`:2314`），
  每条记录 6 字节，`eventCode = u16 @ +2`、参数 `u16/int32 @ +4`（`:2321-2323`）。
- 事件通道是**静默**的：数据集为空只写一行日志后返回（`:2308-2313`）；整个解析被 `try/catch` 包住，
  异常不冒泡（`:2392-2399`）；无论如何最后都复位 `_eventIsbusy` 并重启定时器（`:2400-2401`）。
  这正是库不需要拍摄超时的原因之一——事件不来时它既不报错也不判失败。

各事件的处理与地位：

- `0x4006 CONST_Event_DevicePropChanged`（常量 `NikonBase.cs:108`）→ `ReadDeviceProperties(eventParam)`
  （`:2326-2328`）。**它只是"请刷新这个属性"，在参考实现里从不作为曝光结束或文件完成证据。** 全树没有
  `0xD1B4`、`0xD1B2` 的任何出现（grep 未命中），即参考实现不认识本项目实测到的连拍计数属性。
- `0x4002 CONST_Event_ObjectAdded` / `0xC101 CONST_Event_ObjectAddedInSdram`（`:2330`、`:2329`；常量
  `BaseMTPCamera.cs:86-87`）→ **立即 `GetObjectInfo(handle)` 取文件名并抛 `OnPhotoCapture`**
  （`:2333-2367`）。这是**"图像已可用"的实际触发点**，应用据此开始传输。
- `0x400D CONST_Event_CaptureComplete` / `0xC102 CONST_Event_CaptureCompleteRecInSdram`
  （常量 `NikonBase.cs:110-111`）→ `SlowDownEventTimer(); OnCaptureCompleted(...)`（`:2369-2375`）。
  这是"曝光完成"信号，但**文件传输不依赖它**：应用侧订阅 `CaptureCompleted` 的只有包围/连拍序列推进
  （`CameraControl.Core/Classes/BraketingClass.cs:143`、`:248-254`）与命令行模式
  （`CameraControlCmd/Program.cs:544-556`），主窗口的照片流水线只订阅 `PhotoCaptured`
  （`CameraControl/MainWindow.xaml.cs:270`、`:444-457`）。
- `0x400A CONST_Event_StoreFull`（`NikonBase.cs:109`）只作为常量存在，事件循环不单独分支。
- `0xC104 CONST_Event_ObsoleteEvent` 被显式忽略（常量 `NikonBase.cs:112`，处理 `:2376-2377`）；未知事件
  只写日志（`:2378-2382`）。

超时形状：

- **设备层没有任何拍摄完成截止常量**。唯一的等待是应用层 `WaitForCamera(int timeout)`——轮询 `IsBusy`
  到超时为止（`CameraControl.Core/Classes/CameraHelper.cs:165-174`，`Thread.Sleep(50)`），调用点给出的量级为
  1500 ms（`TriggerClass.cs:136`、`:139`）、2000 ms（`Camera/ViewModel/LiveViewViewModel.cs:2954`）、
  5000 ms（`CameraControl/App.xaml.cs:479`、`Camera/MainWindow.xaml.cs:353`）、30000 ms
  （`CameraControl.Core/Scripting/CommandLineProcessor.cs:45`）。
- 命令级忙重试：`1 ms × 100`（`BaseMTPCamera.cs:91-92`、`:350-366`）；传输重试 10 次，`COMException` 后
  `Thread.Sleep(200)`（`BaseMTPCamera.cs:57`、`:288-295`）。
- 结论：参考实现的策略是**事件驱动、永不因"没等到完成事件"而判失败**；`0xC101/0x4002` 决定图像是否可得，
  `0xC102/0x400D` 只是可选的收尾/进度信号。

## 4 之后如何取对象、如何命名与校验 NEF

- `GetObjectInfo = 0x1008`（`BaseMTPCamera.cs:79`）。SDRAM 事件路径直接用它取文件名：`NikonBase.cs:2333-2342`
  （UTF-16，偏移 53，长度 12×2 字节）。存储列表路径用同样的偏移，并按格式码过滤
  `0x3000/0x3801/0x3800`（`BaseMTPCamera.cs:194-202`）。
- `GetObject = 0x1009`（`BaseMTPCamera.cs:77`），传输入口 `TransferFile`（`:235-313`）：`lock (Locker)`
  → 停事件定时器 → `ExecuteReadBigData(0x1009, stream, ...)` → 恢复定时器；失败按 10 次重试。
- WPD 层实现（`PortableDeviceLib/StillImageDevice.cs:101-282`）：用 MTP 扩展命令
  `EXECUTE_COMMAND_WITH_DATA_TO_READ` → 循环 `READ_DATA` → `END_DATA_TRANSFER` 分块读，**不是 `0x101B`**。
- 命名：来自事件里的名字/扩展名（`Camera/MainWindow.xaml.cs:472`、`:514`、`:536-551`）；
  `PhotoSession.GetNextFileName`（`CameraControl.Core/Classes/PhotoSession.cs:580-611`），扩展名缺失时默认
  `.nef`（`:587`），`.nef` 在 `SupportedExtensions`（`:58-68`）与 `RawExtensions`（`:72`）中。
- 校验：**只有扩展名**。`FileItem.IsRaw` 判断 `.nef`/`.cr2`（`CameraControl.Core/Classes/FileItem.cs:243-250`），
  `PhotoUtils.IsMovie` 判断视频扩展名（`PhotoUtils.cs:194-203`）。没有魔数、EXIF、尺寸或哈希级别的 NEF 校验。
- 传输后不清理机身对象：Nikon 链没有覆盖 `ReleaseResurce`，基类是空实现
  （`CameraControl.Devices/BaseCameraDevice.cs:454-457`）。

## 5 "存卡"与"传回主机"分别写什么属性

- 属性码 `CONST_PROP_RecordingMedia = 0xD10B`（`NikonBase.cs:96`）。
- 通用写入点在 `NikonBase_PropertyChanged`（`NikonBase.cs:1099-1107`，也是 D7100 链上唯一生效的一处）：
  `CaptureInSdRam ? 1 : 0` → `SetDevicePropValue(0xD10B)` → 再回读。故**主机 SDRAM/传回 = 1，存卡 = 0**。
- 默认与来源：`NikonBase.Init` 末尾 `CaptureInSdRam = true`（`NikonBase.cs:450`），随后由保存的相机属性覆盖
  （`Camera/App.xaml.cs:455`、`Camera/MainWindow.xaml.cs:328`）；命令行强制 true，若设备未公布
  `CaptureInRam` 能力则回退 false（`CameraControlCmd/Program.cs:466-469`）。
- UI 三档语义（`Camera/Controls/Controler.xaml.cs:141-155` + `CameraControl.Core/Translation/TranslationStrings.cs:302-304`）：
  `1.Save to PC only` → `CaptureInSdRam = true`；`2.Save to camera only` → `false` 且 `NoDownload = true`；
  `3.Save to PC and camera` → `false` 且 `NoDownload = false`。因此**"只存卡"和"存卡并传回"用的是同一条
  `0x100E` + `0xD10B=0` 路径，差别只在是否下载**（`Camera/MainWindow.xaml.cs:495-509`、`:523`、`:537`、`:613`）。
- 机型例外（**不在 D7100 链上**，仅说明 0xD10B 的 1/0 是机型约定）：D300 在 Live View 期间反写
  `0xD10B=1/0`（`Nikon/NikonD300.cs:52-53`、`:61-62`），D700 同样（`Nikon/NikonD700.cs:59`、`:71`）。

## 6 库证据 vs 本项目推断

**库（参考实现）实际证明的**：

1. D7100 链（`NikonD7100 → NikonD600Base → NikonD800 → NikonBase`）没有拍摄覆盖，走 `NikonBase` 通用路径。
2. "写卡"与"留 SDRAM"是两组不同 opcode：`0x100E`（LV 下 `0x9207`）对 `0x90CB/0x90C0`（第 2 节）。
3. `0xD10B` 的写入映射 `SDRAM=1 / card=0` 且写后回读（`NikonBase.cs:1103-1105`）。
4. 事件解码格式与三条互不相同的处理线：属性刷新 / 对象可用（`0xC101/0x4002` → `GetObjectInfo` → 传输）/
   完成（`0xC102/0x400D` → 仅 `OnCaptureCompleted`）。
5. 传输只用 `GetObjectInfo 0x1008` + `GetObject 0x1009` 的 WPD 分块读，不用 `0x101B`。
6. 文件传输**不等待** `0xC102/0x400D`；设备层没有拍摄截止常量。
7. NEF 只按扩展名识别，无内容级校验；传输后不删除机身对象。

**属于本项目推断、库未证明的**：

- 真实 D7100 在 `0xD10B=1` + `0x90C0`（accepted）之后**具体发哪些事件、以何顺序、多久**。我们实测只看到
  `0x4006/0xD1B4`；库对这个组合没有任何记录或断言。
- `0xC101` 的参数是否恒为对象句柄。库把它直接当句柄用（`NikonBase.cs:2333-2334`），但没有任何形状校验；
  我们实测到的 `0x0B000001` 是"参数值"，不是被库验证过的句柄。
- `0x4006` 参数 `0xD1B4` 的语义。库不认识该属性（全树无命中），因此它**对"曝光是否结束"没有发言权**。
- 孤立 `0xC101` 的处理。库没有"空闲期"概念，收到 `0xC101` 就会取对象并触发传输（`:2329-2367`），
  这不能当成"孤立事件应当被忽略"的依据——库只是没有这个判据，不是判定它安全。

**不得过度声称**：库"能跑"不等于我们的机身"一定先发 `0xC101`"；库的无超时策略不等于我们可以取消截止。

## 7 对本项目主机 RAM 完成判据的最小建议

现状（供对照，本项目源码）：主机 RAM 单拍要求 `0xC102`(CAPTURE_COMPLETE_RAM) 与 `0xC101`(OBJECT_RAM)
同时成立（`backend/ct-engine/src/devices/camera/capture.rs:241-273`、`:319-333`）；RAM 路径拒绝
`0x4002/0x400D/0x400A`（`:292-294`），`0x4006` 只接受白名单属性，否则拒绝（`:295-309`）；空闲排空时任何
`0xC101/0xC102/0x4002/0x400D` 都被判为"未关联事件"并退役会话（`nikon.rs:1306-1329`）；整体截止
`CAPTURE_TIMEOUT = 75 s`（`actor.rs:40`），到期即隔离（`actor.rs:271-276`）。

建议改动（最小、可回退，仅改主机 RAM 单拍判据）：

1. **把 `0xC101` 提升为充分证据**：收到与本操作关联的 `0xC101` 即视为该次曝光产生了主机可读对象，立即
   `GetObjectInfo(0x1008)` 关联并按 `GetObject(0x1009)` 下载；`0xC102` 由"必须"降级为"可选提前/收尾信号"。
   依据：`NikonBase.cs:2333-2367`（对象事件独立驱动取回与传输），`:2369-2375`（完成事件只发
   `OnCaptureCompleted`），应用侧仅 `BraketingClass.cs:143` 与 `Program.cs:544` 消费后者。
2. **保留"必须存在与本操作关联的对象"作为提交条件**。这里要**比库更严**：库对无主对象也照传，我们不能把
   来源不明的对象当交付。
3. **截止时间形状**：库不给拍摄截止值，只给轮询节奏（100 ms，`NikonBase.cs:348`）与秒级等待用例
   （1500 / 2000 / 5000 / 30000 ms，见第 3 节）。因此建议在既有 75 s owner deadline 内，为"对象到来"设一个
   **秒级**窗口（量级与库的 5 s / 30 s 用例一致），而不是靠把等待缩到亚秒去"修复"。这是与库量级一致的工程
   选择，不是库的证明。
4. **`0x4006` 只作诊断**：库把它当"刷新属性"（`NikonBase.cs:2326-2328`），绝不作为完成或提交证据。建议把
   实测到的 `0xD1B4` 记为独立诊断字段（例如"已观测一次曝光"），**不得**用它代替 `ExposureEnded`，也不得由它
   提交文件。
5. **空闲 `0xC101` 保持 fail-closed**：继续拒绝把无法解释的 RAM 对象事件当交付证据，并保留现场。库之所以
   "接受"它，是因为它有人值守且 `ReleaseResurce` 对 Nikon 是空操作（`BaseCameraDevice.cs:454-457`），对象会
   一直留在机身；本项目无人值守，必须在事务边界上证明来源。唯一建议的措辞修正：日志要写清"收到的是一次
   `0xC101` RAM 对象事件、参数未经验证"，不要让它读起来像 `0x400D`/曝光结束语义。
6. **继续保持 fail-closed 的清单**：超时即隔离且清理未确认不得报成功（`actor.rs:271-276`）；卡路径不得接受
   RAM 证据（`card_capture.rs:689-694`）；RAM 路径不得接受卡片证据（`capture.rs:292-294`）；`0xC104` 仍只当噪音
   （`capture.rs:310`，对应 `NikonBase.cs:2376-2377`）；不引入 `0x101B`，也不因 `NikonBase.cs:59-61` 存在私有码
   就向其未广告的能力试发。
7. **顺带修正表述（可只改记录，不改代码）**：本项目 `lastCaptureEventDiagnostic.operationCode` 来自事件轮询
   （`nikon.rs:1240-1273`，常量 `:28`），实机记录第 7 节把它写在"操作码"栏容易被读成拍摄操作。下次记录建议
   分开显示"捕获操作码（`0x90C0` 或 `0x100E`）"与"事件轮询操作码（`0x90C7`）"。

## 8 参考实现没有告诉我们的（避免过度声称）

1. 真实 D7100 在 `0xD10B=1` / `0x90C0` accepted 之后的事件集合与时序。
2. `0xC101` 参数的形式保证（是否恒为句柄、是否可能带别的高位语义）。
3. `0x4006` 参数 `0xD1B4` 的含义与触发条件（全树无此码）。
4. "曝光结束"的权威事件到底是哪一个：库不以此判失败，也没有超时，因此无法从库推出判据。
5. 什么情况下一次曝光只发 `0x4006` 而不发 `0xC101`/`0xC102`（我们实机遇到的就是这种）。
6. 存卡路径（`0x100E`）在 D7100 上究竟发 `0x4002` 还是只发 `0x400D`、以及二者顺序（本项目尚未实测）。
7. 机身 RAM 对象是否会自行过期/被覆盖，以及 `0xC101` 句柄复用的边界（库假定可复用；本项目
   `capture.rs:336-338` 已按"未解决的操作毒化会话"处理，这仍是我们的推断）。
8. WPD 层对 Nikon 私有事件的投递保证：库只显式处理 `WPD_EVENT_OBJECT_ADDED` 与 `DEVICE_REMOVED`
   （`PortableDeviceEventCallback.cs:46-58`、`BaseMTPCamera.cs:160-170`、`NikonBase.cs:1109-1124`），其余一律靠
   `0x90C7` 轮询，因此库不能证明"事件一定会主动到达"。
9. NEF 的完整性判据（库只按扩展名，`FileItem.cs:243-250`），以及卡对象内容与 WPD 传输边界的任何数值保证。
10. 任何超时数值本身：库没有拍摄截止常量，只有轮询节奏与秒级 `WaitForCamera` 用例。

## 9 快速索引（opcode / 事件 / 属性）

| 名称 | 值 | 参考实现定义位置 | 本项目对应 |
| --- | --- | --- | --- |
| `InitiateCapture` | `0x100E` | `CameraControl.Devices/BaseMTPCamera.cs:82` | `INITIATE_CAPTURE`（`nikon.rs:17`） |
| `InitiateCaptureRecInSdram` | `0x90C0` | `Nikon/NikonBase.cs:61` | `CAPTURE_RAM`（`nikon.rs:14`） |
| `AfAndCaptureRecInSdram` | `0x90CB` | `Nikon/NikonBase.cs:60` | 本项目不用 |
| `InitiateCaptureRecInMedia` | `0x9207` | `Nikon/NikonBase.cs:59` | 本项目不用（仅参考实现的 Live View 单张） |
| `GetEvent` | `0x90C7` | `Nikon/NikonBase.cs:66` | `GET_EVENT`（`nikon.rs:28`） |
| `DeviceReady` | `0x90C8` | `Nikon/NikonBase.cs:68` | `DEVICE_READY`（`nikon.rs:29`） |
| `GetObjectInfo` | `0x1008` | `BaseMTPCamera.cs:79` | `GET_OBJECT_INFO` |
| `GetObject` | `0x1009` | `BaseMTPCamera.cs:77` | `GET_OBJECT` |
| `GetPartialObject` | `0x101B` | **参考实现未使用** | 本项目曾计划；参考实现靠 WPD 分块读 |
| `ObjectAdded` | `0x4002` | `BaseMTPCamera.cs:86` | `OBJECT_ADDED`（`nikon.rs:22`） |
| `ObjectAddedInSdram` | `0xC101` | `BaseMTPCamera.cs:87` | `OBJECT_RAM`（`nikon.rs:35`） |
| `CaptureComplete` | `0x400D` | `Nikon/NikonBase.cs:110` | `CAPTURE_COMPLETE_CARD`（`nikon.rs:26`） |
| `CaptureCompleteRecInSdram` | `0xC102` | `Nikon/NikonBase.cs:111` | `CAPTURE_COMPLETE_RAM`（`nikon.rs:36`） |
| `DevicePropChanged` / `StoreFull` / `ObsoleteEvent` | `0x4006` / `0x400A` / `0xC104` | `Nikon/NikonBase.cs:108-112` | `PROPERTY_CHANGED` / `STORE_FULL` / 噪音 |
| `RecordingMedia` | `0xD10B` | `Nikon/NikonBase.cs:96`（写入 `:1103-1105`） | `RECORDING_MEDIA`（`nikon.rs:49`） |
| 连拍计数 | `0xD1B4` | **参考实现未定义** | `continuousShootingCount`（`nikon.rs:98-102`） |

