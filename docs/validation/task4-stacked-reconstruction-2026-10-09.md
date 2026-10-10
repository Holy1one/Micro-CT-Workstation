# task4 叠层重建离线验收 · 2026-10-09

**仅离线验证。** 未连接设备、未拍摄、未安装或发布软件。现有工作区包含另一前端任务的改动，本轮保留其布局和已有修改。

## 实现与结果

- 预检准备弹窗新增叠层勾选及 5 行材料/发光颜色/厚度，完整行定义实际层数和入射顺序；半填行不能提交。几何字段冻结、旧字段兼容和浏览器校验一致。
- 线性 CFA 通道配准、G1/G2 合并、原生饱和传播、各波段暗/空场与噪声诊断；单色自动选参考可靠性最高的波段，多色按堆叠发光颜色选择观测。当前自动颜色流程为 FDK。
- 重构入口最后调整，提供 Monochrome/Multicolor 下拉选择及可选实测材料标定。显示维持三切片/3D布局，未标定图例标为材料未验证，MIP 按结构峰值位置取颜色。
- 双基材料查找表、实测配准和独立点误差门槛已实现；合成标定、近似相同响应、错配 profile/层/几何、越域和不合格验证均拒绝。

task4 使用前绿玻璃 1 mm、后蓝玻璃 3 mm，360 投影、10 暗场、10 空场。原 manifest 不改写；补充信息保存于 `E:/Main/Pictures/Camera Roll/task4/reconstruction/detector-config.json`，绑定 manifest SHA-256。

最新结果目录：

`E:/Main/Pictures/Camera Roll/task4/reconstruction/spectral/v2-04ec1329e2f55b770ca0/`

包含根部自动选出的绿通道结构体积、`green/` 和 `blue/` 独立科学体积与投影、质量/覆盖诊断、128³ RGB 预览及封存哈希。科学体积为带符号 float32、256³，计算采样 0.0961538 mm；此数值不能当作实测分辨率。

| 指标 | 结果 |
|---|---|
| 自动选择 | Green；参考 SNR 87.40，Blue 33.88，Red 22.42 |
| 轴校正 | 从记录的 0 mm 调整至 3.826953125 mm；独立角度门槛通过 |
| 同组固定公共射线验证误差 | 1.5340261 → 0.5473432，下降 64.32% |
| 旧轴搜索 | 2.525 mm 命中原 ±16 像素边界，被拒绝并保持 0 mm |
| 新两体积有限值比例 | 100% |
| 严格量化覆盖 | Green 25.13%，Blue 14.87%；噪声截断射线也被排除，定义比旧结果更严 |
| 原生饱和诊断 | Green 325、Blue 41 个重采样投影像素受影响；旧下采样检测为 0，二者口径不同 |
| 计算后端与耗时 | CPU，未选出经验证的 GPU；完整双波段本次测试约 462 s |

验证误差下降来自同一波段内轴调整前后，不能当作不同算法/不同通道的总体图像质量指标。新绿通道背景条纹减轻，中心环状伪影仍明显。共享几何之外，未实测校正的层深度和色差仍存在。task4 默认轴向中间切片没有通过所有角度的双通道共同量化支持，因此按设计显示灰度；其他切片及 3D 有合格区域的颜色响应。

## 门禁与证据

过程目录：`.cache/tests/ct-engine/20261009-task4-spectral/`。

| 状态 | 命令/检查 | 结果/证据 |
|---|---|---|
| PASS | `cargo test --manifest-path backend/Cargo.toml --workspace -- --test-threads=1` | 引擎 344、入口 2、桌面壳 56 项通过；8+1 项显式 ignored；`workspace-tests-serial.log` |
| PASS | `cargo test --manifest-path backend/Cargo.toml -p ct-engine reconstruction -- --test-threads=1` | 最后标定读取调整后的重构专项回归；`reconstruction-tests-final.log` |
| PASS | `cargo test --release --manifest-path backend/Cargo.toml -p ct-engine reconstruction::spectral::tests::reconstruct_task4_colour_offline -- --ignored --nocapture` | 显式 `CT_SCAN_DIR`、`CT_SPECTRAL_OPTIONS={"mode":"multicolor"}`，本次读取 task4 新结果并完整校验；`reconstruction-v2.log` |
| PASS | `cargo test --manifest-path backend/Cargo.toml -p ct-workstation task4_completed_colour_preview_reads_registered_artifacts_offline -- --ignored --nocapture` | `CT_PREVIEW_CACHE` 指向新目录；实际灰度和 RGB 预览读取、字节数、标签和哈希验证；`real-preview-test.log` |
| PASS | `npm.cmd run typecheck`、`npm.cmd run build` | `typecheck.log`、`frontend-build-final.log`；已有大 bundle 提示，不影响构建 |
| PASS | `npm.cmd run test:frontend`、`npm.cmd run test:desktop` | 188、27 项；`frontend-tests-final.log`、`desktop-tests-final.log` |
| PASS | `npm.cmd run module-map:check` | 文件归属、目录门禁和生成图一致；`module-map.log` |
| PASS | 浏览器实际组件、禁设备命令 fixture | 1920×1080、1366×768；模式命令、五行顺序、控件不越界；实际 task4 RGB 体积三切片/3D shader；`ui-qa.json` 和 PNG |
| PASS | 完成后原文件 SHA-256 核对 | 380 个 NEF、manifest 未改写；5 个旧科学成果哈希通过；`final-integrity.json` |
| BLOCKED | task4 定量材料分辨验收 | 尚无通用基材料的实测双通道响应与配准标定；待测物体无需提供已知材料/厚度。本次几何与通道基线未发布定量材料分解成果 |

首次新增几何测试的整数/浮点 JSON 比较及桌面静态检查误扫测试目录创建语句已修复。此前一次默认并行全量运行中，已有 `failed_off_confirmation_remains_unknown_and_latches_fault` 时序测试失败；单独复测和上述完整串行复测通过，未改设备逻辑。失败日志保留在 `workspace-tests-final.log`，不能视为真实设备证据。

比较图使用统一蓝通道显示窗，绿通道独立显示窗；脚本、图及数据记录于 `compare.py`、`comparison.json`。界面截图是离线组件 fixture，其中设备状态为模拟值；仅用于核验界面和实际已生成体数据的绘制，不能证明真实设备连接或扫描成功。

## 下一步

按 [材料成像策略与标定契约](../decisions/stacked-scintillator-material-imaging.md) 使用通用基材料建立系统阶梯和混合厚度网格、独立验证与配准标记。基材料用于表达未知衰减；待测物体材料和厚度均由系统拟合。玻璃组成可暂缺；预测逐层吸收/能谱时才需补充探测器成分、密度和发射/吸收资料。5 层配置不等于 5 个独立能量通道。

本记录原先把未知样品信息列为分解前提，现已更正。用户要求的完整库未知厚度拟合、Al–Cu 分解后材料匹配及结果见 [后续未知材料尝试](task4-unknown-material-inference-2026-10-09.md)，原始通道与几何重建成果继续保留。

256/512 分辨率比较、实测 MTF/CNR、抑环参数和材料定量精度等待已知模体；本轮未将插值密度或颜色渲染当作这些指标的验证。

在现有程序中打开 task4 后，若先读到旧缓存，在 Reconstruction 选择 Change method，再选 Multicolor → Start reconstruction；新程序会校验并载入本次版本结果。安装包尚未更新。
