# D7100 原生相机能力与依赖矩阵

本表对照活动 Rust、Tauri、React 源码与本地 MIT digiCamControl 的 `D7100 → D600Base → D800 → NikonBase → BaseMTPCamera` 继承链。它描述当前实现子集和协议依据。指定 P0包已有真实独立 NEF及 UI Live View分项 PASS（见下文证据范围）；新固定策略/独立自动模式、镜头驱动及完整辐射联动仍按各自证据验收，不能把目录覆盖或离线测试视为全部相机能力认证。

## 真值和职责

- 相机型号、命令列表、属性列表及本次 descriptor/readback 决定支持范围；已知代码和标签只是目录，不能制造设备支持。未声明属性保留禁用占位，不试发私有操作。
- `supportedValues`、`range`、`dataType`、`nativeWritable` 保留设备证据；`supported`、`effective`、`writable`、`disabledReason`、`options` 表达当前能力及依赖。实际值、草稿目标、写入成功、读回确认分别保存。
- 所有相机通信和 COM 对象归相机 owner。领域入口持有扫描/手动占用和跨设备裁决；相机不调用射线或转台，独立拍摄和 Live View 不自动开束。
- 正式采集仍限定 D7100 主机 RAM NEF。能配置某种摄影参数，不代表当前后端能用该模式捕获。未知、超时、失联、取消清理未确认均不能作为成功或安全证据。

活动实现入口：

| 入口 | 作用 |
| --- | --- |
| [nikon.rs](../../crates/ct-engine/src/devices/camera/nikon.rs) 的 `NIKON_PROPERTIES`、`read_properties`、`apply`、`profile_readback` | 属性目录、实际描述、串行写入读回、冻结采集条件 |
| [properties.rs](../../crates/ct-engine/src/devices/camera/properties.rs) 的 `dependency_reason`、`option_reason`、`refresh_property_controls`、`validate_control_values` | 依赖和原生选项策略，写入前后重新核对 |
| [types.rs](../../crates/ct-engine/src/devices/camera/types.rs) | 属性、选项、能力、健康和运行诊断 DTO |
| [actor.rs](../../crates/ct-engine/src/devices/camera/actor.rs)、[wpd.rs](../../crates/ct-engine/src/devices/camera/wpd.rs) | 消息代理、唯一 COM owner、取消代次和有界传输 |
| [device_controls.rs](../../crates/ct-engine/src/device_controls.rs) | 手动占用、独立拍摄目录、Stop 和领域命令 |

## 依据索引

下表的 `NB`、`D6`、`D8`、`D71`、`MTP` 是本地 MIT 源码的行号和方法名；这些文件是参考实现，不是运行时依赖。

