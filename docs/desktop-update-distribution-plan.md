# Windows 更新与分发工程方案与实施记录

研究基线日期：2026-10-03；实施准备修订：2026-10-04；源码实施状态更新：2026-10-05。初始研究阶段只读取活动源码、官方资料和公开元数据，当时尚未修改生产源码或生成签名材料。这段历史说明不能代表当前实施状态；当前状态见第0节，实际构建、安装及升级结果由Root在验收后补录。

已收敛的目标：安装版提供NSIS setup.exe及MSI（至少NSIS必交付），便携版保持独立EXE；检查及后台自动下载，与用户确认安装分开；所有形态都验证签名release-manifest和artifact；设备活动前置拒绝，取得ct-engine权威清理确认报告且owned engine正常exit0后才启动安装/替换；国内HTTPS两层回退、用户已授权的公开GitHub镜像及离线签名包导入。

**已实施路线：自有Rust HTTPS传输＋Minisign双签名/强制version绑定＋同EXE helper，统一在线/离线/NSIS/MSI/portable。官方CLI精确固定2.12.1负责构建签名，不添加tauri-plugin-updater或process插件。** 原文中的官方updater API作为来源/边界分析保留；本产品运行机制由活动源码实现。当前开发候选版本为 **0.9.0-rc.2**；已签名rc1与正式入口保留。

## 0. 当前实施与实际验收状态（2026-10-05）

当前候选已完成`0.9.0-rc.2`构建、重签及portable/NSIS的真实rc1→rc2升级：Cargo/package/锁文件/Tauri版本一致，MSI账本新增`0.9.2`且UpgradeCode不变。发行工具按合法SemVer和公开单调账本验证版本。本文本轮只同步既有证据，不执行构建、签名、安装或设备动作；下面0.1为原rc1历史记录，当前rc2证据见0.2及[rc2独立验收](workstation-restructure-rc2-acceptance-2026-10-05.md)，不由rc1外推。

更新状态DTO新增独立`notice`：只有结构化成功回执、安装器成功退出或便携替换、真实新启动身份/版本及必要SHA匹配后才生成成功notice；失败、未完成或回执不一致保持error。UpdateDialog将notice呈现为普通status、error呈现为alert；新check/import/cancel清理对应旧反馈。rc1实际升级发现的“已核对更新完成”被放error问题由rc2修复，签名私钥和信任公钥保持不变。

### 0.1 已保留的rc1实际验收（历史，2026-10-05）

已交付的`0.9.0-rc.1` candidate和正式0.8.2 c19入口保持原字节。rc1的[build-info](../release/0.9.0-rc.1/build-info.json)记录portable `fc2d9c8f07e7a3e4a2054bb0342756d6c3c348fff194dc1292d4414966e4e6eb`；实际NSIS app `49677074d443059dfb80597701226dfeffbd1b371c64cd8ffd7811e5fb29f4fc`；engine为`f433fde0fb94971e9ecc64d5628541e708e043487d7d942f01a3f6ff6508d1b4`。详细分项与来源见[工作站汇总验收](workstation-restructure-acceptance-2026-10-05.md)。生成物`hardwareAcceptance`保留“仅离线验证”，安装升级或同engine Nano分项证据不升级为完整硬件认证。

| 项目 | 源码与已执行检查 | 当前状态 / 边界 |
|---|---|---|
| 版本与依赖 | App统一0.9.0-rc.1，CLI精确2.12.1、锁文件固定 | PASS；不是硬件认证 |
| Rust更新服务 | 受限匿名HTTPS、分层回退、Minisign数据/global及版本绑定、防降级、ZIP_STORED离线导入 | 离线PASS；GH signed channel manifest未发布，实际线上本候选更新传输不能说PASS |
| typed维护入口 | `prepare_software_update`/cancel使用真实JSONL typed payload；匹配session/op报告后EOF→owned engine0 | 074000真实sidecar回归及两条实际离线升级PASS；均无设备never-owned分支 |
| 历史实际失败 | 064000旧包签名验证已过，062000导入确认失败`ENGINE:COMMAND_PAYLOAD_MISMATCH`，未取得authority | FAIL历史保留；修复typed payload后075000重建、080000重新签名；不把旧验签PASS当完整升级PASS |
| portable完整离线升级 | 内部rc.0测试基线→rc.1，原位替换/备份、completed receipt、最终fc2新启动、父/engine0 | PASS：075600/080500；NEF和操作员文件hash保留，无自动连接 |
| 最终NSIS安装、启动、卸载 | 实际安装EXE49677074…、安装登记、kind nsis、正常退出、卸载登记清除 | PASS：081000/081500；正式c19保持 |
| NSIS完整离线升级 | 实际内部rc.0安装基线→rc.1、authority、installer、重启核对、completed receipt、卸载 | PASS：084500/085000；数据hash保留、无自动连接、父/engine0 |
| MSI发行包 | 实际提取标记、Property/Upgrade表、ProductVersion0.9.1/固定UpgradeCode、签名/ZIP验签 | 该范围PASS；真实安装/升级未执行，不借portable/NSIS结果扩大 |
| 最新全部门禁 | 083000：136前端、243领域＋2入口＋45桌面、typecheck、module-map | PASS；7领域ignored不计PASS |
| Windows发布者签名 | 未配置Authenticode证书 | 未认证；Minisign不替代Authenticode或硬件认证 |
| 当前相机与联动 | 084000 PnP无Camera；旧3b2包Nano运动所用engine与final相同 | Camera BLOCKED等待用户开机；最终fc2 EXE未直接运动复测，完整射线联动未认证 |

证据：[062000失败记录](../tmp/tests/desktop/20261005-062000-portable-upgrade/actions.jsonl)、[074000回归](../tmp/tests/desktop/20261005-074000-update-envelope/result.json)、[075000构建](../tmp/tests/desktop/20261005-075000-typed-update-release/result.json)、[080000签名](../tmp/tests/desktop/20261005-080000-typed-signed-release/result.json)、[075600 portable升级](../tmp/tests/desktop/20261005-075600-typed-portable-upgrade/result.json)、[080500重启](../tmp/tests/desktop/20261005-080500-portable-upgrade-restart/restart-probe.json)、[081000安装](../tmp/tests/desktop/20261005-081000-final-nsis-install/install-result.json)、[081500启动](../tmp/tests/desktop/20261005-081500-final-nsis-start/restart-probe.json)、[084500 NSIS升级](../tmp/tests/desktop/20261005-084500-nsis-installed-upgrade/result.json)、[085000重启](../tmp/tests/desktop/20261005-085000-nsis-upgrade-restart/restart-probe.json)、[083000门禁](../tmp/tests/desktop/20261005-083000-typed-final-gates/result.json)、[084000设备枚举](../tmp/tests/desktop/20261005-084000-current-device-enumeration/result.json)。

