# 工作站重构候选验收汇总（2026-10-05）

对象：`0.9.0-rc.1` Windows x86_64 candidate。本文汇总既有证据，编写时不连接设备、不运行安装器、不改源码或包。`PASS`只适用于每项明确范围；`BLOCKED`表示缺少必要设备/验收条件；安装升级待测项标为`PENDING`，由Root取得实际结果后补齐。

后续完成审计将开发版本推进`0.9.0-rc.2`，包括独立成功notice和发行门禁修复。本文rc1产物、哈希及验收保持为历史基线，已签名rc1文件不改；不能把本文PASS外推为rc2通过。rc2构建和真实rc1→rc2验收由Root在全部源码冻结后另行记录。

## 1. 当前结论

| 项目 | 状态 | 已证明范围 / 尚缺条件 |
| --- | --- | --- |
| 前端、领域和桌面离线门禁 | PASS | 136前端测试；243领域测试＋2入口测试；45桌面测试；typecheck及模块图通过 |
| 最终portable、NSIS、MSI构建 | PASS | 已生成对应包、记录大小/hash；发行身份按实际提取EXE标记核对 |
| 签名manifest、3种artifact及3个离线ZIP验签 | PASS | 同一客户端验证入口检查签名、版本、目标、分发类型及hash；未发布 |
| 同一engine的Nano台架操作 | PASS（旧封装分项） | 旧3b2包实际HOME、3°、途中Stop和fresh LOCKED；其engine与最终fc2包一致，但最终fc2桌面EXE未重新运动实测 |
| 最终候选D7100单拍、Apply恢复及LV | BLOCKED | 当前PnP未发现相机，WPD列表为空；等待用户开机/恢复连接后由Root继续，不能借旧P0给本包认证 |
| 旧P0独立D7100 NEF/LV | PASS（独立证据） | 限定0070包；保留C101/C102、完整文件/hash、参数恢复、30秒LV及strict exit0证据 |
| 完整portable更新：导入→维护报告→退出→helper替换→新启动 | PASS | 实际rc.0→rc.1离线升级、completed receipt、新启动及数据hash保留；使用无设备never-owned分支 |
| 实际NSIS安装/启动/卸载 | PASS | 最终安装EXE身份、正常启动/退出及卸载登记清理已实测 |
| 实际NSIS rc.0→rc.1升级/重启/卸载 | PASS | 完整维护→安装器→重启核对→completed receipt，数据hash保留、无自动连接 |
| 实际MSI安装及版本升级 | PENDING | 当前只有提取、身份和Property/Upgrade表证据，未执行安装 |
| 完整射线联动扫描、联锁、负载与重构精度 | BLOCKED | 本次Nano实机明确排除射线；相机待连接，尚无本包全链验收 |
| 在线signed channel更新与Windows发布者认证 | BLOCKED | GH manifest尚未发布，无本候选线上更新传输PASS；未配置Authenticode证书 |
| 既有正式便携入口 | PASS（保留） | `portable-release/micro-ct-workstation-portable.exe`继续保留c19来源/hash，不被候选替换 |

[最终build-info](../release/0.9.0-rc.1/build-info.json)的`hardwareAcceptance`仍为“仅离线验证”。本文单列Nano分项实机证据，不改生成物字段，也不把候选变成全系统硬件认证版本。

## 2. 包和来源身份

[075000修复后构建结果](../tmp/tests/desktop/20261005-075000-typed-update-release/result.json)、[构建日志](../tmp/tests/desktop/20261005-075000-typed-update-release/stdout.log)记录本次最终产物；[build-info](../release/0.9.0-rc.1/build-info.json)记录基线提交`11eed6cfea8f3f5118fcd07e70319f98fc398b51`、`dirty=true`及构建时源码树SHA-256 `7644197324dcdab16efb7ec6e5bb5c8bb833a788de68b5a7c4f3d3180386d3b6`。基线提交本身不能代替这次含有效未提交改动的产物身份。

