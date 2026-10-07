# 2026-10-04 相机断电期间的分项验证

用户已确认D7100电池耗尽，本次按“先完成其他验证”执行。未尝试连接或启动相机，未连接射线源、写射线参数或开束，未上传固件。工作区继续保留原有修改及正式入口；本记录不是完整系统硬件认证。

## 验证结果

| 项目 | 结果 | 证据及实际范围 |
| --- | --- | --- |
| Nano连接、身份和台架HOME | PASS，实机 | COM7、RTS9060_NANO 2.1.0/protocol 2/BUILD20260918，8微步、96000 pulse/rev；HOME读回IDLE、0脉冲、有效参考 |
| 规定方向3°及运动中STOP | PASS，实机 | 3°读回−800；下一10°动作取消，STOP终态约62.6ms，随后STATUS为LOCKED、pos=target=−1782、reference/homed/rearmed=false、captureId=0 |
| Windows客户区铺满 | PASS，实机桌面 | 当前客户区1536×816 CSS px、DPR2.5，四边差0，无页面溢出；这是现有界面，不是待实施的新布局 |
| 当前新候选的Nano停止与退出 | PASS，实机 | 重建后仅连接/STATUS/STOP/鲜读，未再运动；实际App与内嵌引擎正常退出码均为0 |
| 普通Stop取消竞态 | PASS，仅离线 | 修前原回归第19次复现FAIL；旧capture waiter再次取消会撤销较新Stop清理证据。现仅允许当前epoch失效，保留真实未知和Close失败；确定性回归2项、相机90项、领域10项及原回归修后20次通过 |
| 完整Rust整合门禁 | PASS，仅离线 | `cargo test --workspace --offline`：222 library＋2 main＋28 desktop，共252通过；6项既有硬件测试ignored |
| 前端与模块门禁 | PASS，仅离线 | typecheck、126项`test:sites`、module-map和diff检查通过 |
| 固件重建与源码模拟 | PASS，仅离线 | Flash13416 B、RAM1382 B；实际源码host harness1149断言，未打开串口或上传 |
| 与历史flash应用区比对 | PASS，仅离线复核历史实机读回 | 今日重建与2026-10-03授权取得的完整32768 B读回比较，应用窗口[0,0x7800)代码和FF填充均0差异；本次未再读设备或复位。Bootloader/fuse边界未认证 |
| 公开镜像大文件下载 | PASS，网络传输 | gh-proxy.org一次HTTP200下载官方v0.8.2 EXE，19849728 B/2.199s；与官方digest、正式EXE前后SHA一致。未运行下载EXE；签名更新和安装流程仍未实现/验收 |

停止试验的首次探针因要求设备主动推送MOVE位置和STOP后的即时STATUS而返回FAIL，已保留`stop-during-motion.json`。该控制器不主动推送位置，STOP后旧STATUS被失效；本次验收依据同操作取消/STOP终态及明确发送STATUS所得的新读回，不把计划目标当实测。对应修正证据是`stop-confirmed-readback.json`，不是删除失败记录或将未知值填为正常。

## 交付与来源

- 分支基线：`native-device-controls-offline`、commit `11eed6cfea8f3f5118fcd07e70319f98fc398b51`及已有未提交修改。
- 当前候选：`release/native-device-controls-offline/micro-ct-native-device-controls-offline.exe`。
- 候选SHA256：`d4c0cc9b468c4c89f92d23e082dc6f8e16174c218f771b26235c9efd9319b7bf`。
- 本次真实加载的内嵌引擎SHA256：`6a3f98085d604b6051d7d3f68d513e608bd81b310205e2420eae6a630b709510`。
- 运动试验先在4d09候选/abb74baa引擎完成；之后仅改相机actor取消竞态与测试诊断。d4c0候选补验真实Nano连接、STOP、STATUS及退出，没有重复无关运动。
- 正式`portable-release/micro-ct-workstation-portable.exe`保留不变，SHA256：`c19b828d8f18d13d3ef2811b95b5094863aee0e494611140001bc69716c91334`。
- 活动固件源码SHA256：`470133efbb0f2192c725a0aef223cf6256030ce1f9c0dc541bd3cea4efa8e562`；新HEX与历史已比对HEX均为`59316e6f45bfb6449cbb44c2eb5a59e5c2bbda2d39430bf3d7cbd847987810a9`。

证据均为本机临时验收产物，不纳入版本控制：

- `tmp/tests/desktop/20261004-095000-nano-4d09/`：运动、停止、鲜读、客户区、正常退出。
- `tmp/tests/desktop/20261004-105000-d4c0-nano-smoke/`：当前候选及实际内嵌引擎、STOP/鲜读/退出。
- `tmp/tests/ct-engine/20261004-103124-ordinary-stop-race/`：失败复现、源码修复与针对性门禁。
- `tmp/tests/ct-engine/20261004-104300-stop-race-integration/`：252项Rust整合结果。
- `tmp/tests/ct-engine/20261004-101100-other-verification/`：修复前真实失败以及前端126项和模块门禁；失败记录保留。
- `tmp/tests/turntable/20261004-021518-candidate-avr-build/`、`20261004-021520-candidate-host-harness/`、`20261004-102300-rebuild-historical-flash/`：固件与历史读回复核。
- `tmp/tests/desktop/20261004-095257-mirror-large-asset/`：官方metadata、匿名HTTPS下载、大小/hash及正式入口未变。

## 剩余边界与下一步

相机充电后仍须完成当前原生候选的独立NEF单拍实机验收：确认本次关联完成事件、完整文件/hash、操作终态及清理，鲜读并恢复原记录介质0等测试前设置。本次没有拍摄照片或保存真实Live View帧。

用户要求的UI重构、顶部菜单、预检固定参数新策略、单拍登记预览、签名更新器及NSIS/MSI/便携版分发仍按[界面执行规划](workstation-ui-restructure-plan.md)和[分发规划](desktop-update-distribution-plan.md)待实施。按既定顺序，先完成相机实机验收，再进行大改；不能将本次分项PASS表示为这些功能已经实现。

真实射线、跨设备联动扫描、负载侧角度计量/回零重复性、AVR中断脉冲时序、bootloader与fuse独立认证仍未验证。`hardwareCertified`保持false，未经完整验收的候选不替换正式入口。