正式便携EXE仍为0.8.2，SHA-256仍为`c19b828d8f18d13d3ef2811b95b5094863aee0e494611140001bc69716c91334`。原0.8.2不含更新器，首次0.9迁移仍需人工安装/替换；本次内部rc.0→rc.1实际升级PASS不能改写为原正式0.8.2已具备应用内更新。

最终安装器`bundle.windows.allowDowngrades=false`与更新器SemVer拒降级保持一致；实际MSI表存在拒降级条件，真实手工降级/将来版本矩阵尚未验收。直接复制旧EXE或先卸载再安装旧版不属于更新管线授权。

发现服务指向公开Release的candidate/stable固定通道下的`release-manifest.json`及`.sig`，artifact指向不可变`v<SemVer>`资产，不用`releases/latest`猜candidate。通道未发布时，HTTP/网络错误显示检查失败；历史镜像探针和旧正式包下载不替代本候选在线signed channel验证。当前没有上传channel manifest或发布stable。

### 0.2 当前rc2事实与证据（2026-10-05）

最终portable SHA-256 `88d79ee854c9197757c59f9f990d1c9cdc4d60dfe738565103f8d7cfe7bb0efb`，当前engine `7610bd72d4d9542305796339222b329eff424f55dc08f559c0b38c41e9fce137`，构建来源见[rc2 build-info](../release/0.9.0-rc.2/build-info.json)。原rc1、正式c19及全部原始数据保留。

| 项目 | 当前状态 | 本次证据与边界 |
| --- | --- | --- |
| portable rc1→rc2/重启 | PASS | [142000](../tmp/tests/desktop/20261005-142000-rc1-rc2-portable/result.json)、[143000重启](../tmp/tests/desktop/20261005-143000-rc2-upgrade-restart/restart-probe.json)：备份rc1、目标最终88d、NEF/operator hash保留、completed receipt、父/engine/重启exit0、notice成功/error=null |
| NSIS rc1→rc2/重启/隔离卸载 | PASS | [145000](../tmp/tests/desktop/20261005-145000-rc1-rc2-nsis/result.json)、[145800重启](../tmp/tests/desktop/20261005-145800-rc2-nsis-restart/restart-probe.json)、[退出](../tmp/tests/desktop/20261005-145800-rc2-nsis-restart/exit.json)、[卸载](../tmp/tests/desktop/20261005-145000-rc1-rc2-nsis/uninstall-result.json)：installed EXE/登记匹配、原数据hash保留、receipt completed、父/engine0、kind nsis/notice/error null，无自动设备，卸载登记0且数据保留 |
| 更新前维护authority | PASS（never-owned范围） | 上述两轮匹配session/op、ready/maintenanceLocked/noActiveTransactions，三设备never-owned；不认证真实已连接设备的维护关束/停止 |
| 已连接D7100维护清理 | PASS（独立相机分项） | [153200结果](../tmp/tests/camera/20261005-153200-connected-update-cleanup/result.json)、[EOF退出](../tmp/tests/camera/20261005-153200-connected-update-cleanup/exit.json)：当前7610 engine、USB serial8322769、before.connected=true/LV=false；匹配session/op的ready/maintenanceLocked/noActiveTransactions、cameraCleanup confirmed、相机断开且cleanupConfirmed，维护锁拒重连。Nano/Xray仍never-owned，无installer联动 |
| 最终包字体及独立照片返回 | PASS（分项） | [140000](../tmp/tests/desktop/20261005-140000-rc2-photo-preview/result.json)：实际字体、21,220,976B完整host NEF、6000×4000内嵌预览、保持/返回、参数恢复；一个host文件不证明一次物理曝光 |
| 30秒LV直接显示/不落盘 | PASS（旧桌面封装，同engine分项） | [132000原结果](../tmp/tests/desktop/20261005-132000-rc2-camera-ui/result.json)：f61b桌面封装、相同7610 engine，283 visible/missingAfterFirst0/像素文件0；原descriptor预览FAIL保留，134000修复后140000最终包照片PASS，未说最终88d重复LV |
| 当前固定扫描 | BLOCKED | profile5已补C0/ADL/HDR严格proof，旧1–4原hash只读拒续扫；Multi物理适用但不可观测UNKNOWN，无人工覆盖，人工策略待用户；完整射线联动未测 |
| rc2门禁及重签 | PASS（仅离线验证） | [143000门禁](../tmp/tests/desktop/20261005-143000-rc2-final-gates/result.json)：247library+2main+50desktop/157frontend，typecheck/module map；[140000重签](../tmp/tests/desktop/20261005-140000-rc2-sign/result.json)PASS |
| MSI rc2包/真实安装 | 提取/身份/签名范围已验证；实际安装升级未测 | ProductVersion0.9.2；不借NSIS/portable扩大；Root另行处理真实MSI验收 |
| signed candidate/stable在线channel | BLOCKED（未发布） | 没有当前rc2在线manifest传输验收；历史公开mirror下载不替代，未来发行矩阵待实际版本发行再验，不造未来版本填PASS |

旧Nano运动证据绑定f433 engine/旧3b2桌面，不绑定当前7610 engine；当前相机已返回并有上述照片证据，0.1的“相机缺席”仅为当时状态。Authenticode未配置、负载/重构精度及完整射线联动未认证。详细哈希、失败链与边界见[rc2验收](workstation-restructure-rc2-acceptance-2026-10-05.md)。

153200[维护前](../tmp/tests/camera/20261005-153200-connected-update-cleanup/before-update.json)/[维护后](../tmp/tests/camera/20261005-153200-connected-update-cleanup/prepared-update.json)证明当前真实D7100从已连接、LV OFF到确认清理并断开，重连被`SOFTWARE_UPDATE_MAINTENANCE_LOCKED`拒绝，EOF0。该独立scope没有LV/帧、拍摄、属性setter、Nano运动、射线或安装器，不把它追加成两轮upgrade的设备连接证据，也不声称真实Xray OFF/Nano STOP通过。

## 1. 历史研究基线：仓库事实（2026-10-03）

以下各项描述初始研究时的事实；当前源码版本、更新器与发布工具以第0节和活动源码为准。

