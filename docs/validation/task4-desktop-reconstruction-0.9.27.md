# task4 实际桌面展示与 0.9.27 安装器

## 结果

PASS，**仅离线验证**。已将本轮重建、材料分解后端、单色/多色选择、叠层预检和项目加载进度封装到 0.9.27。实际运行最终 MSI 中提取的应用及其原生引擎，打开 task4，核验 380 个输入文件，再通过软件的多色重建入口核验并显示既有封存的绿/蓝结果缓存。没有重新采集或重新计算整套体积；没有替换系统已安装应用、连接设备或发布版本。

BLOCKED：task4 的定量双基材料分布与唯一材料识别。此前 Al–Cu 主响应假设仅解释 831/31,784 条采样射线（2.61%），探测器响应尚未实测校准。现有三维图的颜色表达闪烁体通道响应，图例保留 `MATERIALS UNVERIFIED`；不能将其解释为铝、铜或其他已确认材料的空间分布。

## 安装器身份

- 交付：`release/micro-ct-workstation_0.9.27_windows_x86_64.msi`，17,265,079 字节。
- MSI SHA-256：`60a1d3ca2325f5e6d95bea3d48c360071a6bbc85891ae91236404ec6c5aad05a`。
- 包内应用 SHA-256：`a4299769109ba6bfb8171b6cb350ec9e85908bf4f4e5f4cbc1c71591583397b0`。
- 包内引擎 SHA-256：`e6b47f124e069b85b2eee3634cba7969bbb3e8254f611150e5b8a497ea174295`。
- 构建记录为本工作目录的已有改动集合，`build-info.json` 明确记录 dirty 状态、源代码树哈希及 `hardwareCertified: false`。MSI、更新容器和清单签名及本地验证通过；签名不代表硬件认证。

证据目录：`.cache/tests/release/0.9.27/`。历史 0.9.25、0.9.26 构建身份及失败展示证据保留，最终交付使用 0.9.27。

## 实际软件操作

1. 离线提取 MSI，核对应用、引擎和最终交付 MSI 的 SHA-256；应用与引擎的来源均为该包。
2. 以独立的任务注册目录和 WebView 数据目录启动正常可见窗口，通过实际 Tauri WebView 操作 Reconstruction Controls，向原生引擎打开 `E:/Main/Pictures/Camera Roll/task4`。
3. 实际读取 10,326,360,061 字节：360 个投影、10 个暗场、10 个空场；最终引擎回执为 380/380、100%。轮询记录约 43.65 秒，实测 ETA 最高 48 秒。此时间只代表本次磁盘缓存状态，不能作为固定加载时长。
4. 选择 `Multicolor`，点击 `Start reconstruction`；引擎确认缓存完成，来源为 `reconstruction/spectral/v2-04ec1329e2f55b770ca0`。界面实际渲染切片及三维 MIP，再拖动三维视图保存第二个角度。三个切片的非灰度像素数为 255、2862、2693；此检查只证明颜色实际显示，不是材料准确性验收。
5. 全程断言相机、转台未连接、射线输出为 UNKNOWN，操作回执仅包含项目加载、元数据检查和离线重建准备。manifest 前后 SHA-256 一致，页面错误数为 0。
6. 使用窗口关闭消息正常关闭本次应用；应用与其所属引擎退出码均为 0。没有终止其他软件实例。

本轮实包展示发现离线历史项目误显示 Fault：前端的首次启动判定把历史采集数量、项目操作回执及成功重建日志当作设备会话证据。修复只接受当前引擎会话中已完成的项目校验回执和一致的封存数量，复用既有设备安全检查；显示 `Stopped / Offline project view`，设备状态继续 UNKNOWN。过期状态、曾连接设备、故障、出束和不一致数据仍保留 Fault，设备准入逻辑不变。0.9.27 第一次展示脚本误要求隐藏的完整帮助文本可见，已改为检查该文本存在及误报不存在；这是验收脚本修复，应用构建未改变。

## 视觉证据

以下文件均位于本轮证据目录：

| 文件 | 含义 |
| --- | --- |
| `task4-native-multicolor.png` | 最终 0.9.27 实际软件完整窗口，绿/蓝响应切片与三维图、360/360、离线状态 |
| `task4-native-result.png` | 同一实际页面的结果区域截图，无合成数据 |
| `task4-native-rotated.png` | 实际拖动三维图后的完整窗口 |
| `task4-real-loading.png` | 本次真实 10.3 GB 输入校验期间的进度条及 ETA |
| `task4-al-cu-diagnostic.png` | 此前 Al–Cu 双基拟合诊断的原图副本：通道轨迹、等效双基散点与候选残差；不是材料分布图 |

诊断图来自 `.cache/tests/ct-engine/20261009-task4-unknown-materials/final/spectral-hypotheses.png`，原图和副本 SHA-256 均为 `cb710b73c32f1a487676c4b6a1b2a3775fb8ec469e802113fc819417480ad90a`。没有按用户透露的对象名称选择候选或修改拟合结果。数值及响应敏感性详见 [未知材料推断尝试](task4-unknown-material-inference-2026-10-09.md)。

## 门禁与剩余边界

| 状态 | 命令 / 检查 | 证据 |
| --- | --- | --- |
| PASS | `npm.cmd run installer:build` | `installer-build.log`；0.9.27 实包、21 个安装器对话框、200 个控件 |
| PASS | `node tools/release/sign-update-release.mjs` | `signature-verification.log`；本地签名与更新容器验证，无发布 |
| PASS | `npm.cmd run typecheck` | `typecheck.log` |
| PASS | `npm.cmd run test:frontend` | `frontend.log`；192 项 |
| PASS | `npm.cmd run test:desktop` | `desktop.log`；27 项 |
| PASS | 最终离线反馈专项 | `offline-project-feedback.log`；17 项 |
| PASS | `cargo test --manifest-path backend/Cargo.toml --workspace -- --test-threads=1` | 本轮 [加载进度验收](scan-project-loading-progress-2026-10-10.md) 的工作区门禁：349 引擎、2 入口、56 桌面；随后后端只更新版本号 |
| PASS | `pwsh -NoProfile -File .cache/tests/release/0.9.27/launch-final.ps1` | `task4-native.log`、`task4-native-report.json`、`task4-loading-progress.json`、`task4-final-snapshot.json`、`native-task4-final/startup-exit.json` |
| PASS | `npm.cmd run module-map:generate`、`npm.cmd run module-map:check`、`git diff --check` | `module-map.log`、`diff-check.log` |
| BLOCKED | 定量材料图与唯一类别 | 实测组装探测器响应尚未校准，原双基拟合有效性与歧义门槛未通过 |

有效的下一步是扫描通用基材料模体，校准组装探测器的响应与串色，不要求提供未知样品的材料标签。软件安装升级、真实设备和材料定量精度未在此次离线展示中验收。