| 最终产物 | 字节数 | SHA-256 |
| --- | ---: | --- |
| [portable EXE](../release/0.9.0-rc.1/micro-ct-workstation_0.9.0-rc.1_windows_x86_64-portable.exe) | 27,330,048 | `fc2d9c8f07e7a3e4a2054bb0342756d6c3c348fff194dc1292d4414966e4e6eb` |
| [NSIS setup EXE](../release/0.9.0-rc.1/micro-ct-workstation_0.9.0-rc.1_windows_x86_64-setup.exe) | 226,449,778 | `026610f57eed8e38cf83f015f4387c86c5ad6911d61e5f21458dfc850dfe87cd` |
| [MSI](../release/0.9.0-rc.1/micro-ct-workstation_0.9.0-rc.1_windows_x86_64.msi) | 227,540,992 | `5615b2ffab635f4865a08cb58eaacab60005bf742c48911128de9319fbed59c5` |

最终portable SHA-256为`fc2d9c8f07e7a3e4a2054bb0342756d6c3c348fff194dc1292d4414966e4e6eb`，实际NSIS安装EXE为`49677074d443059dfb80597701226dfeffbd1b371c64cd8ffd7811e5fb29f4fc`，engine仍为`f433fde0fb94971e9ecc64d5628541e708e043487d7d942f01a3f6ff6508d1b4`。这些最终封装已作更新/安装启动验收。

[045600 Nano实机process.json](../tmp/tests/desktop/20261005-045600-restructured-hardware/process.json)记录的是较早桌面EXE `3b2baaaa466a3f7c8a9812909cc640a082b847202ae58a5943840796815b7804`，其engine恰与最终包完全相同。该运动证据只属于旧3b2封装/相同engine，不能写为最终fc2 EXE直接完成运动实测。相同发行文件名后续重建也不改变历史process.json中的实际hash。

[实际提取身份](../tmp/tests/desktop/20261005-075000-typed-update-release/final-bundle-identities.json)及[发行身份记录](../release/0.9.0-rc.1/bundle-identities.json)确认NSIS安装EXE标记`__TAURI_BUNDLE_TYPE_VAR_NSS`，提取EXE hash为`49677074d443059dfb80597701226dfeffbd1b371c64cd8ffd7811e5fb29f4fc`；MSI安装EXE标记`__TAURI_BUNDLE_TYPE_VAR_MSI`，提取EXE hash为`3077e759b0f046aba51ea51cf1b3ae98332ff7bbe2e3ca70e78f9784e42fc81e`。安装器里的EXE与portable有意分别携带分发身份，不能凭相同文件名或版本混用。

[MSI身份](../release/0.9.0-rc.1/msi-identity.json)和[实际策略表](../tmp/tests/desktop/20261005-075000-typed-update-release/final-msi-policy-tables.json)记录：

- App SemVer仍为`0.9.0-rc.1`；MSI ProductVersion映射为`0.9.1`，目标`x64;0`。
- 固定UpgradeCode为`{A96DC7F4-7384-4B5E-9AB5-6BE9CB76C8F2}`；本包ProductCode为`{86A688C9-4FA9-4F9F-8A38-445ADFEF1C60}`。
- Upgrade/LaunchCondition表存在`WIX_DOWNGRADE_DETECTED`拒降级配置；`installerExecuted=false`。
- [NSIS提取日志](../tmp/tests/desktop/20261005-075000-typed-update-release/final-nsis-extraction.txt)为成功提取；[MSI反编译日志](../tmp/tests/desktop/20261005-075000-typed-update-release/final-msi-extraction.txt)含ControlEvent引用警告。上述提取/表检查均不证明实际安装、更新或数据保留。

正式入口SHA-256继续为`c19b828d8f18d13d3ef2811b95b5094863aee0e494611140001bc69716c91334`，由build-info和构建脚本保护。候选位于`release/0.9.0-rc.1/`；没有把正式入口更新成rc包。

## 3. 历史失败、修复、签名和离线运输包

