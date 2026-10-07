# 工作站 rc2 独立验收记录（2026-10-05）

对象：`0.9.0-rc.2` Windows x86_64 candidate。本文绑定实际产物和分项证据，保留[rc1历史记录](workstation-restructure-acceptance-2026-10-05.md)及失败过程。编写本文不连接设备、不改源码或签名产物。离线门禁为**仅离线验证**；实机PASS只适用于下面明确范围，不代表完整CT认证。

## 1. 最终身份

| 对象 | SHA-256 |
| --- | --- |
| 最终portable，27,412,480 bytes | `88d79ee854c9197757c59f9f990d1c9cdc4d60dfe738565103f8d7cfe7bb0efb` |
| 内嵌及实际运行engine | `7610bd72d4d9542305796339222b329eff424f55dc08f559c0b38c41e9fce137` |
| NSIS，226,392,448 bytes | `7c3a4d2427cf1a56ec02b6337bc6e2bf046d545ec17e4ffff0f08d5743ef3d4c` |
| MSI，227,590,144 bytes | `d0531079e56adecb82e0d7261abf07ae2d5e8d0cb23be46975ec1fb3a092ca25` |

来源：[build-info](../release/0.9.0-rc.2/build-info.json)、[SHA256SUMS](../release/0.9.0-rc.2/SHA256SUMS)、[最终相机运行身份](../tmp/tests/desktop/20261005-140000-rc2-photo-preview/process.json)。源码提交基线为`11eed6cfea8f3f5118fcd07e70319f98fc398b51`且构建时dirty，不能把提交号单独当作全部源码身份。正式入口原c19包继续保留，不自动以候选替换。

## 2. 当前分项结论

| 项目 | 状态 | 范围 |
| --- | --- | --- |
| 字体布局离线11 case | PASS | 无overflow/page error，不发设备命令 |
| 最终portable实际字体及独立照片 | PASS | 140000绑定最终88d包；照片预览6000×4000，保持/关闭/返回及参数恢复 |
| 同engine的30秒Live View | PASS（旧桌面封装分项） | 132000封装f61b、相同7610 engine；283 visible，首帧后missing0，实时像素文件0；未在最终88d包重复LV |
| 原132000照片descriptor预览 | FAIL（保留） | 完整NEF已保存，但COMMAND_PAYLOAD_MISMATCH使预览未解码；134000修复合同，140000最终重测PASS |
| 最终离线门禁 | PASS | 247 library＋2 main＋50 desktop、157 frontend；typecheck/module map全绿；ignored不计PASS |
| 最终重签及验证 | PASS | 140000-rc2-sign；签名验证不等于设备认证或线上发布 |
| 实际portable rc1→rc2及重启 | PASS | 142000更新、143000重启；completed receipt、真实NEF及operator数据hash保留，父/engine/重启exit0，notice成功、error=null |
| 实际NSIS rc1→rc2升级、重启及卸载 | PASS | 145000更新、145800重启；登记与安装EXE匹配、数据hash保留、never-owned/ready维护证据及completed receipt；隔离安装卸载exit0、登记0、数据保留 |
| 已连接D7100更新维护清理 | PASS（相机独立分项） | 153200当前7610 engine、选定USB serial8322769，已连接/LV OFF→cameraCleanup confirmed、断开及cleanupConfirmed、维护锁拒重连、EOF0；Nano/Xray never-owned，无安装器联动 |
| MSI实际安装/升级 | BLOCKED（未测） | 构建、签名和身份不能代替安装验证 |
| profile5真实扫描 | BLOCKED | 多重曝光物理适用但不可观测，UNKNOWN，无人工覆盖 |
| 完整射线联动、负载精度及重构精度 | BLOCKED | 本次未测，不外推相机和离线PASS |

## 3. 字体、照片与失败修复链

[120547字体离线结果](../tmp/tests/frontend/20261005-120547-control-typography/result.json)覆盖11个visual case，overflowCases、deviceCommands、pageErrors均为空。实际桌面140000结果确认按钮/标签/选择器使用同一Inter系统回退字体栈，客户区1536×816。

[132000原结果](../tmp/tests/desktop/20261005-132000-rc2-camera-ui/result.json)为FAIL，不能改写成整轮PASS。其[运行身份](../tmp/tests/desktop/20261005-132000-rc2-camera-ui/process.json)绑定旧f61b桌面封装与最终相同7610 engine。LV 30.054秒、284 polls/283 visible、missingAfterFirst=0、stale=3、实时像素文件0，LV期间captureDisabled=true；这些分项有效。其保存照片预览返回`ENGINE:COMMAND_PAYLOAD_MISMATCH`，原FAIL保留。

[134000 envelope合同修复](../tmp/tests/desktop/20261005-134000-capture-envelope/result.json)离线PASS后，[140000最终重测](../tmp/tests/desktop/20261005-140000-rc2-photo-preview/result.json)绑定88d最终包，得到一个21,220,976-byte host NEF，SHA-256 `6069ac32a94cb37e779cead3edbae3bbfff127cbf9111177a53bfc3ad4875e57`；capture/session/operation关联、exposureEnded=true。内嵌JPEG预览6000×4000、error=null，photoHeld/photoReturned=true。[参数恢复](../tmp/tests/desktop/20261005-140000-rc2-photo-preview/restored.json)PASS、differences为空。该入口保证单个完整主机NEF输出，不证明一次物理曝光；没有射线或Nano动作。

## 4. 当前扫描证据边界

