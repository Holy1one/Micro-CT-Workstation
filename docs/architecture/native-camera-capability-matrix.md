# D7100 原生相机能力与依赖矩阵

本表对照活动 Rust、Tauri、React 源码与本地 MIT digiCamControl 的 `D7100 → D600Base → D800 → NikonBase → BaseMTPCamera` 继承链。它描述当前实现子集和协议依据。指定 P0包已有真实独立 NEF及 UI Live View分项 PASS（见下文证据范围）；新固定策略/独立自动模式、镜头驱动及完整辐射联动仍按各自证据验收，不能把目录覆盖或离线测试视为全部相机能力认证。

## 真值和职责

- 相机型号、命令列表、属性列表及本次 descriptor/readback 决定支持范围；已知代码和标签只是目录，不能制造设备支持。未声明属性保留禁用占位，不试发私有操作。
- `supportedValues`、`range`、`dataType`、`nativeWritable` 保留设备证据；`supported`、`effective`、`writable`、`disabledReason`、`options` 表达当前能力及依赖。实际值、草稿目标、写入成功、读回确认分别保存。
- 所有相机通信和 COM 对象归相机 owner。领域入口持有扫描/手动占用和跨设备裁决；相机不调用射线或转台，独立拍摄和 Live View 不自动开束。
- 正式采集限定 D7100 单 NEF，默认 SD 存卡，显式选择并 Apply 主机目标后可用 RAM 路径。能配置某种摄影参数，不代表当前后端能用该模式捕获。未知、超时、失联、取消清理未确认均不能作为成功或安全证据。

活动实现入口：

| 入口 | 作用 |
| --- | --- |
| [nikon.rs](../../backend/ct-engine/src/devices/camera/nikon.rs) 的 `NIKON_PROPERTIES`、`read_properties`、`apply`、`profile_readback` | 属性目录、实际描述、串行写入读回、冻结采集条件 |
| [properties.rs](../../backend/ct-engine/src/devices/camera/properties.rs) 的 `dependency_reason`、`option_reason`、`refresh_property_controls`、`validate_control_values` | 依赖和原生选项策略，写入前后重新核对 |
| [types.rs](../../backend/ct-engine/src/devices/camera/types.rs) | 属性、选项、能力、健康和运行诊断 DTO |
| [actor.rs](../../backend/ct-engine/src/devices/camera/actor.rs)、[wpd.rs](../../backend/ct-engine/src/devices/camera/wpd.rs) | 消息代理、唯一 COM owner、取消代次和有界传输 |
| [device_controls.rs](../../backend/ct-engine/src/device_controls.rs) | 手动占用、独立拍摄目录、Stop 和领域命令 |

## 依据索引

下表的 `NB`、`D6`、`D8`、`D71`、`MTP` 是本地 MIT 源码的行号和方法名；这些文件是参考实现，不是运行时依赖。