第一次[064000旧签包](../tmp/tests/desktop/20261005-064000-signed-release/result.json)的签名/ZIP验证通过，但[062000实际portable导入与确认日志](../tmp/tests/desktop/20261005-062000-portable-upgrade/actions.jsonl)保存了真实失败`ENGINE:COMMAND_PAYLOAD_MISMATCH`：Host发送维护命令时缺少领域要求的typed payload；没有取得安全报告，更新未继续。旧签包PASS不能覆盖这一失败。

源码修复后，[074000真实sidecar envelope回归](../tmp/tests/desktop/20261005-074000-update-envelope/result.json)与[执行命令](../tmp/tests/desktop/20261005-074000-update-envelope/command.txt)为PASS；075000重新构建并核对实际身份，随后[080000最终签名结果](../tmp/tests/desktop/20261005-080000-typed-signed-release/result.json)、[最终签名/验证日志](../tmp/tests/desktop/20261005-080000-typed-signed-release/stdout.log)为PASS。[当前release-manifest](../release/0.9.0-rc.1/release-manifest.json)绑定最终fc2/0266/5615产物的size/hash/signature及`0.9.0-rc.1` candidate身份。旧062000失败记录独立保留。

[实际签包脚本](../scripts/sign-update-release.mjs)先对portable/nsis/msi逐一调用`--verify-update-package`，再对每个ZIP调用`--verify-offline-update`；任何非0退出即中止。080000日志的汇总PASS位于全部验证之后。日志没有为每个ZIP单独输出完整诊断，不能虚构不存在的独立日志。

| 离线运输包 | 状态 | 验证范围 |
| --- | --- | --- |
| [portable ZIP](../release/0.9.0-rc.1/micro-ct-workstation_0.9.0-rc.1_windows_x86_64_portable-offline-update.zip) | PASS | ZIP_STORED、同一签名/版本/target/kind/hash入口验证 |
| [NSIS ZIP](../release/0.9.0-rc.1/micro-ct-workstation_0.9.0-rc.1_windows_x86_64_nsis-offline-update.zip) | PASS | 同上；不等于实际安装已通过 |
| [MSI ZIP](../release/0.9.0-rc.1/micro-ct-workstation_0.9.0-rc.1_windows_x86_64_msi-offline-update.zip) | PASS | 同上；不等于实际MSI升级已通过 |

GH signed channel manifest未发布，因此本候选实际线上更新传输尚无PASS。未配置Authenticode证书；Minisign更新签名不等于Windows发布者认证、设备安全或硬件认证；私钥和密码不进入本文或发行包。完整在线/离线更新策略见[分发方案](desktop-update-distribution-plan.md)。

## 4. 已实现的工作站和领域合同

实现事实见[架构](architecture/README.md)、[执行流程](architecture/execution-flows.md)、[相机能力矩阵](architecture/native-camera-capability-matrix.md)。本节描述源码和离线合同覆盖，不声称每种DPI、镜头、自动模式或设备联动都已在最终包实测。

