# 已安装软件联动验收与故障修正（2026-10-13）



## 结果与版本范围

- **PASS**：0.9.36、0.9.37、0.9.39 各完成四投影和10暗场+10空场，共72个NEF、1,783,149,134字节。全部生产NEF校验、独立长度/SHA-256复核及本地封存通过；各精确删除24个对应SD对象并确认消失。本地原始文件全部保留。
- **PASS**：0.9.39运动中Stop，点击到OFF读回日志51毫秒；相机1440毫秒、113次查询取得同会话关联完成，取消和关闭确认；Nano物理LOCKED，界面Stopped、任务释放、无错误。20张参考回执和一条未解决投影意图保留，不下载、不清卡、不补拍。随后重连相机、HOME和新任务预检通过。
- **PASS**：SD进度按已持久化回执增长到1/4、2/4、3/4、4/4；实测偏差告警按连续告警段去重，最新数值与重新出现的告警保留。相机曝光区三行双列及运行状态栏完整进度条保留。
- **PASS**：最终0.9.41离线门禁通过。0.9.40修正worker退出后导入总数归零；0.9.41修正已封存schema 4项目被旧读取器误拒绝。设备控制、采集、Stop和清卡沿用0.9.39实测实现。最终安装、三设备连接/OFF、真实项目读取及正常退出见下方最终运行结果。

**PASS**：最终0.9.41安装至 E:\Application\Micro-CT Workstation，应用与相邻引擎SHA-256匹配实际MSI解包结果，签名和离线更新容器通过。最终已安装界面连接三台真实设备，电量80%、Nano物理LOCKED、射线OFF。

**PASS**：0.9.41通过原生目录选择打开同一uat39-four项目，24/24文件、542,034,133字节完成读取校验，四投影载入、重建入口解锁。原始清单与NEF保持不变。关束确认后断开并正常关闭，退出码0；引擎退出记录OFF、相机清理、Nano STOP全部确认，无未解决清理。截图与读取回执见beam-validation-0.9.41/，退出证据见normal-exit-0.9.41.json。

不能据此声明硬件全面认证。本轮用4 kV/10 µA验证控制流程，ON期间实测电流接近0 µA，也记录电压瞬态偏差；有效剂量和成像效果未证实，几何精度及重建质量未验收。历史2秒/张、360张12分钟目标仍未达成，本次90°四投影组不能用于证明1°扫描提速；历史计时与性能边界见[存卡采集提速](camera-card-serial-multiexposure-2026-10-12.md)和[早释放可行性](history.md)。

## 实测计划与设备

COM9 Moxtek（VID277B/PID07D1/SN183616）、D7100 USB序列号8322769、COM7 Nano 2.3.0/build20261011。电池更换后首次读回100%，后续读回80%。源设定4 kV/10 µA，实际设定读回约3.99971 kV/9.99451 µA；USB Auto Shut Down明确关闭。联锁、温度、OFF和HOME均读取设备证据。

任务参数：1秒曝光，四投影、90°步进，480秒连续出束上限，默认SD、早释放开启；SOD50 mm、轴到探测器30 mm、有效宽40 mm、零偏移、未镜像、counterclockwise、measured，沿用已有已测几何。保留生产预检和10暗+10空参考，参考与装样提示按用户授权自动确认。

0.9.39三次预取MOVE均早于前张receipt持久化，下一次拍摄仍晚于前张receipt；末张无额外预取。投影组连续出束160.440秒，空场整组一次出束；空场OFF到投影ON约19.37秒，满足至少2秒。末张关联结束证据后OFF，随后自动导入，用时约75.3秒；没有后空场或后暗场。时序见beam-validation-0.9.39/acquisition-sequence-verification.json及states.jsonl。

## 真实失败及修正