- remote：`https://github.com/Holy1one/Micro-CT-Workstation.git`，公开仓库、默认 main。GitHub workflow API 返回 **0**；当前工作树无 `.github`。
- GitHub 最新正式 release **v0.8.2**（2026-10-01），附件只有 `build-info.json` 和 `micro-ct-workstation-portable.exe`，没有 updater manifest 或签名。[现有 Releases](https://github.com/Holy1one/Micro-CT-Workstation/releases)
- 研究时Cargo workspace、package.json、tauri.conf.json均为 **0.8.2**。锁文件Tauri **2.11.5**，本地CLI **2.11.4**；当时只注册dialog，尚无公钥或更新endpoint。
- 研究时portable/candidate脚本均调用`tauri build --no-bundle`，复制单EXE并生成build-info；当时配置写targets=all，原脚本并未产生NSIS/MSI。
- EXE内嵌 engine；运行时按 SHA-256写入 `%LOCALAPPDATA%/MicroCTWorkstation/runtime/ct-engine-<hash>.exe`，现有 EngineClient 明确关闭 stdin并等待正常 exit，未确认时不 kill。
- 数据默认 `%LOCALAPPDATA%/MicroCTWorkstation`，不在 EXE目录；原始扫描文件可能在用户指定的其他位置，更新不得处理为安装资源。

原正式0.8.2没有updater能力，**首个带更新器的版本需要一次人工安装/替换**，之后才能通过Help → Check for Updates使用应用内更新。新版本号必须统一增加；不能用原0.8.2版本号覆盖不同字节来期待版本比较发现更新。

## 2. 官方支持边界与推荐包装

| 分发形式 | 研究基线状态（2026-10-03） | 官方 updater 安装含义 | 方案 |
|---|---|---|---|
| NSIS setup EXE | 尚未构建 | 启动 NSIS 安装程序 | 首选普通安装版，默认 current-user；使用 passive 安装反馈 |
| MSI | 尚未构建 | 启动 Windows Installer | 纳入安装版交付计划；单独 msi target/安装身份，不与NSIS交叉替换；NSIS为首期最低必交付 |
| 现有单 EXE便携版 | 已有正式入口 | 不是原地替换便携 EXE 的实现 | 签名下载后交给本产品受控 helper 原地替换；不得把普通EXE当setup安装器 |

官方 v2 updater 使用普通 NSIS/MSI安装器及 `.sig`。[Updater](https://v2.tauri.app/plugin/updater/) 本路线仍构建这些标准安装器，但在最终文件名/Authenticode处理完成后用CLI2.12.1统一手签。因此不需要为应用配置 `plugins.updater`，也不依赖 `createUpdaterArtifacts` 自动签名；不存在未启用的plugin开关可被当成保护措施。

Windows安装包可由本机Windows构建 NSIS/MSI；WebView2默认安装方式需联网，国内/离线交付可准备包含其离线安装器的 NSIS包；单EXE便携版仍需已有WebView2，不能把“免安装App”误说成“无WebView2依赖”。[Windows分发](https://v2.tauri.app/distribute/windows-installer/)

两种信任独立处理：**Tauri updater签名**用于更新包真实性；**Windows Authenticode**用于EXE/installer发布者验证与Windows分发。Authenticode不能替代updater签名。[Windows代码签名](https://v2.tauri.app/distribute/sign/windows/)

推荐输出到 `release/<version>/` 进行待发布组装，保留现用正式 `portable-release/` 直到人工确认发布：

```text
micro-ct-workstation_<version>_windows_x86_64-setup.exe
micro-ct-workstation_<version>_windows_x86_64-setup.exe.sig
micro-ct-workstation_<version>_windows_x86_64.msi
micro-ct-workstation_<version>_windows_x86_64.msi.sig
micro-ct-workstation_<version>_windows_x86_64-portable.exe
micro-ct-workstation_<version>_windows_x86_64-portable.exe.sig
latest.json
release-manifest.json
release-manifest.json.sig
SHA256SUMS
build-info.json
micro-ct-workstation_<version>_windows_x86_64_<kind>-offline-update.zip
LICENSE.txt / THIRD-PARTY-NOTICES（沿既有许可收集链）
```

NSIS/MSI原始输出在 `target/release/bundle/nsis/`、`.../msi/`。便携版沿现有 `--no-bundle` 构建链，保存其未打installer bundle marker的原始产物，再独立构建安装器；发布脚本核对每种产物实际bundle身份及哈希，不能从同一缓存EXE盲目复制后凭文件名分类。Tauri官方通过打包时二进制marker识别安装类型。[bundle身份实现](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-utils/src/platform.rs#L330)

## 3. 密钥、版本绑定与元数据

官方updater2.13.1有requireSignedVersion/allowDowngrades，但其默认值/配置不是本路线的运行机制。本产品的共同验证器必须无开关地落实等价或更强条件：HTTPS与有效证书/hostname、完整数据与global签名、被签version一致、拒无version/降级、目标/类型/字节数/SHA一致。[来源边界：2.13.1 Config](https://docs.rs/tauri-plugin-updater/2.13.1/tauri_plugin_updater/struct.Config.html)

**版本绑定已确定，不再是条件猜测：** CLI2.11.4的`sign_file(secret_key,path)`只写timestamp/file，自动bundle也调用此二参函数；新稳定CLI **2.12.1**新增`sign_file(...,version)`与`signer sign --app-version`，build自动传`settings.version_string()`，trusted comment里的version受global签名覆盖。将NPM devDependency精确固定2.12.1并更新锁文件，所有手签文件均带`--app-version 0.9.0-rc.1`。[CLI2.11.4来源](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.11.4/crates/tauri-cli/src/helpers/updater_signature.rs)，[CLI2.12.1 signer](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.1/crates/tauri-cli/src/signer/sign.rs)，[自动bundle签名](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.1/crates/tauri-cli/src/bundle.rs)

所有NSIS/MSI/portable与manifest均用同一Minisign验证路径：先验证原始manifest bytes与global签名，再信任其trusted comment并要求`signedVersion == manifest.version == selectedTargetVersion`；artifact数据/global签名成功后，其signedVersion也必须等于该版本，再比对target/kind/size/SHA。任何旧无version签名拒绝。当前版本比较使用SemVer，禁止降级；跨stable/candidate仍须遵循已签channel与用户选择。

公钥长期稳定保存于发布者指定位置，并由发布者保管恢复备份；公开公钥已追踪并编入应用。不得每次构建重新生成或从下载包接受新公钥。私钥内容不出现在报告、源码、构建产物、命令输出或记忆中；2026-10-05已授权生成密钥，过程及验证状态见第0节，不在文档展示私钥。

本客户端读取签过名的release-manifest及其`.sig`，artifact使用`target=windows-x86_64`与`kind=nsis|msi|portable`独立字段。`latest.json`仅是可选官方兼容分发附件，本客户端不凭其未签字段授权安装。未识别本机分发身份返回可解释错误，不自动混装。

`release-manifest.json`（新格式方案，需版本化）建议记录：schemaVersion、SemVer、channel、target、packageKind、sourceCommit、构建时间、JSONL版本/数据兼容边界、每个artifact的字节数/SHA256/updater签名与已批准mirror URLs、实际验收范围。该manifest整体签名；客户端只接受目标/类型/签名/版本/哈希相互一致的文件。

`build-info`/SHA256单独不构成发布者真实性；它们用于来源审计、传输一致性和启动后确认。软件/包签名不产生任何硬件认证或射线权限。

## 4. 国内源、GitHub备源与回退

用户已明确授权公开国内GitHub镜像作为下载渠道。首期可以配置经过本次探针的 `https://gh-proxy.org/`，保留GitHub直连回退、可禁用/调整的provider和离线签名包；`gh-proxy.com`也是候选，但本轮其asset路径超时，不记录为已验证可用。用户控制的国内HTTPS对象存储/服务器仍是可选长期渠道，非首期必需新增基础设施。[COS官方文档](https://cloud.tencent.com/document/product/436/11142)，[OSS官方文档](https://help.aliyun.com/zh/oss/user-guide/access-oss-by-https-protocol)

provider官方文档给出的用法是将代理前缀拼到完整公开GitHub资源URL前，支持Release assets和Raw资源；更新必须指向确切JSON/签名/asset，不使用其仓库主页转换页面。[GH-Proxy官方用法](https://gh-proxy.com/docs/github-accelerator)

客户端不持有云账号AK或GitHub发布token。公开proxy使用独立匿名HTTP client，绝不携带Authorization、Cookie、Proxy-Authorization、URL中的GitHub凭证或发布token；只接受本仓公开资源与明确批准的HTTPS provider，不配置系统/global Git代理，不执行provider网页推荐的curl管道脚本。私有仓库不能借该公开通道传token获取资源。

**本次小文件核查（2026-10-03，瞬时证据）：**

| 通道 | 结果 | 文件/耗时 |
|---|---|---|
| GitHub官方public asset API，asset id603153372 | PASS HTTP200 | build-info.json，421字节，约2.04s |
| gh-proxy.org + v0.8.2公开Release URL | PASS HTTP200 | 421字节，约1.44s |
| gh-proxy.com + 同一Release URL | FAIL | 本机20s超时 |
| github.com同一Release直连路径 | FAIL | 本机20s超时；官方asset API可读 |

两个成功响应**字节完全相同**，raw SHA256=`bc947531320c81bbe8d6faaa4df6e465a075df992cd3a2939b6cddb14d3216a4`，也等于GitHub官方asset metadata的digest。JSON内声明的v0.8.2 EXE SHA256=`c19b828d8f18d13d3ef2811b95b5094863aee0e494611140001bc69716c91334`，本轮没有下载或验证该EXE。该probe只证明421字节公开metadata当前可用，不证明大包带宽、未来稳定性或签名包验证。证据：`tmp/tests/desktop/20261003-225913-public-mirror-probe/result.json`。

**2026-10-04补充大文件验证：** 对官方v0.8.2 asset `603148632`进行独立匿名HTTPS查询和一次镜像下载，`gh-proxy.org`返回HTTP200、19,849,728字节，耗时2.199秒，无重定向。下载SHA256、官方digest和现有正式EXE前后哈希均为上述`c19b828d...c91334`。未运行下载EXE、未改正式入口或系统代理。证据：`tmp/tests/desktop/20261004-095257-mirror-large-asset/result.json`。这补齐了本次实际大文件传输一致性验证，不代表未来可用性、更新签名或安装升级已通过。

公开mirror只承载传输，不成为信任根。发行信任仍是内置/长期保管的签名pubkey、签名manifest及artifact，绑定version/target/kind/hash；镜像返回内容不一致、HTML/旧包或签名失败，拒绝应用。

区分两层回退：

1. **元数据查询**：按用户配置在GitHub直连、已授权匿名公开mirror、自有HTTPS渠道之间进行有界回退；原始URL和mirror转换都只指向本仓的相同channel/manifest。官方文档简述非2xx回退；2.13.1源码还会在网络send错误后继续。但204立即表示无更新，首个有效release停止查询；不是从全部源挑最高版本。JSON语法错误也不是可依赖的自动回退路径。[2.13.1检查源码](https://github.com/tauri-apps/plugins-workspace/blob/updater-v2.13.1/plugins/updater/src/updater.rs#L580)
2. **artifact下载**：独立从当前选定下载源尝试，网络/超时/HTTP错误后使用同一公开Release asset的已批准mirror/直连。官方 Update只持一个download_url，列出两个metadata endpoint不等于有文件镜像回退。Rust分发服务保持同一version/target/kind/size/hash/signature，每次完整下载后验证；签名/哈希失败报告校验失败，不降低校验或静默换成其他发布物。公开mirror绝不附加GitHub token。[下载实现](https://github.com/tauri-apps/plugins-workspace/blob/updater-v2.13.1/plugins/updater/src/updater.rs#L777)

metadata、artifact和redirect均要求HTTPS与固定发布域名集合；配置有限超时、重试次数、最大包大小和可取消下载。将“无更新”“检查失败”“下载失败”“验证失败”分别表达，网络错误不能显示成最新。

自有双源发布先上传不可变version/hash文件并验证，再推进stable manifest指针；metadata短缓存、版本资产长缓存。公开proxy依赖第三方缓存，不能保证同步或长期可用，客户端保留直连/离线导入和禁用provider能力；对较旧/不一致metadata不做“已是最新”的无依据断言，显示查询来源及实际版本。`.com`/`.org`不应被假定为两家独立容灾运营方。

## 5. 权限与领域边界

Help → Check for Updates 通过 `src/platform` 的小型IPC facade进入自有Rust updater模块。既有capability不添加updater/process/filesystem/shell通用权限；不引入JS updater/process包。frontend只展示发行状态和发送check/download/apply命令，不直接给endpoint、任意路径、安装bytes或设备安全结论。HTTP由Rust执行，现有webview CSP的connect-src无需增加远端站点。

Tauri可以拥有检查、下载、签名校验、待安装包和安装器生命周期；**设备busy/tasklock、允许停止/安装前准备、清理确认仍由ct-engine裁决**。建议复用现有操作/epoch模型添加 `prepare_software_update` 或等价维护停机入口：

1. 引擎收到apply准备请求时原子地重新检查扫描/暂停/收尾、拍摄、运动、Live View、重构活动、其他手动busy与持久未完成任务锁；任何上述活动拒绝安装。tasklock不自动放弃、不改manifest/原始文件；要求完成或明确放弃并清理后才安装。Live View须先由操作员显式Stop，不为更新偷偷停止活动。
2. 通过前置检查后，引擎建立维护占用并拒绝新拍摄/运动/参数/开束，安全Stop/状态读仍可用；执行必要OFF/STOP/设备取消与文件/登记收尾。曾连接的射线OFF未知或任何cleanup未确认，拒绝安装。
3. 全未连接的新启动可按“从未建立会话”的确证走无设备停机；仍保持设备/射线UNKNOWN，不能要求虚假OFF，也不能借此覆盖曾连接后的未知状态。
4. **先取得权威确认、再EOF、再exit0。** Root负责`prepare_software_update → snapshot.softwareUpdatePreparation`，Tauri消费本次engineSessionId/operationId对应报告：真实OFF或明确never-owned分支、camera cleanup确认、Nano STOP/止动确认、无busy/LV/扫描/重构、事务/登记收尾完成、维护占用已锁定。任一未知/未确认或ack缺失拒绝安装；不得仅从App/engine退出码推定安全。确认后复用 `EngineClient.begin_shutdown()` + `wait_for_exit()`，owned engine exit0也是必要条件。当前源码engine停机预算65s、桌面等待90s，可复用但不能以耗时猜完成。Tauri只消费引擎authority，不自建第二套设备状态。
5. 若安装调用失败，保持设备已停止与引擎不可用状态，显示实际错误。重新启动控制服务建立新会话，禁止自动连接设备/恢复LV/开束。保存的任务、照片和扫描数据保留。

检查metadata可在busy时只读进行；用户从Help触发检查，发现更新后可自动启动可取消的后台下载，下载不调用设备或阻塞安全监测。下载和用户确认apply是分开的动作；未确认不得自动安装。下载完成回调发生在签名验证前，**只有download返回验证成功且项目manifest/hash/类型校验通过才启用apply**。

未采用的官方plugin边界：Windows`Update::install(bytes)`启动installer后`std::process::exit(0)`，其`on_before_exit`不能返回失败否决；`Update`只能由在线check取得且内部context私有。它不提供本产品所需的统一离线构造、authority门禁和缓存验签管线，因此不作为首期依赖。[来源实现](https://github.com/tauri-apps/plugins-workspace/blob/updater-v2.13.1/plugins/updater/src/updater.rs)

安装前还需持有本产品更新排他锁，并拒绝其他同目标/同数据身份App实例仍活动；不能让installer强关别人的engine。NSIS模板自身有运行进程检查，不能把这个检查替代领域停机。[NSIS模板](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi#L598)

同EXE helper默认passive启动NSIS/MSI，不传NSIS自动重启的`/R`，不自动重新启动App。首次显示“安装程序已启动”；helper等待installer退出并写receipt，下次App启动再核对真实版本/产物身份后确认完成。当前App/engine exit0不等于installer成功。

### 实机发现：exit0并非清理确认

Root在实机“3781”发现D1A4只读通知被误拒后 `camera.cleanupConfirmed=false`，正常WM_CLOSE仍得到App/engine exit0。该旧候选的Engine.shutdown返回unit、输出错误但main仍Ok，证明仅退出码不能作为更新安全barrier。

本次只读源码已看到Root后续改成 `shutdown()->Result` 与 `shutdown_exit_result` 传播错误；这不等于新的 `prepare_update/safe_shutdown_report` authority契约已经完成，也不把旧实机结果改成安全成功。后端该契约由Root/native后续实现，更新代码等待其明确报告。

报告须含本次会话/STOP ticket、维护占用和各确认项；旧failed/cleanup_unconfirmed ticket不升级为success，后续新STOP应给新operationId。全未连接例外要明确never-owned证据并保留射线UNKNOWN显示。收到report后到EOF期间继续锁住新设备命令，helper只接受本次被确认的本地update intent；没有确认ack，即便engine exit0，也不启动installer/替换。

## 6. 便携版受控替换

继续维持一个便携交付EXE。同可执行文件的早期`--apply-update` helper模式按已验证kind执行NSIS/MSI启动或portable替换；portable也可提供`--apply-portable-update`内部别名。从已验证副本启动且无WebView/engine/device初始化，维护逻辑仍是OS层，不复制领域状态。不创建新的进程功能目录，实现文件归现有 `src-tauri/src/updater.rs`。portable使用已签新EXE；NSIS/MSI可用本次受信运行EXE的验证副本作helper，不能误把setup本身当helper模式App。

- 签过名的manifest和EXE缓存到用户专属缓存；helper重新验签、version/target/kind/hash、父进程身份及目标canonical路径。
- 目标只能是本次当前EXE及已确认的同目录替换位置；不接受远端metadata或renderer给任意安装路径。
- 只有权威safe shutdown report确认、引擎exit0和父App正常退出都满足才动原EXE；同卷staging/backup与原子rename，失败保留原可用EXE，写可核查receipt。Windows锁文件/只读目录明确失败，不强删、不kill其他实例；缺ack不得因父进程/engine已退出就继续。
- 成功后由操作员启动，fresh新会话仍未连接。不得触碰扫描目录、预设/任务登记、AppData或照片目录；不自动回退到旧相机工具。

安装版与便携版统一导入离线签名包：zip只包含选定artifact、artifact.sig、release-manifest.json及manifest.sig/许可资料，不带信任私钥，不从包中更新信任公钥。导入Native file picker后由Rust限制展开路径/总大小、禁止绝对路径和`..`穿越，使用与在线下载完全相同的双签名/身份/hash/版本/防降级验证，再进入相同的用户确认和安全barrier。离线导入不绕过tasklock、LV或cleanup未知。该zip是产品离线运输容器，不是Tauri v1Compatible updater格式。

## 7. 构建/发布实施顺序

1. 先按用户要求验收C84C设备边界，由Root持有真实设备；随后实施大UI及更新逻辑。新软件继续交付离线候选，完成对应验收前不推stable。
2. 统一版本/identifier；构建便携和独立NSIS/MSI，每种形态实际启动/关闭/身份核验；保持内嵌engine SHA链和已有许可收集。
3. 如配置Authenticode，先签最终EXE/installer，再生成updater签名/manifest哈希。不得在签名后改包。
4. 运行签名/版本绑定/manifest交叉校验及适用全门禁，生成待发布文件集；先检查，后上传资产，最后更新channel指针。
5. 首期复用现有 `scripts/` 做本机Windows的prepare/verify/publish分步，并准备相同签名离线包。可选官方tauri-action，但其draft默认false、NSIS JSON偏好需显式配置；不能直接套默认自动发布。 [官方Action输入](https://github.com/tauri-apps/tauri-action/blob/dev/action.yml)

当前目录政策没有`.github`且模块归属也没有它；如选择CI，需把新顶层`.github/workflows`纳入人工目录/用途批准和模块图。无需为首期更新功能提前创建CI目录。

## 8. 文件/契约影响与验证

实施职责：本协作agent负责`src-tauri/src/updater.rs`、`src/UpdateDialog.tsx`、`src/platform/updater.ts`、更新依赖与构建脚本；main注册、App菜单/渲染接入及领域command/DTO由Root整合。以上源码、依赖和契约已实施，结果见第0节。

实施影响：`src/menuActions.ts`、App更新dialog、`src/platform` facade；`src-tauri/src` updater/helper与既有shutdown复用、Cargo/配置；ct-engine维护门禁/操作DTO；build/release脚本、版本和模块edges。设备模块继续独立，不导入Tauri/HTTP发布逻辑。

新增隐藏边：frontend→desktop update IPC、desktop→engine维护停机、release→签名/manifest契约；同任务同步模块图、CLI包和锁文件。

必要验证应复现具体风险：错签名/旧签包伪装新版本/改包hash；主源超时与匿名mirror保持相同身份且无token；204与网络失败分类；scan/busy/tasklock拒安装；cleanup未知/确认ack缺失但engine exit0/engine非0/超时均不启动installer；Stop并发和迟到结果；其他实例不被kill；便携目标锁定/只读/半途退出/rename失败保留旧包；NSIS/MSI升级与用户数据保留；原型和软件包真实Windows启动/正常关闭。设备与射线结论继续另用本次授权下的硬件证据。

## 9. 实施准备确认项与当前结论

1. 用户已授权的公开mirror可按本轮.org探针作为首期可配置渠道；不再要求先购买自有云源。后续若选择自有域名/COS/OSS，再补账号/区域/公开下载策略；实际provider URL仍应由版本化配置固定，保留直连与离线路径。
2. Root已指定长期密钥位置`%LOCALAPPDATA%/MicroCTReleaseKeys`，当前用户ACL＋DPAPI保护密码；内置公共key固定。2026-10-05已授权生成并验证，发布者仍需保管恢复备份；文档不记录私钥内容。
3. Windows代码签名证书/签名服务是否已有；没有时仍可制定签名更新链，但发行包的Windows发布者体验不同。
4. 开发候选版本已统一为`0.9.0-rc.2`，保留已签名rc1和正式入口；stable晋级和人工迁移仍需实际验收与发布决定。
5. 安装策略建议NSIS current-user为主、MSI为机构分发；如要求per-machine，应匹配管理员/管理部署和安装权限。
6. 选择本机发布脚本还是批准CI目录；GitHub发布凭证只留发布端。本环境没有gh，但公共API已完成只读核对。

文档应改：当前AGENTS/portable README的“只打tag、接收方重建而不下载二进制”需随用户的新更新/国内分发授权改为**发布签名release附件且仍不把二进制提交到Git**。候选/正式来源哈希、数据保留和硬件门禁继续保持。此研究没有修改这些产品/规则文件。

## 10. 可直接接手的最小实施清单

本节是Root于2026-10-04接受的实施路线，以下清单和命令模板保留其设计来源。2026-10-05已完成的源码、依赖、密钥及验证项见第0节；不能把本节准备阶段的未执行说明当作当前状态，最终构建/签名/安装结果由Root另行补录。

### 10.1 直接依赖与权限

| 项目 | 选择 | 用途 / 精确API |
|---|---|---|
| `@tauri-apps/cli` | 精确`2.12.1` | `signer sign --app-version`；普通NSIS/MSI bundler |
| `reqwest` | 现锁文件同版`0.13.5`，default-features=false，features=`rustls,stream` | `Client::builder().https_only(true).no_proxy().connect_timeout(...).timeout(...).redirect(...)`；`Response::bytes_stream()` |
| `futures-util` | `0.3`，锁定解析版本 | `StreamExt::next()`；不建立第二个runtime |
| `minisign-verify` | `0.2.5`（同0.2受控API系列） | `PublicKey::decode`、`Signature::decode`、`verify(...,false)`、`verify_stream`、`StreamVerifier::update/finalize`、`Signature::trusted_comment` |
| `base64` | `0.22` | `STANDARD.decode` Tauri外层base64；不自造签名格式 |
| `semver` | 现锁文件同版`1.0.28`，serde feature | 解析/比较app版本；candidate/stable规则单独明确 |
| `zip` | `4.6.1`，default-features=false | 离线包首期用ZIP_STORED，拒加密/链接/子目录；`ZipArchive::new/by_index`，白名单basename和大小检查 |
| `tokio` | `1`，只补`fs,io-util,sync,time,macros`所需features | 复用Tauri async runtime，async文件写入/超时/取消，不创建第二个runtime |
| `fs2` | 锁定`0.4.3` | App共享数据身份实例锁；helper在owned parent退出后取得排他锁，拒绝同数据身份其他实例 |
| `serde/serde_json/sha2` | 复用现有 | 明确schema、原始bytes签名、SHA-256；不把JSON重新序列化后验签 |
| `windows-sys` | 复用`0.61.2`与现有Windows features | 进程handle/exit检查、ShellExecuteExW、同卷原子替换；按实际使用补feature |

reqwest0.13的TLS feature名是`rustls`，不是旧代码常见的`rustls-tls`。无cookies feature、不设Authorization/默认凭证；mirror是普通HTTPS URL前缀而非system proxy。源和redirect host/scheme白名单在同一个transport模块落实。[reqwest0.13.5 API](https://docs.rs/reqwest/0.13.5/reqwest/struct.ClientBuilder.html)

Minisign的数据/global签名必须成功后才读取trusted version。小manifest用`verify(bytes,&signature,false)`拒legacy；大artifact用stream verifier，边写入边SHA和验签，必须`finalize()`成功；stream模式只支持现代prehashed签名，适合新的CLI2.12.1。[PublicKey](https://docs.rs/minisign-verify/0.2.4/minisign_verify/struct.PublicKey.html)，[StreamVerifier](https://docs.rs/minisign-verify/0.2.4/minisign_verify/struct.StreamVerifier.html)

**不添加**tauri-plugin-updater、tauri-plugin-process、JS updater/process绑定或对应capability。`default.json`现有window/dialog权限足够作为起点，更新文件IO/验证在Rust内完成；离线文件picker复用dialog。`plugins.updater.*`和`createUpdaterArtifacts`不是本路线安全设置，不写未采用开关为“已验证”。

### 10.2 已实施的自有函数边界

| 文件 / 函数 | 输入 | 成功输出与限制 |
|---|---|---|
| `UpdaterService::new(data_dir)` | 引擎同源数据目录、内置pubkey、受控source配置 | main注册发行状态，持有实例锁，不拥有设备状态 |
| `UpdaterService::check(channel)` | 仅stable/candidate | 验签元数据并自动后台下载；无新版本与错误分开 |
| `verify_manifest_with_key(raw,sig,public,...)`（私有） | 有界原始JSON bytes及Tauri `.sig`，仅内置公钥 | 数据/global签名→trusted version→JSON schema/identity→SemVer，产出不可由UI构造的Verified类型 |
| `download`/`verify_artifact`（私有） | 已验证manifest内的选择 | HTTPS逐源有界流→尺寸/SHA/数据/global/version一致→sync/原子提交cache；下载/验证可取消 |
| `UpdaterService::import_zip(path)` | main Native picker选出的ZIP；UI不传path | 固定文件白名单、路径/大小界限、共同验证器；不依赖network |
| `software_update_confirm_apply`（main IPC） | updateId、expectedVersion和明确用户确认 | prepare_apply复验cache→领域prepare→本次权威report→EOF/owned engine exit0→spawn_helper→App正常exit0；不重算设备安全 |
| `UpdaterService::cancel(update_id)` | 发行job ID | 取消网络/验证job；不作设备控制；preparing/applying禁止此入口取消，维护取消由领域协议裁决 |
| `run_apply_helper(job_path)` | 本地受控intent路径 | 在main最前分支，先于WebView/engine/bootstrap；复验包、等待owner退出、按kind执行 |
| `check_receipts`（私有） | 当前版本/分发身份和本次receipt | 真实启动后核对版本/目标/身份及便携SHA；“installer已启动”仍不代表安装完成 |

自有IPC拟为`software_update_check/status/cancel/import/confirm_apply`；UI不传pubkey、URL、target路径或二进制。`src/platform/updater.ts`只调用这些IPC，`UpdateDialog`显示发行状态、源/版本、进度、验证结果及用户确认；设备拒绝原因来自Root的snapshot。

原始manifest建议固定字段：`schemaVersion=1`、`appId=edu.lzu.microct.workstation`、`version`、`channel`、`publishedAt`、`sourceCommit`、`releaseNotes`、`artifacts[]`。artifact包含`target=windows-x86_64`、`kind=nsis|msi|portable`、`fileName`、`size`、`sha256`、`signature`、本仓公开asset URL；mirror前缀由受控本地provider列表组合。manifest与artifact的trusted comment version均必须等于manifest.version及本次选定目标版本，拒重复version字段和无version签名。

发行状态DTO只含`updateId/phase/currentVersion/targetVersion/channel/kind/source/downloadedBytes/totalBytes/verified/notice/error`等；不加入beam/位置/cleanup的第二份判断。Root负责`prepare_software_update`、`snapshot.softwareUpdatePreparation`及维护锁；Consumer必须核对报告的本session/op、ready、camera cleanup、Nano STOP、Xray OFF或明确never-owned、无活动/待提交事务，再`begin_shutdown`并90s等待owned engine exit0。报告空/旧/blocked/unknown与exit0组合也必须拒绝。

### 10.3 密钥生成与长期保存流程

Root指定目录：`%LOCALAPPDATA%\MicroCTReleaseKeys`。

```text
microct-updater.key              Tauri格式、带密码的加密私钥
microct-updater.key.pub          长期公钥；CLI自动在私钥路径后加.pub
microct-updater-password.dpapi   DPAPI CurrentUser保护的密码blob
```

初始化程序先核对key、公钥和DPAPI文件；已有材料禁止覆盖，优先恢复/验证。限制目录/文件ACL为当前Windows用户及必要SYSTEM，不授予Users/Everyone读取。私钥/密码恢复备份由发布者保管；DPAPI blob不能被当作任意机器上可直接解密的备份。[Microsoft DPAPI](https://learn.microsoft.com/en-us/dotnet/api/system.security.cryptography.protecteddata.protect)

首次生成始终带`-w`，不带`--force`/`--ci`；密码不进入OS进程argv。以下为官方交互模式的命令形状；2026-10-05在无交互控制台的本机环境中，`update-keygen-child.mjs`使用官方CLI2.12.1的run API，密码仅通过短命child环境进入CLI内存，避免控制台等待；既有材料均保留：

```text
node node_modules/@tauri-apps/cli/tauri.js signer generate -w <LocalAppData>/MicroCTReleaseKeys/microct-updater.key
```

官方`-w`模式只显示保存位置，不把私钥内容输出；无`-w`模式会打印私钥，因此禁止后一模式。密码DPAPI使用`ProtectedData.Protect(...,DataProtectionScope.CurrentUser)`，只在签名child进程的环境里短暂提供`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`，结束后清除；不得在密钥已加载时调用会展示env默认值的help/debug/完整环境转储。

公钥从长期`.pub`文件经格式/指纹核对后复制到可跟踪公共文件（建议现有`src-tauri/`下`update-public-key.pub`）并编入EXE；公钥可公开，私钥/DPAPI blob永不进入repo/release/tmp证据或日志。普通运行和offline包都不能替换信任根。

### 10.4 当前候选打包与手签顺序（rc2）

以下为rc2的命令模板，本轮源码修改阶段不执行构建或签名。Cargo workspace、package.json、tauri.conf.json与锁文件均统一为0.9.0-rc.2；candidate channel使用rc，保持rc1全部已签名附件及正式portable入口原hash。

1. 固定CLI2.12.1与NPM锁文件，核对`--app-version`可用。通过SemVer和MSI账本检查后只准备`release/0.9.0-rc.2/`，不覆盖`release/0.9.0-rc.1/`或正式入口。
2. `tauri build --no-bundle`沿现有beforeBuildCommand/build.rs生成前端与内嵌engine，将原始`target/release/ct-workstation.exe`复制到该release目录的portable最终文件名。便携身份须是未打installer marker的类型。
3. `tauri bundle --bundles nsis,msi`利用同一版本构建产物打标准安装器；NSIS配置`windows.nsis.installMode=currentUser`，所需WebView2策略由Root决定（国内/离线默认倾向offlineInstaller）。产物来自`target/release/bundle/nsis/`与`.../msi/`，复制到最终发布文件名。
4. 若配置Authenticode，对最终raw portable、installer及Tauri内部安装binary分别按官方bundler流程完成签名。先固定最终bytes/文件名，再Minisign；之后不得改字节或重新打包installer。
5. signer对NSIS、MSI、portable全部明确传版本；私钥通过`--private-key-path`或child环境提供，密码仅child环境，不作为参数：

```text
node node_modules/@tauri-apps/cli/tauri.js signer sign --private-key-path <protected-private-key-file> --app-version 0.9.0-rc.2 <final-artifact>
```

6. 使用已签artifact的准确size/SHA/`.sig`生成最终manifest原始UTF8 bytes（包含确定换行）；再同样`signer sign --app-version 0.9.0-rc.2 release-manifest.json`。末尾用既有公钥验证所有数据/global签名、trusted version、target/kind、SHA/size和文件来源；不打印或更换私钥。
7. 用ZIP_STORED打每个kind的离线运输包（manifest/sig、选定artifact/sig、必要license），不把key或DPAPI文件打入。复读zip并执行相同客户端验证；此包不是官方v1Compatible archive。
8. 进行对应离线门禁及每种候选的真实Windows启动/正常关闭/数据保留测试，仍以candidate交付。只有Root确认验收与发布后才上传不可变version assets，再推进签过名的channel manifest；不发布/改stable于本阶段。

RC特别注意：GitHub`releases/latest`不用于发现prerelease candidate；candidate source要使用明确配置的版本/tag或独立签名channel manifest，不能凭普通latest猜rc版本。

MSI的ProductVersion只比较前三段，第四段被Windows Installer忽略，不能靠`0.9.0.1→0.9.0.2`表达rc到stable升级。[Microsoft ProductVersion](https://learn.microsoft.com/en-us/windows/win32/msi/productversion) MSI交付前需Root确认单调numeric映射；可选`wix.version=0.8.9001`表示rc.1、后续rc递增并在stable用0.9.0（签名manifest仍写真实app SemVer0.9.0-rc.1）。这是待确认映射建议，不自动替Root决定；首期最低NSIS不被这一MSI待验项阻塞。

### 10.5 helper执行与安全边界

helper的意图文件包括updateId、本次签名manifest hash、目标kind/version、准确target/staging/backup路径、本次authority报告身份和owned app/engine进程身份。由Rust内部创建，caller只能给updateId；文件位于当前用户受控cache且helper重新验证，不能从网络metadata接受目标写路径。

- 只有matching `snapshot.softwareUpdatePreparation`确认→EOF→owned engine exit0→owner App正常退出全部满足才进行动作；helper不初始化Tauri WebView或任何设备。
- NSIS按官方受控`/UPDATE /P`模式启动、不传`/R`；如果使用`/D=`须遵守NSIS参数末尾及路径quoting规则，不能用通用字符串拼cmd/Powershell。使用Native ShellExecuteExW/进程handle保留实际安装结果；MSI调用固定SystemRoot下msiexec，参数数组为`/i <verified-msi> /passive /norestart`，管理员/拒绝/重启需要的结果如实记录。
- portable用受控新EXE helper，等待old app/engine后同卷rename替换并保留backup，失败不kill/强删；已退出也不补造缺失的authority确认。
- 如果另一实例持有同目标或数据身份，拒绝而非installer强关。签名一致不证明设备安全、硬件认证、重构精度或兼容任务可续扫。

源码实施后的针对性离线验证见第0节；历史准备阶段曾待Root执行installer/portable完整自动更新验收；当前portable/NSIS rc1→rc2已按0.2取得实际结果，MSI仍未实际安装升级。文档/API查询、旧窗口exit0、小文件mirror probe和单元测试均不能代替实际安装升级结果。

### 10.5 MSI候选与正式版的版本映射

2026-10-04补齐此前待定的rc→stable约束：Windows Installer升级只比较ProductVersion的前三个数字，第四段不能用于区分rc与正式版。因此不把`0.9.0-rc.1`截成`0.9.0`，也不依赖`0.9.0.1`升级到`0.9.0`。[Microsoft ProductVersion](https://learn.microsoft.com/en-us/windows/win32/msi/productversion)

应用、菜单、签名trusted version和release manifest继续使用完整SemVer；MSI采用独立、严格递增的安装构建序号，明确写入`bundle.windows.wix.version`。Tauri提供此版本覆盖和固定`upgradeCode`配置。[Tauri WixConfig](https://v2.tauri.app/reference/config/#wixconfig)

- 在现有`scripts/`职责内维护公开发行版本账本：每个不可变app SemVer对应一个`major.minor.installSequence`。同一major/minor的下一个较高SemVer使用下一个序号，不根据当前日期、构建次数或rc字符串临时计算。
- 首个`0.9.0-rc.1`映射为`0.9.1`。如果后续只有`0.9.0`正式版，其映射为`0.9.2`；如中间实际发行其他rc，先给这些版本递增序号，再分配正式版。该例不是对未来发布次序的假定。
- major/minor范围0–255，安装序号1–65535；拒绝溢出、账本重复、已发行版本重新分配或应用SemVer降低。major/minor增加时可以重置序号，但仍检查完整三段版本相对上一安装发行严格增加。
- MSI artifact在签名manifest中附`installerProductVersion`。构建后读取实际MSI Property表，核对ProductVersion、ProductCode、固定UpgradeCode、目标架构和app SemVer来源，然后才生成hash/签名；客户端不得把安装序号当应用SemVer或忽略被签版本。
- 首次打包前用当前产品名生成并固定UpgradeCode，升级保持不变。NSIS与portable仍使用各自身份和完整SemVer，不根据MSI序号判断更新。
- 必跑实际升级检查：rc1→rc2、最后rc→stable、stable→下一较高SemVer；验证应用版本确实改变、数据保留、旧版不重复安装。2026-10-05已创建账本并配置首个映射与固定UpgradeCode；MSI最终包生成、实际提取/表核对、签名和ZIP验证已PASS，真实MSI安装升级未执行；下表为截至rc1的历史矩阵，当前rc2结果见0.2。


#### 历史版本与分发测试矩阵（截至rc1，2026-10-05）

上文早期`wix.version=0.8.9001`是历史建议，当前不采用；公开账本把App`0.9.0-rc.1`映射为MSI`0.9.1`，rc2映射为0.9.2，UpgradeCode固定。下表保留截至rc1的原验收结果与当时待测项；其中“当前实测只到rc.0→rc.1”和“相机缺席”均为历史，不能覆盖0.2的rc2当前结果。当前portable/NSIS rc1→rc2已PASS；MSI实际安装、未来rc→stable/stable→更高版本和线上signed channel仍按0.2边界，不提前认证未来发行。

| 版本 / 分发路径 | 当前结果 | 证据及限制 |
| --- | --- | --- |
| 内部portable rc.0→rc.1 | PASS | [075600](../tmp/tests/desktop/20261005-075600-typed-portable-upgrade/result.json)、[080500重启](../tmp/tests/desktop/20261005-080500-portable-upgrade-restart/restart-probe.json)；完整离线升级、completed receipt、数据保留；无真实设备 |
| 最终NSIS rc.1安装/启动/卸载 | PASS | [081000安装](../tmp/tests/desktop/20261005-081000-final-nsis-install/install-result.json)、[081500启动](../tmp/tests/desktop/20261005-081500-final-nsis-start/restart-probe.json)、[卸载](../tmp/tests/desktop/20261005-081000-final-nsis-install/uninstall-result.json) |
| 内部NSIS rc.0→rc.1 | PASS | [084500](../tmp/tests/desktop/20261005-084500-nsis-installed-upgrade/result.json)、[085000重启](../tmp/tests/desktop/20261005-085000-nsis-upgrade-restart/restart-probe.json)、[卸载](../tmp/tests/desktop/20261005-084500-nsis-installed-upgrade/uninstall-result.json)；完整链路、数据保留，无真实设备 |
| MSI rc.1包（ProductVersion0.9.1） | 提取/表/签名/ZIP验证PASS | [实际表](../tmp/tests/desktop/20261005-075000-typed-update-release/final-msi-policy-tables.json)、[最终签名](../tmp/tests/desktop/20261005-080000-typed-signed-release/result.json)；installerExecuted=false |
| MSI实际安装、rc间/rc→stable升级 | PENDING，未执行 | 不能用NSIS或portable结果替代；需逐版验证ProductVersion、UpgradeCode、真实版本改变和数据保留 |
| 任一kind的rc.1→下一rc、最终rc→stable、stable→较高SemVer | PENDING | 当前实测只到内部rc.0→rc.1；未来MSI序号按账本分配，真实升级再验收 |
| installer真实手工拒降级 | PENDING | 更新器拒降级和MSI实际拒降级表已检查，但尚无实际降级安装结果 |
| GH signed channel / mirror在线本候选更新 | BLOCKED | manifest尚未发布；不能以历史旧包镜像下载或离线ZIP成功代替 |

内部rc.0是测试基线，不是把正式c19入口改成rc版本；首次原0.8.2迁移仍按人工安装/替换处理。Minisign验证、Windows发布者签名、设备清理authority和installer receipt是独立证据。历史rc1相机缺席不代表当前rc2；Authenticode未配置、当前Multi扫描BLOCKED及完整射线联动未认证仍保持。当前版本矩阵见0.2，不修改原rc1验收文件。