- NB：[NikonBase.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/Nikon/NikonBase.cs)。属性常量 73–106；基础标签 219–267；高级目录 `AddAditionalProps` 505–534。
- D6：[NikonD600Base.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/Nikon/NikonD600Base.cs)。继承 D800：45；D7100 沿用的参数和 Live View 覆盖。
- D8：[NikonD800.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/Nikon/NikonD800.cs)。继承 NikonBase：42；ISO 与 Auto ISO 标签、能力和 Bulb 实现。
- D71：[NikonD7100.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/Nikon/NikonD7100.cs)。继承 D600Base：41；Live View 布局 43–72；电影光圈禁用 75–78；裁切覆盖 81–94。
- MTP：[BaseMTPCamera.cs](../../../Related_files/digiCamControl-master/CameraControl.Devices/BaseMTPCamera.cs)。标准传输与大端头字段读取 `ToInt16` 620–623。
- UM：[Nikon D7100 官方英文说明书](https://download.nikonimglib.com/archive7/VrdxE00Uyz9906XH31L97Xwuu922/D7100_%28En%2904.pdf)。下文使用印刷页码；PDF 页码为印刷页码加 28。
- GP：[libgphoto2 PTP定义](https://github.com/gphoto/libgphoto2/blob/master/camlibs/ptp2/ptp.h)及[属性配置映射](https://github.com/gphoto/libgphoto2/blob/master/camlibs/ptp2/config.c)确认D0C0为Bracketing enable（UINT8、OFF0/ON1）；只作协议事实核对，不复制运行代码。D078及D0C1–C6的类型/步长/程序/计数不是active OFF证据。NB 95/2089的D1B1是有符号曝光表读数，非BKT；D1B0–4不能凭零值推断BKT OFF。
- UM 136–139：M 模式包围仍改变快门，single release 每触发一张但序列继续，ADL 菜单 OFF 不取消 ADL 包围；UM 115：HDR 不能记录 NEF。[CCP2 Mechanical](https://nikonimglib.com/ccp2/onlinehelp/en/08_camera_controls_04.html) 支持远程 BKT。原审计 [FAIL 记录](../../.cache/tests/camera/20261005-fixed-automation-audit/result.json) 保留为历史证据，不定义当前准入策略。

## 手册功能关系与机身检查

以下关系来自 UM，而属性代码/数字标签来自协议参考及当前 descriptor；不能把菜单排序当作 PTP 编码。手册的模式可用性表已逐格查看（印刷 pp.321–323）。

| 功能关系 | 软件处理与不可交互条件 | 官方印刷页 |
| --- | --- | --- |
| M/P/A/S与快门、光圈 | M两者手动；S只手动快门；A只手动光圈；P两者自动。未确认模式禁依赖项；只读模式用机身切换，不自动夺取控制 | 47–51 |
| Auto ISO与ISO基准、上限 | Auto ISO控制只在P/S/A/M可用；上限仅ON有效。ON时ISO是基准，不冒充实测；用户基准高于上限时机身可能采用基准。固定扫描要求OFF | 81–82、323注4 |
| 测光、中央重点、曝光补偿 | 测光与中央重点只在P/S/A/M；中央范围仅中央重点有效，非CPU镜头固定8mm。M且Auto ISO OFF时补偿不改快门/光圈。特效模式的特殊补偿不按未识别模式码猜测 | 83、86–87、236、321 |
| 白平衡、荧光灯/色温、Picture Control | 只在P/S/A/M可调；荧光灯类型仅WB=5，色温仅WB=8012。先Apply父项并读回。手动预设是机身测量/照片参照，不是应用预设 | 89、94–101、105、321 |
| RAW/JPEG与裁切 | RAW位深/压缩仅含NEF的格式有效；纯NEF的尺寸来自图像区域，JPEG尺寸也适用于NEF+JPEG里的JPEG。裁切改变尺寸描述后重读；参数可设不扩展主机捕获格式 | 64–68 |
| HDR与RAW、包围、闪光 | HDR仅P/S/A/M且不能记录NEF；生效时不能包围或闪光。开启HDR需JPEG与鲜读包围OFF；选择NEF需先明确关闭已广告HDR。OFF选项保留，禁止同时提交HDR ON与包围ON/NEF。闪光模式参数不证明物理闪光灯状态 | 115–117 |
| 包围类型 | NEF只排斥白平衡包围，不能据此判断AE/闪光/ADL包围关闭；M模式包围仍可改变快门。当前只开放严格D0C0 enable，不猜未适配的类型/程序字段；固定扫描仍要求鲜读OFF | 133–139 |
| ADL | P/S/A/M可调，其他部分模式固定Auto或不可用。ADL OFF不能代替ADL包围OFF；固定扫描独立验证两项 | 113、139、321、323注3 |
| 长曝降噪、高ISO降噪 | 长曝NR在>1s时增加约一次曝光长度的处理，并阻止处理期间拍摄；当前快门短不意味着开关不可设。固定扫描要求OFF。高ISO NR OFF仍可能在ISO≥1250处理，不表述为完全无处理 | 228 |
| 闪光模式 | P/A可选慢速同步/红眼慢速，S/M不能；场景模式依实际描述。HDR生效时闪光相关选项禁用，OFF保留；设置不代表灯已收起或射线OFF | 120–121 |
| 释放、对焦、LV | 释放模式、burst、延迟互不等同，当前主机单拍要求single/burst=1/RAM。物理MF不可用软件绕过；LV焦点D061与普通D161分开。应用LV期间禁止摄影Apply/捕获，需明确Stop；不以LV拍摄自动复位机身功能 | 57–60、71–78、141–144、153–158 |

### 多重曝光的证据边界

[Nikon CCP2官方帮助](https://nikonimglib.com/ccp2/onlinehelp/en/07_ccp_window_01.html)把Multiple exposure列为机身操作且状态不显示；本实现没有已验证的对应PTP属性。当前RAM profile 6 / SD profile 7不含该运行时状态、人工确认字段或准入门禁，旧profile仅保留只读解码与原hash校验。

操作员在预检前查看机身拍摄菜单并选择 `Multiple exposure mode → Off`，检查指示器没有亮起或闪烁。UM pp.142–143说明常亮表示已开启、闪烁表示正在记录。USB连接不构成OFF证据；LV中拍摄会复位的规则不能转化为本应用隐式试拍（本应用LV期间禁止拍摄）。关机结束正在记录的一组，也不能被软件解释为当前series模式已读回OFF。

UM p.144只明确说正在记录多重曝光时“部分菜单项”会变灰，没有说某个变灰菜单唯一对应哪一种状态。P/S/A/M限制、HDR互斥和LV限制都是检查背景；不能仅凭菜单变灰判定多重曝光ON或OFF。机身检查提示仅出现在Scan Preparation，不修改readiness、不伪造机器证据、不创建确认门禁。一个host NEF仍不证明一次物理曝光，完整扫描与事件语义继续按实机证据验收。

## 参数覆盖

当前 `NIKON_PROPERTIES`目录实际包含34个 D7100摄影/采集参数候选，新增 `bracketing`；另有单独的只读运行诊断目录。这是已实现子集，不是“全部厂商功能完备”的声明。每项只在设备实际声明且成功描述后可读；可写还要求设备写权限、当前依赖、已确认实际值和未被占用。未声明的 TIFF、未压缩 RAW、自然光自动、亮部重点测光等通用选项显示为不支持。

独立LV目录含2项，不进入摄影34项目录、预设或扫描profile：CameraHealth.liveViewControls={sessionId,properties:{cameraZoom,focusMode}}。每次Start生成独立nonce；读、写与返回都核对当前会话和实际广告，旧会话请求拒绝。已记录的早期分项证据包括cameraZoom读写、内存传输和旧nonce拒绝；focusMode写入、镜头驱动和所有禁止条件仍按具体构建/镜头验收，见[验证索引](../validation/README.md)。

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
| `whiteBalance` / `5005` | 2自动、4晴天、5荧光灯、6白炽灯、7闪光灯、8010阴天、8011阴影、8012色温、8013预设 | 先写白平衡并重读，再应用子参数；扫描 RAM profile 6 / SD profile 7只接受已确认固定档位，拒2及已声明的自然光自动8016 | NB 236–247、1459–1499 |
| `whiteBalanceFluorescent` / `D14F` | 0–6为七种光源标签 | 仅白平衡=5有效 | NB 806–827、1496–1497 |
| `whiteBalanceTemperature` / `D01E` | NB 的0–30色温索引标签；实际编码以 descriptor 为准 | 仅白平衡=8012有效；不将索引直接当Kelvin写入 | NB 830–874、1498–1499 |
| `exposureCompensation` / `5010` | 原生有符号值，显示EV | 保留实际描述；M模式下不伪称它一定改变快门/光圈或射线 | NB 1502–1575 |
| `meteringMode` / `500B` | 2中央重点、3矩阵、4点测光；8010通用候选 | 中央重点范围仅在本模式有效 | NB 262–267、1615–1643 |
| `centerWeightedArea` / `D059` | D7100：6、8、10、13 mm、全画面平均 | 测光模式=2；非CPU镜头的有效直径限制仍需镜头证据；NB重复10mm及通用12mm标签不能照搬 | NB 734–750；UM 236 |
| `driveMode` / `5013` | D6：1单张、2CH、8010CL、8011自拍、8012抬镜、8016安静、8017遥控 | 设备只读时禁写；CT要求单张1；其余设置不等于支持对应采集事务 | D6 190–209 |
| `burstNumber` / `5018` | UINT16，真实范围由 descriptor 决定 | 扫描和独立主机单拍均要求1；不能误用D10B作数量 | NB 1053–1069；`prepare_host_capture` |
| `recordingMedia` / `D10B` | 0存储卡、1SDRAM | 明确 Apply选择SD=0或RAM=1并读回；主机传输须当前会话显式许可；预检/开始/恢复不隐式修复或切换目标 | NB 96、1099–1105 |
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
| `activeDLighting` / `D14E` | 0极高、1高、2标准、3低、4关闭、5自动 | D7100物理适用，RAM/SD固定profile要求OFF=4并冻结；未广告或读取失败仍UNKNOWN并阻扫描，不作不适用 | NB 672–689；UM 113、139 |
| `hdrMode` / `D130` | MIT映射0关闭、1单张、2系列 | NEF/NEF+JPEG不允许开启，OFF仍可用；扫描已广告时必须鲜读OFF=0，ON/error/矛盾拒绝；仅未广告时可用确认NEF=4的互斥证明 | NB 653–667；UM 115；`profile_readback` |
| `hdrExposureDifference` / `D131` | 通用MIT候选0自动、1/2/3 EV | D7100用户手册只列Mode/Strength，不能假定另有此属性；未声明禁用；开启HDR且JPEG格式才有效 | NB 634–649；UM 115–116 |
| `hdrSmoothing` / `D132` | 0高、1标准、2低、3自动、4极高；D7100显示为HDR强度 | HDR开启且JPEG格式才有效；标签来自继承映射，用户手册不提供PTP数字认证 | NB 614–630；UM 116 |

### 写入和依赖刷新

`apply` 先校验整个请求的原生值域与最终依赖。显式 HDR OFF、包围 OFF 请求先执行，避免先切 NEF 再关 HDR，或先开 HDR 再关包围；只执行用户提交的键，不隐式补 OFF。随后按曝光模式、Auto ISO、格式、裁切、白平衡、测光、HDR等父参数排序。父参数（含 HDR、包围、闪光模式）写入且读回后重读 descriptor，随后再次校验后续参数，不能用预计的新枚举代替真实设备描述。原先只读的子参数必须先显式应用父参数并重新读取后才能写。前端仍按当前实际读回启用选项，草稿不会提前解锁控件；整个草稿中的互斥目标另行校验。

相同且已确认的原生值是无写入的 no-op。部分失败保留实际值或未知状态、请求目标和失败原因，不虚构回滚。UI预设只保存已确认的可应用摄影参数；原生设备状态和运行诊断不进入通用可写预设。`camera_apply_properties` 的入队前拒绝按 `PropertyRejection` 分码返回（`LIVE_VIEW_APPLY_FORBIDDEN`、`INVALID_CAMERA_PARAMETER_COUNT`、`CAMERA_PROPERTY_UNDESCRIBED`、`CAMERA_FOCUS_MODE_LOCKED`、`CAMERA_PROPERTY_WRITE_UNSUPPORTED`、`CAMERA_PROPERTY_UNSUPPORTED_VALUE`、`CAMERA_PROPERTY_DEPENDENCY_UNMET`、`CAMERA_PARAMETER_SESSION_UNAVAILABLE`），不再折叠为单一错误码；`preset_load` 仍使用粗粒度的 `UNSUPPORTED_CAMERA_PROPERTIES`。

## 操作能力

| 功能 | 当前覆盖 | 依据和边界 |
| --- | --- | --- |
| 显式发现、选择、连接、断开 | 已实现 | `actor`与`wpd`；无参数连接拒绝隐式选择；未知型号仅标准已描述能力。无 `deviceId` 的 `camera_connect`（含 `retry_device{device:"camera"}`）只做发现，operation kind 为 `camera_discover`，受理记录不声明已连接；有明确 `deviceId` 才为 `connect`；清理未确认时需显式 `acknowledgeUnconfirmedCleanup` |
| DeviceInfo、vendor属性扩展 | 已实现 | `nikon::connect`仅在D7100身份绑定且90CA被声明时读取，保留原始/有效列表和诊断哈希 |
| 单拍 RAM NEF、SD 存卡与 CT 捕获 | 原生事务已实现；已有分项实测 | RAM 要求当前会话显式主机许可、关联 C101、完整文件/hash；C102 可选。SD 要求唯一卡对象与完成事件、严格元数据和持久回执，扫描零图像载荷。两者均要求 LV OFF、session/operation 和各自 readiness；默认 SD 的后续导入与精确清卡见 [ADR 0004](../decisions/0004-camera-sd-card-acquisition.md)。一个文件不证明一次物理曝光。 |
| 独立JPEG/TIFF、RAW+JPEG双对象采集 | 禁用 | 当前对象解析与提交仅接受NEF，缺少多对象关联和格式确认事务；更改格式不会扩展捕获能力 |
| AF | 已实现；镜头与工况按各自证据验收 | NB `AutoFocus` 1853–1858、90C1；实际操作列表及DeviceReady门禁；LV需fresh lensSort=1、照片selector=0、D061 mode0；物理MF/AF-F/未知镜头资格禁用，不以lensSort单项推断motor。能力未广告时 `camera_autofocus` 入队前拒绝为 `CAMERA_AUTOFOCUS_UNQUALIFIED` |
| 镜头步进 | 已实现候选；新增LV资格仅离线 | NB 1799–1807、9204；LV开启、步数有界、fresh lensSort1/selector0/D061 mode0或4及实际操作支持；AF-F2禁止有界motor、hardMF3禁写motor；不将脉冲计数当绝对焦点。LV未激活或 `focusStep` 未广告时 `camera_focus` 入队前拒绝为 `CAMERA_FOCUS_UNQUALIFIED` |
| Live View启停 | 已实现；P0流与EOF清理已分项验证 | NB 1697–1724、9201/9202，原生读取D1A2确认；开启期间前后端拒绝拍摄，不自动停后拍再恢复 |
| Live View网格、准线、显示缩放 | 已实现 | React纯显示；CSS显示缩放不改变相机D1A3，也不制造拍摄/对焦命令 |
| 相机端LV缩放D1A3、LV焦点D061 | 新增独立2项LV目录；读取与 `cameraZoom` 写入已实机验证，`focusMode` 写入仍未验证 | NB 1126–1147、1646–1688；当前候选`cameraZoom`按标量整数descriptor类型1–8的原生宽度读写（值域0–255），`focusMode`仍严格UINT8；当前广告/鲜读/同类型写后读回，唯两key；typed read/configure绑定当前LV nonce。2026-10-06 实机：两项描述与鲜读成功、`cameraZoom` configure 完成并读回、旧 nonce 拒绝为 `STALE_LIVE_VIEW_SESSION`；物理MF3下 `focusMode` 只读，无写入实机证据（见[后端重验证记录](../validation/README.md) T5）。D061只允许有据的目标0 AF-S/2 AF-F/4软件MF，物理MF3只读；不混同D161或UI缩放 |
| 点击画面对焦区域9205、焦点框/旋转/水平元数据 | 尚未覆盖 | NB 1943–1948；D71 43–72提供头字段；需严格坐标范围、当前帧/会话和已声明操作，不能凭CSS像素直接发坐标 |
| Bulb/Time、电影录制、电影光圈 | 禁用 | D8 199–247存在不同开始/终止链；D71 75–78禁用电影光圈；当前没有这些模式的有界捕获/结束/清理事务 |
| Host Mode/Lock、Application Mode | 未作为普通参数开放 | NB 416–431、D8 228–247；初始化没有证明必须自动90C2(1)，不可为解除只读而隐式改变模式 |
| 卡格式化、删除、设备复位 | 未开放 | 参考实现有此类操作；它们不属于主机NEF采集所需能力，不能作为错误恢复自动执行 |
| 电池与LV禁止条件 | 已实现按声明的只读原始诊断 | `RUNTIME_DIAGNOSTICS`含5001电池UINT8及D1A4禁止条件UINT32；不混入摄影profile，不把未知位/原始状态当整体健康保证 |
| 镜头类型、卡槽及LV驱动资格 | 已实现只读运行证据及保守LV资格，非全工况认证 | LensSort D0E1：NB554–567；ActiveSlot D1F2：NB571–586；D1A6照片/电影selector及D061 LV焦点已入专用模型。D0E1/D1A6/D1F2只按本次广告读取，缺席UNKNOWN，保留nativeWritable但不开放setter；D1A4仍为原始诊断，不声称全部镜头/禁止条件已完备 |
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

源文件：[live_view.rs](../../backend/ct-engine/src/devices/camera/live_view.rs)、[camera_forwarder.rs](../../backend/src-tauri/src/camera_forwarder.rs)、[Tauri入口](../../backend/src-tauri/src/main.rs)、[cameraControls.ts](../../frontend/src/platform/cameraControls.ts)、[cameraLiveView.ts](../../frontend/src/platform/cameraLiveView.ts)、[WorkstationMedia.tsx](../../frontend/src/WorkstationMedia.tsx)、[CameraPanel.tsx](../../frontend/src/CameraPanel.tsx)。

这条预览链不调用应用的JPEG临时文件、图像导出、扫描manifest或JSONL图像字段。NEF文件写入仅存在于明确拍摄事务；named pipe借用Windows文件句柄接口不等于图像落盘。“不落盘”指应用预览实现不创建帧文件；不把操作系统分页、崩溃转储或外部截图行为纳入此保证。

## 仍需保留的限制

1. 设备证据按构建、相机、镜头和工况记录。已有独立 NEF/LV 及后续小规模联动分项 PASS，不能认证当前全部参数、长时预览、联锁或物理精度，见[验证索引](../validation/README.md)。
2. LV 驱动需当前操作广告、鲜读 lensSort/照片选择器/D061 及 DeviceReady；未知镜头、物理 MF 和 AF-F 的限制不能由历史 setter 成功覆盖。
3. HDR 配置不等于 HDR 捕获；当前不提供已验证 HDR 采集事务。自定义 Picture Control 完整编辑和点击 AF 也不能从通用目录推定可用。
4. RAM profile 6 / SD profile 7 共用严格摄影策略，目标和事务分别验证；单个 NEF 文件不证明机身多重曝光已关闭，人工菜单检查不构造机器 OFF 证据。
5. 参考实现中的静默写入错误和通用标签不直接沿用；生产端严格检查编码、实际型号描述和写后读回。

## 固定扫描、独立摄影与预览访问

### 独立LV控制会话

每次inactive→Start建立新32hex nonce，区别于稳定camera connection/capture ID会话。`camera_live_view_read {sessionId}`鲜读两项LV descriptor/value；`camera_live_view_configure {sessionId,controls}`只收cameraZoom/focusMode，实际广告、原生整数descriptor（D1A3类型1–8且0–255；D061严格UINT8）、fresh权限/值及同类型写后读回均必要。帧pipe/metadata与controls会话一致，Stop/隔离/退休撤销，旧请求/迟到结果不跨新LV。普通Apply仍拒绝LV，LV控制不进入摄影presets或固定profile，不提供任意setter透传。

Advanced的preset Compare只把supported/readable/confirmed、无error的actual作为证据；未知或保留旧值即使相等也显示UNKNOWN。WorkstationDialog改为mount焦点和最新callback refs，持续snapshot不抢输入焦点。[163257实际组件浏览器回归](../../.cache/tests/frontend/20261005-163257-dialog-focus-fix/result.json)PASS且明确UI_ONLY_NO_HARDWARE。摄影34项、LV2项及运行诊断分别展示；9205区域AF、自定义Picture Control编辑、Bulb/Time、视频/JPEG/多对象事务仍未实现。

相机核心参数只有明确 Apply 才串行写入并读回；`prepare_scan` 只原子提交任务与完整几何，用确认快门派生曝光。预检、开始、恢复和扫描事务鲜读比较，不自动停 LV、不写快门/RAM/burst。RAM 使用 schema 3 / profile 6，默认 SD 使用 schema 4 / profile 7 并冻结卡策略；两者冻结 WB、C0/ADL/HDR 证明及实际采集参数，规范 hash 纳入固定策略版本 3。旧 profile 1–5 原 hash 和只读重构保持，拒绝追加投影。HDR 只有未广告时可使用已确认 NEF=4 的互斥证明；已广告的 ON、错误或矛盾读回仍拒绝。

SD 独立单拍使用固定卡参数策略并只确认回执，不生成本地照片预览；以下独立自动摄影与文件预览契约适用于已明确授权的 RAM 路径。独立摄影允许已确认的M/P/A/S、自动ISO/WB及支持的AF模式；不会隐式触发AF。只有实际自动模式决定的ISO/快门/光圈输出可变化；RAM介质、NEF格式、单host输出数量、会话及C101对象/文件证据仍严格校验。此入口保证一个验证完整的主机 NEF。90C0、一个与本操作关联的C101对象、完整长度/hash、秒级对象窗口、多对象拒绝和取消门禁保持（C102完成可选：可早到、迟到或不到，且仅作诊断）。Bulb/Time、JPEG和多对象采集不会因此开放。

正常 `camera_capture` 从唯一 `Parameters.savePath`根目录派生 `CameraCaptures`子目录，renderer不能换成任意路径；显式目录仅用于诊断 `camera_test_capture`且仍检查已有扫描清单、目录别名和不覆盖。完整文件提交后登记 capture ID、会话、operation ID、bytes及hash。照片预览只接收 capture ID，Tauri再次核对登记、大小/hash，复用 NEF内嵌 JPEG；无内嵌 JPEG时报预览不可用，不另写 JPEG临时文件。浏览器模拟只报告 NO HARDWARE，不登记或制造真实照片。

## 验证边界

历史指定 P0 包证据（不代表当前交付）：[0070 result](../../.cache/tests/desktop/20261005-004700-0070-nef/result.json)、[新NEF与C101/C102](../../.cache/tests/desktop/20261005-004700-0070-nef/captured-nef.json)。EXE SHA-256 `0070808c9fb206d4745ba039bcd1eb4c13d19e0a12fef3d164be93797637862c`；engine SHA-256 `f8f074d75518e4cf0304e630c683ba643e8f5f23be882b00687ee03e3b191ffa`。新文件23,476,423字节，SHA-256 `b20e4b1b15e1c97541befc9dbe482f15ff7dabd2a9910304527b6b0028e7c2f9`；参数恢复PASS；UI LV 30.1096秒、首帧后missing=0、实时像素文件=0，期间存在STALE/首次暂无帧记录。App/engine均strict exit0；关闭前独立 LV-stop终态快照未保留，不能据此扩大该单项验收。测试明确无射线/Nano运动、fullLinkedHardwareCertified=false。上述tmp本机过程证据不进入Git。

历史 profile 5 策略的 [offline PASS](../../.cache/tests/ct-engine/20261005-profile5-final/result.json) 与 [simulation-only PASS](../../.cache/tests/camera/20261005-fixed-automation-audit/preview-final-result.json) 仅认证对应历史实现。当前策略为 RAM profile 6 / SD profile 7，不能沿用旧包的硬件结论。当前及历史验收见 [后端重验证记录](../validation/README.md)；后续小规模联动已有分项实测；当前版本全部工况、物理精度和定量成像不能由旧包或有限实测推定，见验证索引。