- NB：[NikonBase.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/Nikon/NikonBase.cs)。属性常量 73–106；基础标签 219–267；高级目录 `AddAditionalProps` 505–534。
- D6：[NikonD600Base.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/Nikon/NikonD600Base.cs)。继承 D800：45；D7100 沿用的参数和 Live View 覆盖。
- D8：[NikonD800.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/Nikon/NikonD800.cs)。继承 NikonBase：42；ISO 与 Auto ISO 标签、能力和 Bulb 实现。
- D71：[NikonD7100.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/Nikon/NikonD7100.cs)。继承 D600Base：41；Live View 布局 43–72；电影光圈禁用 75–78；裁切覆盖 81–94。
- MTP：[BaseMTPCamera.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/BaseMTPCamera.cs)。标准传输与大端头字段读取 `ToInt16` 620–623。
- UM：[Nikon D7100 官方英文说明书](https://download.nikonimglib.com/archive7/VrdxE00Uyz9906XH31L97Xwuu922/D7100_%28En%2904.pdf)。下文使用印刷页码；PDF 页码为印刷页码加 28。
- GP：[libgphoto2 PTP定义](https://github.com/gphoto/libgphoto2/blob/master/camlibs/ptp2/ptp.h)及[属性配置映射](https://github.com/gphoto/libgphoto2/blob/master/camlibs/ptp2/config.c)确认D0C0为Bracketing enable（UINT8、OFF0/ON1）；只作协议事实核对，不复制运行代码。D078及D0C1–C6的类型/步长/程序/计数不是active OFF证据。NB 95/2089的D1B1是有符号曝光表读数，非BKT；D1B0–4不能凭零值推断BKT OFF。
- UM 136–139：M模式包围仍改变快门，single release每触发一张但序列继续，ADL菜单OFF不取消ADL包围；UM 141–144：多重曝光将2/3次曝光合成一照片并可能使用Auto gain；UM 115：HDR不能记录NEF。[CCP2 Mechanical](https://nikonimglib.com/ccp2/onlinehelp/en/08_camera_controls_04.html)支持远程BKT，[CCP2 Window](https://nikonimglib.com/ccp2/onlinehelp/en/07_ccp_window_01.html)说明多重曝光仅机身操作且状态不显示，这不证明90C0/RAM免疫。原审计[FAIL记录](../../tmp/tests/camera/20261005-fixed-automation-audit/result.json)保留；审计当时关于未知Multi阻单拍的建议已由当前单host输出契约修正。

## 参数覆盖

当前 `NIKON_PROPERTIES`目录实际包含34个 D7100摄影/采集参数候选，新增 `bracketing`；另有单独的只读运行诊断目录。多重曝光是健康证据中的物理适用/不可观测状态，没有猜测出的可写参数。这是已实现子集，不是“全部厂商功能完备”的声明。每项只在设备实际声明且成功描述后可读；可写还要求设备写权限、当前依赖、已确认实际值和未被占用。未声明的 TIFF、未压缩 RAW、自然光自动、亮部重点测光等通用选项显示为不支持。

当前新源码另有2项独立LV目录，不能加到摄影34项或预设/profile里：`CameraHealth.liveViewControls={sessionId|null,properties{cameraZoom,focusMode}}`。2026-10-06 已在本轮 rc3 候选上取得实机证据：LV Start 生成独立 32hex nonce、`cameraZoom`(0xD1A3)/`focusMode`(0xD061) 描述与鲜读、`cameraZoom` 写入+读回成功、约 32 秒仅内存传输、停止后旧 nonce 报 `STALE_LIVE_VIEW_SESSION`；`focusMode` 因当前物理 MF 开关锁定而只读，写入路径仍未实机验证。不改写[已签rc2独立验收](../workstation-restructure-rc2-acceptance-2026-10-05.md)，完整结果见[后端重验证记录](../backend-revalidation-2026-10-06.md)。

| 参数键 / 代码 | 标签或原生值依据 | 当前依赖、限制和处理 | MIT / 官方依据 |
| --- | --- | --- | --- |
| `exposureMode` / `500E` | 1 M、2 P、3 A、4 S；其他场景值只认实际枚举 | 描述只读时禁写，不自动进入 Host Mode 夺取控制；CT 要求 M=1 | NB 219–234、1315–1381 |
| `shutter` / `D100` 或 `500D` | D100 为高16位分子/低16位分母的秒分数；500D 为0.1 ms单位 | M/S 才手动有效；严格匹配定时档位，不四舍五入；Bulb/Time/Flash Sync 禁用 | D6 310–403；NB 1345–1372；原生 `shutter_ms` |
| `aperture` / `5007` | f-number ×100，档位来自镜头实际描述 | M/A 才手动有效；镜头变化需重读；不把电影光圈视作可写 | NB 1384–1455；D71 75–78 |
| `iso` / `500F` | D6 按 UINT16 descriptor 读取；Hi 标签沿用 D8 的表 | Auto ISO 开启时仍可设基准，不把基准值当曝光实测 ISO；CT 要求 Auto ISO 关闭 | D6 233–298；D8 47–81；UM 81 |
| `autoIso` / `D054` | 0关闭、1开启 | 影响上限参数；不凭 Auto ISO 推断 ISO 不可写 | NB 511、982–989；UM 81 |
| `autoIsoHighLimit` / `D183` | D8 的0–25索引表，不是ISO整数 | 仅 Auto ISO=1 有效；只认设备允许的索引/范围，未知索引保留原生值 | NB 754–779；D8 84–112 |
| `rawFormat` / `5004` | 0/1/2 JPEG Basic/Normal/Fine，4 NEF，5/6/7 NEF+JPEG；3 TIFF是通用候选 | 改变 RAW/JPEG/HDR 依赖；CT捕获只接受 NEF=4 | NB 250–259、1579–1612；`profile_readback` |
| `rawBitDepth` / `D149` | 0=12 bit、1=14 bit | 仅包含 NEF 的格式4–7有效 | NB 878–892；UM 67 |
| `rawCompression` / `D016` | 0无损压缩、1压缩；2未压缩仅设备声明才支持 | 仅格式4–7有效；D7100说明书只列前两种 | NB 963–978；UM 67 |
| `crop` / `D030` | D7100覆盖：0 DX、1 1.3x | 不继承通用FX/5:4标签；变更后重读图像尺寸等描述 | D71 81–94 |
| `imageSize` / `5003` | PTP字符串，保留设备返回的真实尺寸 | 控制JPEG；纯NEF4无效；NEF+JPEG5–7仍可按描述设置JPEG尺寸；裁切改变可选尺寸 | NB 933–960；UM 67–68 |
| `whiteBalance` / `5005` | 2自动、4晴天、5荧光灯、6白炽灯、7闪光灯、8010阴天、8011阴影、8012色温、8013预设 | 先写白平衡并重读，再应用子参数；扫描 profile 5只接受已确认固定档位，拒2及已声明的自然光自动8016 | NB 236–247、1459–1499 |
| `whiteBalanceFluorescent` / `D14F` | 0–6为七种光源标签 | 仅白平衡=5有效 | NB 806–827、1496–1497 |
| `whiteBalanceTemperature` / `D01E` | NB 的0–30色温索引标签；实际编码以 descriptor 为准 | 仅白平衡=8012有效；不将索引直接当Kelvin写入 | NB 830–874、1498–1499 |
| `exposureCompensation` / `5010` | 原生有符号值，显示EV | 保留实际描述；M模式下不伪称它一定改变快门/光圈或射线 | NB 1502–1575 |
| `meteringMode` / `500B` | 2中央重点、3矩阵、4点测光；8010通用候选 | 中央重点范围仅在本模式有效 | NB 262–267、1615–1643 |
| `centerWeightedArea` / `D059` | D7100：6、8、10、13 mm、全画面平均 | 测光模式=2；非CPU镜头的有效直径限制仍需镜头证据；NB重复10mm及通用12mm标签不能照搬 | NB 734–750；UM 236 |
| `driveMode` / `5013` | D6：1单张、2CH、8010CL、8011自拍、8012抬镜、8016安静、8017遥控 | 设备只读时禁写；CT要求单张1；其余设置不等于支持对应采集事务 | D6 190–209 |
| `burstNumber` / `5018` | UINT16，真实范围由 descriptor 决定 | 扫描和独立主机单拍均要求1；不能误用D10B作数量 | NB 1053–1069；`prepare_host_capture` |
| `recordingMedia` / `D10B` | 0存储卡、1SDRAM | 明确 Apply选择RAM并读回1；预检/开始/恢复不隐式修复；禁止卡回退 | NB 96、1099–1105 |
| `focusMode` / `D161` | 0 AF-S、1 AF-C、2 AF-A、3物理MF、4软件MF | 物理MF3锁定选择器；CT接受3/4，拒绝AF；不保存假焦点位置 | NB 1646–1694；D6 471–475 |
| `exposureDelay` / `D06A` | D6覆盖：0=3秒、1=2秒、2=1秒、3关闭 | CT要求3；不能沿用通用0关闭的假设 | D6 213–228 |
| `longExposureNoiseReduction` / `D06B` | 0关闭、1开启 | CT要求0，避免确定性曝光/处理事务改变 | NB 513、982–989；`profile_readback` |
| `highIsoNoiseReduction` / `D070` | 0关闭、1低、2标准、3高 | 只认实际枚举；冻结已支持、已确认的采集参数 | NB 1074–1083 |
| `colorSpace` / `D032` | 0 sRGB、1 Adobe RGB | 对RAW文件的影响不能等同于JPEG处理；保留原生读回 | NB 789–802 |
| `pictureControl` / `D200` | 1–6基础控制；201–209自定义候选 | 仅descriptor支持才启用；自定义名称/完整编辑尚未读取，显示名称未知 | NB 896–929 |
| `flashMode` / `500C` | 2禁止、4红眼、8010前帘、8011慢速、8012后帘、8013红眼慢速 | 参数设置不证明物理闪光灯收起/未出光；不参与射线健康推断 | NB 1014–1027 |
| `flashCompensation` / `D124` | 有符号原生值，NB按六分之一EV显示 | 实际类型/范围决定写入；不借用long setter的字节数 | NB 712–730 |
| `flashSyncSpeed` / `D074` | 0=1/320自动FP、1=1/250自动FP、2–8=1/250至1/60 | 以设备枚举筛选；不把这些设置当受控曝光结束证据 | NB 590–610 |
| `bracketing` / `D0C0` | UINT8，0关闭、1开启 | 本次广告、严格descriptor及鲜读0才证明OFF；未广告、错类型或读取失败阻扫描，不盲读/写；错类型不提供OFF标签或no-op成功 | GP `Bracketing`；UM 136–139 |
| `activeDLighting` / `D14E` | 0极高、1高、2标准、3低、4关闭、5自动 | D7100物理适用，profile5要求OFF=4并冻结；未广告或读取失败仍UNKNOWN并阻扫描，不作不适用 | NB 672–689；UM 113、139 |
| `hdrMode` / `D130` | MIT映射0关闭、1单张、2系列 | NEF/NEF+JPEG不允许开启，OFF仍可用；扫描已广告时必须鲜读OFF=0，ON/error/矛盾拒绝；仅未广告时可用确认NEF=4的互斥证明 | NB 653–667；UM 115；`profile_readback` |
| `hdrExposureDifference` / `D131` | 通用MIT候选0自动、1/2/3 EV | D7100用户手册只列Mode/Strength，不能假定另有此属性；未声明禁用；开启HDR且JPEG格式才有效 | NB 634–649；UM 115–116 |
| `hdrSmoothing` / `D132` | 0高、1标准、2低、3自动、4极高；D7100显示为HDR强度 | HDR开启且JPEG格式才有效；标签来自继承映射，用户手册不提供PTP数字认证 | NB 614–630；UM 116 |

### 写入和依赖刷新

`apply` 先校验整个请求，再按曝光模式、Auto ISO、格式、裁切、白平衡、测光、HDR等父参数排序。父参数写入且读回后重读 descriptor，随后再次校验子参数，不能用预计的新枚举代替真实设备描述。原先只读的子参数必须先显式应用父参数并重新读取后才能写。

相同且已确认的原生值是无写入的 no-op。部分失败保留实际值或未知状态、请求目标和失败原因，不虚构回滚。UI预设只保存已确认的可应用摄影参数；原生设备状态和运行诊断不进入通用可写预设。`camera_apply_properties` 的入队前拒绝按 `PropertyRejection` 分码返回（`LIVE_VIEW_APPLY_FORBIDDEN`、`INVALID_CAMERA_PARAMETER_COUNT`、`CAMERA_PROPERTY_UNDESCRIBED`、`CAMERA_FOCUS_MODE_LOCKED`、`CAMERA_PROPERTY_WRITE_UNSUPPORTED`、`CAMERA_PROPERTY_UNSUPPORTED_VALUE`、`CAMERA_PROPERTY_DEPENDENCY_UNMET`、`CAMERA_PARAMETER_SESSION_UNAVAILABLE`），不再折叠为单一错误码；`preset_load` 仍使用粗粒度的 `UNSUPPORTED_CAMERA_PROPERTIES`。

## 操作能力

| 功能 | 当前覆盖 | 依据和边界 |
| --- | --- | --- |
| 显式发现、选择、连接、断开 | 已实现 | `actor`与`wpd`；无参数连接拒绝隐式选择；未知型号仅标准已描述能力。无 `deviceId` 的 `camera_connect`（含 `retry_device{device:"camera"}`）只做发现，operation kind 为 `camera_discover`，受理记录不声明已连接；有明确 `deviceId` 才为 `connect`；清理未确认时需显式 `acknowledgeUnconfirmedCleanup` |
| DeviceInfo、vendor属性扩展 | 已实现 | `nikon::connect`仅在D7100身份绑定且90CA被声明时读取，保留原始/有效列表和诊断哈希 |
| 单拍RAM NEF、CT捕获 | 事务已实现；真实扫描因Multi UNKNOWN阻塞；P0独立照片已分项验证 | NB `CapturePhotoNoAf` 1890–1941未证明BKT/Multi免疫；固定扫描与独立摄影有不同admission；两者均校验单host NEF、LV OFF、session/operation、C101/C102、完整文件/hash和原子不覆盖。一个host文件不证明一次物理曝光；新的独立自动模式与完整CT联动仍未由P0证明。未就绪（`captureReady`/`independent_readback` 未通过）时 `camera_capture` 在入队前拒绝为 `CAMERA_CAPTURE_UNQUALIFIED`，不置 Fault、不断开会话 |
| 独立JPEG/TIFF、RAW+JPEG双对象采集 | 禁用 | 当前对象解析与提交仅接受NEF，缺少多对象关联和格式确认事务；更改格式不会扩展捕获能力 |
| AF | 已实现候选；新增LV资格仅离线 | NB `AutoFocus` 1853–1858、90C1；实际操作列表及DeviceReady门禁；LV需fresh lensSort=1、照片selector=0、D061 mode0；物理MF/AF-F/未知镜头资格禁用，不以lensSort单项推断motor。能力未广告时 `camera_autofocus` 入队前拒绝为 `CAMERA_AUTOFOCUS_UNQUALIFIED` |
| 镜头步进 | 已实现候选；新增LV资格仅离线 | NB 1799–1807、9204；LV开启、步数有界、fresh lensSort1/selector0/D061 mode0或4及实际操作支持；AF-F2禁止有界motor、hardMF3禁写motor；不将脉冲计数当绝对焦点。LV未激活或 `focusStep` 未广告时 `camera_focus` 入队前拒绝为 `CAMERA_FOCUS_UNQUALIFIED` |
| Live View启停 | 已实现；P0流与EOF清理已分项验证 | NB 1697–1724、9201/9202，原生读取D1A2确认；开启期间前后端拒绝拍摄，不自动停后拍再恢复 |
| Live View网格、准线、显示缩放 | 已实现 | React纯显示；CSS显示缩放不改变相机D1A3，也不制造拍摄/对焦命令 |
| 相机端LV缩放D1A3、LV焦点D061 | 新增独立2项LV目录；读取与 `cameraZoom` 写入已实机验证，`focusMode` 写入仍未验证 | NB 1126–1147、1646–1688；`cameraZoom`/`focusMode`严格UINT8、当前广告/鲜读/写后读回，唯两key；typed read/configure绑定当前LV nonce。2026-10-06 实机：两项描述与鲜读成功、`cameraZoom` configure 完成并读回、旧 nonce 拒绝为 `STALE_LIVE_VIEW_SESSION`；物理MF3下 `focusMode` 只读，无写入实机证据（见[后端重验证记录](../backend-revalidation-2026-10-06.md) T5）。D061只允许有据的目标0 AF-S/2 AF-F/4软件MF，物理MF3只读；不混同D161或UI缩放 |
| 点击画面对焦区域9205、焦点框/旋转/水平元数据 | 尚未覆盖 | NB 1943–1948；D71 43–72提供头字段；需严格坐标范围、当前帧/会话和已声明操作，不能凭CSS像素直接发坐标 |
| Bulb/Time、电影录制、电影光圈 | 禁用 | D8 199–247存在不同开始/终止链；D71 75–78禁用电影光圈；当前没有这些模式的有界捕获/结束/清理事务 |
| Host Mode/Lock、Application Mode | 未作为普通参数开放 | NB 416–431、D8 228–247；初始化没有证明必须自动90C2(1)，不可为解除只读而隐式改变模式 |
| 卡格式化、删除、设备复位 | 未开放 | 参考实现有此类操作；它们不属于主机NEF采集所需能力，不能作为错误恢复自动执行 |
| 电池与LV禁止条件 | 已实现按声明的只读原始诊断 | `RUNTIME_DIAGNOSTICS`含5001电池UINT8及D1A4禁止条件UINT32；不混入摄影profile，不把未知位/原始状态当整体健康保证 |
| 镜头类型、卡槽及LV驱动资格 | 已新增只读运行证据及保守LV资格；仅离线 | LensSort D0E1：NB554–567；ActiveSlot D1F2：NB571–586；D1A6照片/电影selector及D061 LV焦点已入专用模型。D0E1/D1A6/D1F2只按本次广告读取，缺席UNKNOWN，保留nativeWritable但不开放setter；D1A4仍为原始诊断，不声称全部镜头/禁止条件已完备 |
| D1B2显示诊断 | 已实现原始读回 | `runtime_properties`保留实际数字及semanticStatus=unknown；不能仅由0/1映射判断真实故障/正常，也不能由LCD PC字样判断健康 |

## Live View全链与不落盘边界

| 阶段 | 当前源码事实 |
| --- | --- |
| 相机传输 | `actor`的owner每100ms最多拉一次9203，单次共享2秒deadline；WPD返回有界内存字节；不调用拍摄或对象保存 |
| D7100解析 | `live_view::d7100_frame`要求384字节头，8/10偏移的大端宽高，JPEG完整性和尺寸一致；依据D71 45–46与MTP620–623 |
| 引擎帧槽 | `LatestFrameSlot`覆盖单一最新帧，1秒新鲜度；撤销会话清除帧和descriptor，无历史帧队列 |
| 引擎到Tauri | 随机当前用户ACL named pipe，64字节version/session/sequence/codec/length envelope；有界写入和32字节精确session/sequence ACK；pipe HANDLE不是磁盘文件 |
| Tauri | `camera_forwarder::read_frame`在同一250ms deadline内重试暂时missing/busy、完整读取、验证和ACK；`camera_live_view_frame`只接收afterSeq，使用引擎端点，返回前再核会话 |
| Renderer | `cameraControls::validateLiveFrame`校验会话、单调序号、尺寸和最多4MiB JPEG；`liveViewDataUrl`使用有界内存data URL，避免Blob/objectURL的Chromium磁盘spill路径 |
| 展示与释放 | `MemoryLiveViewFeed`只容纳新帧与待退休旧帧，解码和DOM提交后替换；null/暂时错误保留上张已解码画面并显示STALE；Stop/断开/会话变化清除引用，迟到工作不得恢复预览 |

源文件：[live_view.rs](../../crates/ct-engine/src/devices/camera/live_view.rs)、[camera_forwarder.rs](../../src-tauri/src/camera_forwarder.rs)、[Tauri入口](../../src-tauri/src/main.rs)、[cameraControls.ts](../../src/platform/cameraControls.ts)、[cameraLiveView.ts](../../src/platform/cameraLiveView.ts)、[WorkstationMedia.tsx](../../src/WorkstationMedia.tsx)、[CameraPanel.tsx](../../src/CameraPanel.tsx)。

这条预览链不调用应用的JPEG临时文件、图像导出、扫描manifest或JSONL图像字段。NEF文件写入仅存在于明确拍摄事务；named pipe借用Windows文件句柄接口不等于图像落盘。“不落盘”指应用预览实现不创建帧文件；不把操作系统分页、崩溃转储或外部截图行为纳入此保证。

## 仍需保留的限制

1. **P0独立NEF/LV分项PASS，全面认证仍BLOCKED。** 指定0070包已有新NEF、C101/C102、完整文件/hash、参数恢复及30秒UI LV证据；这不覆盖所有摄影选项、自动摄影模式、镜头驱动、持续长时LV或完整CT联动。新 profile 5门禁及照片登记实现另有离线回归，不能由旧包证据直接升级为当前候选硬件认证。
2. **LV资格已新增，实机仍待验。** 当前源码把D061专用LV控制、D1A6/LensSort/ActiveSlot只读证据纳入有据的保守判定，D1A4保留原始诊断；不是全部镜头和禁止条件的完备覆盖。两项LV控制的会话绑定、只读描述与拒绝路径已在本轮实机确认（T5/T6，见[后端重验证记录](../backend-revalidation-2026-10-06.md)）；成功 setter 写回的实机证据以本轮验收资料为准，fresh 镜头驱动资格因相机已从 Windows 枚举消失仍需重新连接后复核，旧rc2实机结果不认证新setter或nonce契约。
3. **HDR配置不等于HDR捕获。** NEF互斥已处理；参考手册还限制闪光、包围、多重曝光、电影和特殊快门。当前未读取全部物理条件，不提供HDR采集事务或自动化健康保证。
4. **自定义Picture Control和点击AF尚未完成。** 安全扩展顺序为已声明只读名称/头字段、坐标验证、显式操作能力，再实现控制；不能自动写未知私有扩展或扩大扫描profile。
5. **原rc1固定自动化缺口已确认，当前真实扫描仍BLOCKED。** 原策略没有证明机内包围关闭，并会跳过未广告/读取失败的ADL；旧P0、RAM单文件、burst1及C102都不能豁免该缺口。当前新增C0严格OFF证明、物理适用ADL及HDR原生/NEF互斥证明，并升级profile5。多重曝光没有可靠PTP编码，真实D7100返回物理适用、不可观测、UNKNOWN，阻止扫描；不猜私有码、不自动清除、不用人工确认当机器OFF。用户关于另设人工验收策略的问题仍待答，当前无人工覆盖。独立照片保留单个完整host NEF输出契约并明确UNKNOWN诊断，不声称一次物理曝光。
6. **参考实现的静默错误和标签不直接沿用。** MTP `SetProperty` 497–615会捕获并记录部分错误；NB尺寸setter会跳过所有含RAW格式，而D7100官方规则只说明NEF尺寸固定；NB中央重点标签重复10mm。生产端保持严格编码、读回和准确型号覆盖。

## 固定扫描、独立摄影与预览访问

### 独立LV控制会话

每次inactive→Start建立新32hex nonce，区别于稳定camera connection/capture ID会话。`camera_live_view_read {sessionId}`鲜读两项LV descriptor/value；`camera_live_view_configure {sessionId,controls}`只收cameraZoom/focusMode，实际广告、严格UINT8、fresh权限/值及写后读回均必要。帧pipe/metadata与controls会话一致，Stop/隔离/退休撤销，旧请求/迟到结果不跨新LV。普通Apply仍拒绝LV，LV控制不进入摄影presets或固定profile，不提供任意setter透传。

Advanced的preset Compare只把supported/readable/confirmed、无error的actual作为证据；未知或保留旧值即使相等也显示UNKNOWN。WorkstationDialog改为mount焦点和最新callback refs，持续snapshot不抢输入焦点。[163257实际组件浏览器回归](../../tmp/tests/frontend/20261005-163257-dialog-focus-fix/result.json)PASS且明确UI_ONLY_NO_HARDWARE。摄影34项、LV2项及运行诊断分别展示；9205区域AF、自定义Picture Control编辑、Bulb/Time、视频/JPEG/多对象事务仍未实现。

相机核心参数只有明确 Apply才串行写入并读回；`prepare_scan`只原子提交任务与完整几何，用确认快门派生曝光。预检、开始、恢复和扫描事务鲜读比较，不自动停LV、不写快门/RAM/burst。当前清单schema3中的profile版本为5，冻结WB、C0/ADL/HDR证明及多重曝光证据，规范hash纳入固定策略版本2。旧profile1–4原hash和只读重构保持，拒绝追加投影。HDR只有未广告时可用已确认NEF=4的官方互斥证明；已广告的ON、错误或矛盾读回仍拒绝。

独立摄影允许已确认的M/P/A/S、自动ISO/WB及支持的AF模式；不会隐式触发AF。只有实际自动模式决定的ISO/快门/光圈输出可变化；RAM介质、NEF格式、单host输出数量、会话及C102/文件证据仍严格校验。UNKNOWN多重曝光明确诊断并不单独永久禁用普通独立照片：此入口保证一个验证完整的主机NEF，未承诺一次物理曝光。90C0、一C101、关联可靠C102、完整长度/hash、超时、多对象拒绝和取消门禁保持。Bulb/Time、JPEG和多对象采集不会因此开放。

正常 `camera_capture` 从唯一 `Parameters.savePath`根目录派生 `CameraCaptures`子目录，renderer不能换成任意路径；显式目录仅用于诊断 `camera_test_capture`且仍检查已有扫描清单、目录别名和不覆盖。完整文件提交后登记 capture ID、会话、operation ID、bytes及hash。照片预览只接收 capture ID，Tauri再次核对登记、大小/hash，复用 NEF内嵌 JPEG；无内嵌 JPEG时报预览不可用，不另写 JPEG临时文件。浏览器模拟只报告 NO HARDWARE，不登记或制造真实照片。

## 验证边界

本机P0证据：[0070 result](../../tmp/tests/desktop/20261005-004700-0070-nef/result.json)、[新NEF与C101/C102](../../tmp/tests/desktop/20261005-004700-0070-nef/captured-nef.json)。EXE SHA-256 `0070808c9fb206d4745ba039bcd1eb4c13d19e0a12fef3d164be93797637862c`；engine SHA-256 `f8f074d75518e4cf0304e630c683ba643e8f5f23be882b00687ee03e3b191ffa`。新文件23,476,423字节，SHA-256 `b20e4b1b15e1c97541befc9dbe482f15ff7dabd2a9910304527b6b0028e7c2f9`；参数恢复PASS；UI LV 30.1096秒、首帧后missing=0、实时像素文件=0，期间存在STALE/首次暂无帧记录。App/engine均strict exit0；关闭前独立 LV-stop终态快照未保留，不能据此扩大该单项验收。测试明确无射线/Nano运动、fullLinkedHardwareCertified=false。上述tmp本机过程证据不进入Git。

当前profile5领域策略另有[offline PASS](../../tmp/tests/ct-engine/20261005-profile5-final/result.json)（247库+2进程测试），浏览器合同为[simulation-only PASS](../../tmp/tests/camera/20261005-fixed-automation-audit/preview-final-result.json)（38项）。离线门禁和参考协议对照不能替代当前新候选的自动摄影、驱动资格、联动安全或重构精度实机验收；真实扫描仍因多重曝光UNKNOWN而BLOCKED。原rc1及已签rc2验收保留为各自产物的独立历史记录；当前源码已构建rc3 host并记录候选哈希与真实Tauri IPC、相机/Nano分项证据（见[后端重验证记录](../backend-revalidation-2026-10-06.md)），当前候选独立NEF T7与F-01修复实机验收已PASS（重验证§5z/§5k–§5q）；F-12原WPD失败本轮未复现且根因未确证，HOME/MOVE、射线、完整扫描与软件更新维护锁仍未完成，不把旧P0/RAM one-file当固定自动化豁免。本文只更新文档，不发设备命令；适用文档门禁为 `npm.cmd run module-map:check`。