- **常驻控制与中央展示**：Camera/Turntable分别连接和操作；基本参数与Advanced共用会话草稿。菜单按Task/Devices/View/Tools/Help组织。中央切换Equipment、Result、独立照片和Live；Stop保持可达。底部日志/运行状态共用209.375设计高度、六个竖排日志标签；删除重复PROCESS SUMMARY和独立重构事件区。
- **唯一保存根目录和原子准备**：`set_capture_root`只编辑`Parameters.savePath`；`prepare_scan`一次验证并提交任务ID、1–3600整数投影数、maxXraySec和全部几何。曝光由确认的原生快门精确派生，取消未提交窗口不建文件、锁或半套参数；预检后没有第二个几何窗口。
- **明确Apply与固定扫描**：只有明确Apply写核心相机配置并读回。预检、开始、恢复及拍摄事务鲜读比较，不自动停LV、不写快门/RAM/burst。扫描要求M、Auto ISO OFF、硬件/软件MF、固定WB及已支持ADL/HDR/长曝降噪/曝光延迟OFF；ADL OFF=4、HDR/长曝NR OFF=0、曝光延迟OFF=3。未知、漂移、未应用草稿、自动设置按结构化issues阻塞。
- **profile兼容和Start拒绝**：新schema 3清单采用内部profile 4，冻结固定策略支持证据并纳入hash。旧profile 1/2/3保留原hash和只读重构，拒追加。Start先鲜读通过，再登记、后台持久化和真正启动；同一operation receipt贯穿全程。ISO/WB漂移或LV拒Start不产生假taskLock，已持久/恢复锁继续保留。
- **独立摄影与媒体**：已验证能力下允许自动曝光/ISO/WB和支持的AF模式，不隐式触发AF、出束或停止LV。当前捕获仍仅单张D7100 RAM NEF，严格C101/C102、会话/操作、曝光结束与完整文件/hash、取消、不覆盖。正常单拍仅写根目录/CameraCaptures。完整CapturedImage登记后才允许capture ID预览，Tauri重核大小/hash并提取内嵌JPEG；LV全链在内存，不产生应用实时帧文件。
- **事件与维护**：重构真实worker事件进入聚合consoleLogs，稳定ID、`source=reconstruction`，UI不制造事件。更新前维护入口拒扫描、暂停、收尾、拍摄、运动、LV、重构、busy和持久锁；本session/op清理报告、maintenanceLocked与无事务证据成立后，Tauri才EOF→owned engine exit0→原App正常退出→helper。普通Stop使旧ready失效但不清维护锁；取消须匹配操作且清理确认，不自动放弃或恢复设备。
- **浏览器预览**：只模拟合同，显式连接和Apply，启动不自动HOME/预检；不写真实照片、不登记CapturedImage或可访问descriptor，不能作为生产维护报告或硬件证据。

## 5. 最终离线门禁

[083000最终结果](../tmp/tests/desktop/20261005-083000-typed-final-gates/result.json)、[命令](../tmp/tests/desktop/20261005-083000-typed-final-gates/command.txt)、[前端日志](../tmp/tests/desktop/20261005-083000-typed-final-gates/frontend.log)及[Rust日志](../tmp/tests/desktop/20261005-083000-typed-final-gates/rust.log)均为PASS：136前端、243领域＋2入口＋45桌面测试；typecheck和module-map均退出0。7个领域ignored仍不计为PASS。

此前[055000前端门禁](../tmp/tests/frontend/20261005-055000-workstation-gates/result.json)的135测试与[055000 Rust门禁](../tmp/tests/desktop/20261005-055000-workspace-gates/result.json)的243＋2＋44属于修复前范围；[075000并行门禁日志](../tmp/tests/desktop/20261005-075000-typed-workspace-gates/stdout.log)出现既有idle-camera Stop单测的80ms固定等待假设失败。该测试随后只改为有界等待真实清理及匹配Stop回执，保留cleanup先于首次tick、无Fault及Stopped要求；生产6s清理预算未变。083000是当前最终全绿记录，历史失败不删除。

覆盖固定策略、独立自动模式、准备原子性、完整登记、Start拒绝不遗留假锁、旧数据只读、取消/迟到结果、维护报告、typed update envelope及更新器校验边界。离线成功不产生相机或射线联动实机结论。

## 6. 实机证据及最终包边界

### 6.1 Nano台架：PASS（旧3b2封装、相同最终engine）

[045600过程身份](../tmp/tests/desktop/20261005-045600-restructured-hardware/process.json)、[命令](../tmp/tests/desktop/20261005-045600-restructured-hardware/commands.jsonl)、[关联快照](../tmp/tests/desktop/20261005-045600-restructured-hardware/snapshots.jsonl)绑定旧3b2 portable及engine f433fde hash；该engine与当前final一致，最终fc2桌面封装未在此记录里重新运动。设备报告RTS9060_NANO、protocol 2、version 2.1.0、build 20260918、8微步、96,000 pulses/rev；这些字符串不单独证明flash、bootloader/fuse或负载精度。