| 问题与证据 | 修正 |
| --- | --- |
| 0.9.34包内应用嵌入引擎身份与实际引擎不符，不能正常启动 | build.rs追踪共享sidecar和孤立发布引擎；安装器提前准备引擎，并强制核验实际MSI解包应用中的引擎哈希。失败包及身份记录保留。 |
| 并行回归的早释放测试开关相互串扰 | 测试开关改为线程内RAII作用域；生产环境开关不变，默认并行workspace通过。 |
| 实际Moxtek连接拒绝被提示为相机值不支持 | 只对明确的相机参数拒绝显示该提示，其他设备使用对应设备建议。 |
| 0.9.36实机SD采集中进度一直0/4，实测偏差数值变化刷屏 | 回执持久化及释放后更新captured；连续告警按类别、设备和设定去重，实时health及安全监测保持。 |
| 0.9.37 Stop清理收到WPD DeviceBusy后立即隔离，退出码1 | 在原曝光+2秒时限内重试PTP Busy和WPD DeviceBusy；仍要求同会话、同目的地完成证据。 |
| 0.9.38清理实测成功，但协调器提前把cleanup_pending判为Fault | OFF和STOP后有界等待原会话清理终态，再读取最新射线证据；超时、未知、换会话仍Fault。回归修复前FAIL、修复后PASS；0.9.39实机Stop通过。 |
| 新任务混入旧导入计数；worker退出后recordCount归零，详情分母为0 | 准备、路径变更、项目载入和任务关闭清除旧报告；worker退出后沿用导入reporter中的已验证总数。生产协调器回归修复前FAIL、修复后PASS；新任务计数仍为0。 |
| 0.9.40实际打开本轮已封存schema 4项目，旧重建读取器误报未封存 | 共用读取器接纳schema 4；复用acquired/verified封存规则，并要求全部回执唯一、完整且绑定对应本地文件。未导入、缺记录、重复、路径/hash/大小不一致仍拒绝，全部输入继续独立验证。旧schema 2/3兼容。单色、彩色、项目读取共用门禁。 |

前序续办已保留的修正：双卡overflow按原生剩余容量规划可拍槽；首启接纳合法cardAcquisition/earlyRelease字段；D7100只开放有证据的SD(0)/Host RAM(1)；导入仅保留日期明确且严格早于本次回执的旧未认领对象，本次多余、等时/未知日期和关联歧义仍拒绝。

## 最终门禁与证据

证据根：.cache/tests/ct-engine/20261013-installed-hardware/，发布证据：.cache/tests/release/0.9.41/。完整机器核验见final-verification.json。

| 检查 | 命令/证据 | 结果 |
| --- | --- | --- |
| Rust默认并行 | cargo test --offline --manifest-path backend/Cargo.toml --workspace；workspace-0.9.41.log/.exit | PASS：489引擎库+2主程序+56桌面，547通过，10忽略 |
| 前端/桌面合同 | npm.cmd run test:frontend；npm.cmd run test:desktop | PASS：202/27 |
| 类型/模块/差异 | npm.cmd run typecheck；module-map:generate；module-map:check；git diff --check | PASS |
| 打包/构建/签名 | node tools/release/build-update-release.mjs；node tools/release/sign-update-release.mjs | PASS；包含npm构建、实际MSI解包身份及离线更新容器检查 |
| 实际安装/启动 | msiexec /i MSI /qn /norestart INSTALLDIR与日志；已安装应用CDP+Windows原生目录选择 | PASS，安装和正常退出均为0 |
| 联动/停止 | beam-validation-0.9.36、0.9.37、0.9.38、0.9.39 | 正常扫描及0.9.39 Stop PASS；早期真实FAIL保留 |
| 原始文件保护 | all-full-scan-files-final-check.json、original-nine-final-check.json、protected-final-check.json | PASS：72+9个NEF一致；旧task6清单、日志和生产任务登记哈希一致 |

离线门禁本身为**仅离线验证**。本报告明确列出的连接、HOME、拍摄、联动ON/OFF、自动导入、清卡和Stop有真实设备证据；数值重建和有效剂量不在本轮通过范围。未远程发布。

## 保留与清理

前序相机组件九张原始NEF（261,851,558字节）再次逐个复核；当时九个对应SD对象精确删除，另140个旧对象按完整前后枚举保持一致。固定九对象的ignored验收入口是一次性记录，未重跑、未用于后续清卡。