原rc1固定自动化缺口已由[审计FAIL](../tmp/tests/camera/20261005-fixed-automation-audit/result.json)确认，旧P0、RAM one-file、burst1及C102不证明无机内包围/多重曝光。当前profile5要求本次广告D0C0、严格UINT8 descriptor和鲜读OFF=0；D7100 ADL物理适用，未广告/读取失败仍UNKNOWN，必须OFF=4；HDR仅未广告时可用已确认NEF互斥证明，已广告ON/error/矛盾仍拒绝。旧profile1–4保持原canonical/hash和只读重构，拒绝续扫。

`multipleExposureEvidence`明确D7100物理适用但不可观测，当前没有可靠PTP OFF编码，真实扫描BLOCKED；不猜私有码、不自动清除、不把人工确认当机器OFF。用户关于另设人工验收策略的问题仍待答，当前无人工覆盖。独立自动曝光/ISO/WB照片继续开放单host输出契约，明确UNKNOWN诊断。D1B1是曝光表读数，非BKT。primary依据与完整边界见[能力矩阵](architecture/native-camera-capability-matrix.md)。

## 5. 门禁、签名与升级

[143000最终门禁](../tmp/tests/desktop/20261005-143000-rc2-final-gates/result.json)记录typecheck、module map、Rust、frontend退出0；[Rust日志](../tmp/tests/desktop/20261005-143000-rc2-final-gates/rust.log)和[前端日志](../tmp/tests/desktop/20261005-143000-rc2-final-gates/frontend.log)绑定247＋2＋50与157项。7个library ignored不计为PASS。

[140000最终重签](../tmp/tests/desktop/20261005-140000-rc2-sign/result.json)PASS；[manifest](../release/0.9.0-rc.2/release-manifest.json)及三种offline ZIP为对应分发类型。签名/离线验证不证明Authenticode或GitHub线上已发布。

portable[142000完整结果](../tmp/tests/desktop/20261005-142000-rc1-rc2-portable/result.json)PASS，备份rc1 SHA-256为`fc2d9c8f07e7a3e4a2054bb0342756d6c3c348fff194dc1292d4414966e4e6eb`，实际目标为最终88d包；receipt completed、exitCode=0。实际NEF及operator数据hash保持一致。维护报告匹配会话/操作，ready、maintenanceLocked、noActiveTransactions成立，三设备均never-owned；父App/engine及重启App/engine退出0。[143000重启probe](../tmp/tests/desktop/20261005-143000-rc2-upgrade-restart/restart-probe.json)确认currentVersion=rc2、kind=portable、notice“已核对更新完成”、error=null；无自动连接、LV或运动，beam仍unknown，taskLock unlocked。成功notice不是错误，never-owned分支不证明真实已连接设备的停机。

NSIS[145000完整升级结果](../tmp/tests/desktop/20261005-145000-rc1-rc2-nsis/result.json)PASS：rc1→rc2、实际安装EXE SHA-256 `1f9fe4eea0b929fa7a29175a9be00a8b32e2835acd5e02a81fd72319473e6cf3`、registrationMatches=true、同一原始数据hash保留。维护报告匹配会话/操作、never-owned/ready、maintenanceLocked及noActiveTransactions成立；安装receipt completed/exitCode0，父App/engine正常退出0。[145800重启probe](../tmp/tests/desktop/20261005-145800-rc2-nsis-restart/restart-probe.json)确认kind=nsis、rc2成功notice、error=null、无自动设备连接/LV/运动，画布四边铺满1536×816客户区；[重启退出](../tmp/tests/desktop/20261005-145800-rc2-nsis-restart/exit.json)App/engine均0。[隔离安装卸载结果](../tmp/tests/desktop/20261005-145000-rc1-rc2-nsis/uninstall-result.json)PASS、exitCode0、登记数量0、dataPreserved=true；只卸载本次隔离安装，不删除原始数据。

### 5.1 已连接真实相机的维护门禁

[153200独立结果](../tmp/tests/camera/20261005-153200-connected-update-cleanup/result.json)PASS，[进程身份](../tmp/tests/camera/20261005-153200-connected-update-cleanup/process.json)绑定当前engine `7610bd72d4d9542305796339222b329eff424f55dc08f559c0b38c41e9fce137`。明确选择D7100 USB设备序列8322769（原生PTP serial返回`83227690000000000000000000000000`）；[维护前快照](../tmp/tests/camera/20261005-153200-connected-update-cleanup/before-update.json)connected=true、Live View=false。`prepare_software_update`取得本次session/op匹配的ready、maintenanceLocked、noActiveTransactions及cameraCleanup=confirmed；[维护后快照](../tmp/tests/camera/20261005-153200-connected-update-cleanup/prepared-update.json)相机会话已断开、cleanupConfirmed=true。维护锁下重连被`SOFTWARE_UPDATE_MAINTENANCE_LOCKED`拒绝，[EOF退出](../tmp/tests/camera/20261005-153200-connected-update-cleanup/exit.json)为0。

这是已连接相机的实际maintenance门禁分项，独立于上述两轮仍为全never-owned的升级验收。本轮Nano/Xray仍never-owned，没有LV启动/帧、拍摄、属性setter、Nano运动、射线或installer操作；不证明真实Xray OFF、Nano STOP或设备已连接时的安装器完整联动。

MSI仅验证提取、包身份和签名，真实安装/升级未测。全部原始扫描数据、既有NEF、未完成任务和rc1历史证据保留。

本机`tmp/`过程证据不进入Git。本文文档门禁：`npm.cmd run module-map:check`、本地引用存在性及`git diff --check`，不执行任何设备动作。