| 动作 / 状态 | 本次证据 |
| --- | --- |
| 明确连接与台架确认 | COM7；physicallyIsolated=true；射线始终UNKNOWN，未连接或开束 |
| HOME | 对应operation完成，fresh IDLE，position/target=0、reference/homed/rearmed有效 |
| 规定方向3° | 对应operation完成，fresh IDLE，position/target=−800 pulses，展示−3°；captureId=0 |
| 请求10°途中Stop | 未把10°请求当到位；Stop receipt从started 22:52:50.008Z到completed 22:52:50.134Z，126ms |
| Stop后的即时状态 | 角度UNKNOWN、status清除；不沿用运动目标或此前有效参考 |
| 后续fresh STATUS | LOCKED，position/target=−1141，reference/homed/rearmed均无效、captureId=0；保持HOME要求 |
| 文件与退出 | 图像数0；[exit.json](../tmp/tests/desktop/20261005-045600-restructured-hardware/exit.json)为App/engine exit0 |

126ms是本次软件Stop operation受理至确认的时间，不是独立机械计量、急停继电器响应或完整射线关断时延认证。该流程没有逆向普通运动、相机拍摄或射线动作。

### 6.2 最终候选相机：BLOCKED，等待用户开机

[084000落盘PnP枚举](../tmp/tests/desktop/20261005-084000-current-device-enumeration/result.json)为BLOCKED，cameraPresent=false，仅见CH340(COM7)；落盘[UI状态](../tmp/tests/desktop/20261005-045600-restructured-hardware/ui-state.json)和[最终快照](../tmp/tests/desktop/20261005-045600-restructured-hardware/latest-snapshot.json)显示camera disconnected、discovered为空，ISO/快门/光圈/WB为UNKNOWN且Read/Apply/拍摄/LV禁用。当前候选不能进行相机实机验收，等待用户开机并恢复连接；保持明确选择，不以空列表隐式连接、不模拟成功。

本包的单拍登记/预览、Apply参数恢复、新固定策略、独立自动模式、LV和取消清理必须使用本包的新会话设备证据再验收。PnP枚举与WPD/UI空列表证据分别保留。

### 6.3 P0相机分项：独立保留

[0070 P0 result](../tmp/tests/desktop/20261005-004700-0070-nef/result.json)、[NEF/C101/C102](../tmp/tests/desktop/20261005-004700-0070-nef/captured-nef.json)限定EXE `0070808c9fb206d4745ba039bcd1eb4c13d19e0a12fef3d164be93797637862c`及engine `f8f074d75518e4cf0304e630c683ba643e8f5f23be882b00687ee03e3b191ffa`。记录新独立NEF 23,476,423字节、hash、可靠关联曝光结束、原参数恢复、30.1096秒UI LV（首帧后missing=0、应用实时像素文件=0）及strict App/engine exit0。期间STALE/首次暂无帧记录保留，关闭前独立LV-stop终态快照未保留。无射线/Nano运动、fullLinkedHardwareCertified=false。该证据不跨包替代最终0.9候选或全CT认证。

## 7. 实际安装和升级验收

### 完整portable rc.0→rc.1升级：PASS

[075600结果](../tmp/tests/desktop/20261005-075600-typed-portable-upgrade/result.json)、[维护ready状态](../tmp/tests/desktop/20261005-075600-typed-portable-upgrade/ready-state.json)和[080500新启动probe](../tmp/tests/desktop/20261005-080500-portable-upgrade-restart/restart-probe.json)证明真实离线导入→验证→匹配本session/op的maintenance ready→父App/engine exit0→helper替换→最终fc2新启动。旧EXE备份hash为`668c3cf5acce1d0a2f213a268c73035fb38d239e841a4014b7b08e600fcb08aa`；新版本0.9.0-rc.1、kind portable、receiptStatus completed。新启动无操作、无任务锁、Camera/Nano/LV均未连接，射线UNKNOWN；[重启退出](../tmp/tests/desktop/20261005-080500-portable-upgrade-restart/exit.json)为App/engine0。

### 最终NSIS安装、启动、卸载：PASS

[081000安装](../tmp/tests/desktop/20261005-081000-final-nsis-install/install-result.json)核对实际EXE49677074…、DisplayVersion 0.9.0-rc.1及安装登记，installer exit0；[081500启动probe](../tmp/tests/desktop/20261005-081500-final-nsis-start/restart-probe.json)为kind nsis、无自动连接、射线UNKNOWN，[退出](../tmp/tests/desktop/20261005-081500-final-nsis-start/exit.json)为App/engine0。[卸载结果](../tmp/tests/desktop/20261005-081000-final-nsis-install/uninstall-result.json)为exit0、registrationRemaining=0、正式c19保持。

### 实际NSIS rc.0→rc.1完整升级、重启、卸载：PASS

[084500升级结果](../tmp/tests/desktop/20261005-084500-nsis-installed-upgrade/result.json)保存从实际rc.0安装到rc.1的完整更新、当前session/op maintenance ready、父App/engine0、最终实际EXE49677074…及completed receipt（kind nsis、最终setup hash026610f5…）。[085000重启probe](../tmp/tests/desktop/20261005-085000-nsis-upgrade-restart/restart-probe.json)核对更新完成0.9.0-rc.1且无自动连接，[重启退出](../tmp/tests/desktop/20261005-085000-nsis-upgrade-restart/exit.json)为App/engine0；[卸载](../tmp/tests/desktop/20261005-084500-nsis-installed-upgrade/uninstall-result.json)exit0、登记清除、数据与正式c19保留。

portable及NSIS更新的[原始数据清单](../tmp/tests/desktop/20261005-075600-typed-portable-upgrade/data-before.json)、[NSIS数据清单](../tmp/tests/desktop/20261005-084500-nsis-installed-upgrade/data-before.json)分别包含原始NEF样本hash`b20e4b1b15e1c97541befc9dbe482f15ff7dabd2a9910304527b6b0028e7c2f9`和操作员文件hash`a8a0d4128ce6b636342294b67522eee1cfb6845bb412405f063ab35c1ca4da28`；结果记录dataPreserved=true。两条升级均使用无真实设备的never-owned清理分支，不证明连接设备时更新或射线关断时序已实测。结果是本次rc.0→rc.1，不代表rc.1→stable或将来版本矩阵全部通过。

### 实际MSI安装/升级：未执行

最终实际提取、Property/Upgrade表、签名和ZIP验签均PASS；[075000实际MSI表](../tmp/tests/desktop/20261005-075000-typed-update-release/final-msi-policy-tables.json)仍明确installerExecuted=false。真实安装、rc间/rc至stable升级、实际拒降级和数据保留未做，保持PENDING，不把其他kind的安装结果借给MSI。

## 8. 已知功能和验收缺口

- 当前33项摄影参数目录及只读runtime目录是实现子集：原生LV缩放D1A3/LV焦点模式D061、9205点击AF及焦点框/方向/水平元数据、LensSort/ActiveSlot和完整LV镜头资格、自定义Picture Control名称/完整编辑尚未覆盖。电池5001和D1A4已有原始只读诊断，不再列为未实现。
- 自动包围/多重曝光尚无完整专用预检证据；单张RAM/burst和重复/异常对象拒绝不能取代全部机身功能诊断。JPEG/TIFF/RAW+JPEG多对象、Bulb/Time和电影事务仍有意禁用。
- 完整射线联动、新候选相机、长时间LV、镜头驱动、负载精度、跨HOME物理配准及重构定量精度没有由本记录认证；未知设备状态继续fail closed。
- GH signed manifest未发布，本候选实际线上查询/下载/更新传输不能说PASS；Authenticode证书未配置。
- MSI真实安装/升级以及未来rc/stable版本矩阵仍未验收。
- 安装/替换前的安全报告与installer结果、更新签名与硬件认证分别验收；保留原始NEF、断点和正式c19入口。

上述release和tmp证据是本机可核查生成物，不进入Git；本文的路径链接以当前工作区为准。对外发布与stable晋级没有在本记录中执行或确认。
