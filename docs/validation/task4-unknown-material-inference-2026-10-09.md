# task4 未知材料与未知厚度推断 · 2026-10-09

**仅离线验证。** 本次已实际运行通用双基分解和材料库匹配，但尚未得到可确认的材料身份。待测物体材料与厚度均不是输入要求。此前把待测材料当作标定前提的说明已更正，几何和双色重建成果保留。

## 输入、假设与信息隔离

- 输入为原 task4 的封存 v2 绿/蓝投影、各自质量标志与暗/空场统计；没有改写原 NEF、manifest 或既有重建文件。分析开始与结束均校验 v2 完整 artifacts 封存。
- 参考项目为用户指定的 `E:/Main/OneDrive/LanZhouUniv/Class/mluti-energy imaging/Method`。读取现有 v7 模型、Al–Cu 双基脚本、完整 66 项通用材料库、平滑源谱，以及本机 xrayatten 的 NIST v1.4 元素衰减数据。吸收边保留上下侧数据，禁止系数越域外推。
- 固定使用 Method 的 Al–Cu 组合，全部材料候选参与，未按对象用途删减材料或调整基材料。玻璃、聚合物、金属、合金、陶瓷和复合物均在库内。
- task4 探测器前绿 1 mm、后蓝 3 mm；Method 的三层实测 W 不直接作为 task4 的已标定响应。本次以其玻璃曲线为假设，重新计算两层能量沉积与前层过滤。其成分/密度不是 task4 的已确认值。
- 源谱按 59.8995 或 50 keV 截止，不推断这是 task4 的实测源谱。除主假设外，还固定比较密度倍率 0.7/1.3、open 归一化串色比例 0.15/0.30、附加铝过滤 0.5/1.0 mm，共 8 组。参数网格在查看材料排名前设定；比较结果不能当作这些参数的估计。
- 用户在执行前已透露对象为“鳄鱼夹”，因此这不是严格的人类盲测。代码、候选库筛选、拟合、图像结构先验和厚度边界均未使用这条描述；仅在结果完成后讨论。形状识别也没有参与材料推断。

## 实际计算

1. 从 360×256×256 射线中选两通道均为 measured 的区域：排除低增益、饱和、噪声截断；中心半径 100 像素、两通道衰减范围筛选，5×5 腐蚀避免边缘混合。
2. 共 4,070,605 条合格内部射线。固定每 8 个角度、每 4 个行/列采样，分析 31,784 条；按角度模 3 分为 21,149 和 10,635 条比较组。每条射线仍分别拟合未知路径，角度比较用于稳定性检查，不是材料真值验收。
3. 由实测暗/空场方差传播得到参考对数噪声尺度 G=0.0210614、B=0.0591391；当前对角近似未包含所有空间相关和散射误差。
4. 对每条射线求 Al/Cu 非负等效面密度，使用谱积分网格、双线性反演和边界最小残差；不用显示颜色反算能量。
5. 所有候选的连续未知厚度透射轨迹映射到同一 Al–Cu 坐标。只有数据与库轨迹都在基模型可解释域内才参与基坐标匹配。
6. 同时在原 G/B 对数域对每个候选优化未知面密度，比较两条推断路径。候选几何路径均允许 0–40 mm；厚度是拟合输出，按库参考密度换算，不能视为独立厚度测量。单材质射线假设不适用于任意叠加材料。

## 结果与判读

| 指标 | 结果 |
|---|---|
| 主响应假设下的 Al–Cu 可解释比例 | 831/31,784 = **2.61%**（残差不超过一个参考噪声单位） |
| 8 组假设的 Al–Cu 可解释比例 | **1.50%–31.71%** |
| 主假设薄响应 Jacobian 条件数 | 36.54；仅薄响应局部量，不代表全路径可逆性 |
| 参考原通道单材质候选最优拟合落在噪声范围内 | 75.63%；66 个候选中取最小误差，不能当分类准确率 |
| 主假设前两名原通道候选不能区分的射线比例 | **88.63%**；前两名平方噪声残差之差 <1 |
| 主假设 G/B 谱形比 B/G 的 P10/P50/P90 | 0.6545 / 0.7549 / 0.8789 |
| 原通道强衰减组跨假设统一 top-1 | **没有** |

以下列出强衰减组（G 对数衰减 >1）的原通道未知厚度候选，并展示真正的“先分解再匹配”结果。基坐标匹配只代表上述少量可解释射线，不能外推为整个物体：

| 响应假设 | 原通道未知厚度 top-3 | 分解后的基坐标 top-3 |
|---|---|---|
| reference60 | graphite、boron_nitride、Mo | soda_lime_glass、fiberglass_polyester、silica |
| reference50 | Mo、SnPb63_37_solder、graphite | borosilicate_glass、PDMS、fiberglass_polyester |
| 密度倍率 0.7 | Mo、graphite、boron_nitride | PDMS、fiberglass_polyester、G10_fiberglass_epoxy |
| 密度倍率 1.3 | graphite、boron_nitride、Ag | fiberglass_polyester、G10_fiberglass_epoxy、silicon_carbide |
| 串色 0.15 | zirconia、SnPb63_37_solder、PTFE | borosilicate_glass、silicon_carbide、Mg |
| 串色 0.30 | Zn、Cu、brass | Si、Cu、brass |
| 附加铝 0.5 mm | SnPb63_37_solder、Mo、PTFE | Mg、silica、G10_fiberglass_epoxy |
| 附加铝 1.0 mm | Mo、SnPb63_37_solder、alumina | silicon_carbide、fiberglass_polyester、silica |

**本次没有可靠判断为哪种材料。** 基坐标域覆盖不足、两条推断路径不一致、候选随系统假设改变，阻止发布唯一类别。不能因已知对象名称而挑出串色 0.30 那一行，不能把 graphite 或 Mo 的偶然排名当作对象真材质，也不能用“鳄鱼夹常见金属”补写答案。对象名称本身也没有提供其基体合金、涂层与其他部件的材料真值。

## 实现与后续可执行路径

- `tools/lib/material_inference.py`：NIST 吸收边插值、谱积分、双基非负反演、未知厚度轨迹和基坐标材料匹配。
- `tools/verify/infer-unknown-materials.py`：封存输入检查、固定射线抽样、噪声尺度、8 组假设、全库排名、等效路径、来源哈希与诊断图。没有待测材料或厚度参数。
- `backend/ct-engine/src/material_decomposition.rs`：标定可附通用 `materialLibrary`，签名按同一基坐标表示，逐候选联合估计未知密度幅值、拒绝坏拟合和近似共线候选。
- `reconstruction_spectral.rs`：在实测标定的双基体积上使用共同覆盖生成候选索引、误差、密度幅值和歧义/未知图；新增科学产物纳入原结果封存。候选身份不自动标为已验证，前端本轮未继续调整。

下一步是用通用基材料校准**组装探测器响应**，不提供待测物体标签。对 Al–Cu 之外的库域可比较 PMMA/Al、PMMA/Cu 的可解释性与条件数；选择依据通用库及独立模体。实测系统响应可吸收未知玻璃组成与串色，因此探测器组成不是实测标定的必需前提；若采用纯物理预测，则需核对这两片探测器玻璃的成分、密度和实际源谱。未知样品继续按库匹配、密度幅值拟合、歧义拒绝处理。

## 保存位置、复现与门禁

最终过程目录：`.cache/tests/ct-engine/20261009-task4-unknown-materials/`。

最终结果 `final/` 包含：

- `report.json`：全部输入/代码哈希、候选库、射线抽样、噪声、基分解诊断、两种匹配排名与敏感性。
- `*-rankings.csv`、`*-basis-match-rankings.csv`、`*-library-basis.csv`：全部候选与拟合路径/基坐标，未只保留 top-1。
- `sampled-ray-fits.npz`、`sampled-ray-hypotheses.csv`：射线位置、双基系数、逐候选误差、路径与歧义。
- `spectral-hypotheses.png`：原通道轨迹、等效双基散点和假设候选排名；图中明确标为未标定假设。
- `sensitivity.csv`：候选随响应假设变化的统计。

`run1/` 为初始节点网格诊断；`run2/` 增加网格间反演；`run3/` 接入基坐标库匹配。最终版本保留这些早期结果供核对，结论使用 `final/`。

| 状态 | 验证 | 结果 |
|---|---|---|
| PASS | `python -B tests/helpers/test_material_inference.py`（pytorch311） | 5 项：未知路径拟合、不可区分曲线、网格间混合反演/越域、吸收边不平均且不外推，以及分解后库匹配 |
| PASS | `cargo test --manifest-path backend/Cargo.toml --workspace -- --test-threads=1` | 345 引擎、2 入口、56 桌面壳；8+1 ignored，`workspace-tests.log` |
| PASS | `cargo test --manifest-path backend/Cargo.toml -p ct-engine reconstruction::material -- --test-threads=1` | 最后数值极限拒绝保护后的 4 项材料专项，`material-tests-final.log` |
| PASS | `npm.cmd run test:sites`、`npm.cmd run typecheck` | 构建通过，前端 188、桌面 27；已有大 bundle 提示。`sites-tests.log`、`typecheck.log` |
| PASS | `npm.cmd run module-map:check` | 240 个文件、16 模块、39 个目录边界；Python bytecode 移至生成物缓存，源码入口禁止在 tools 中生成 bytecode；`module-map.log` |
| PASS | 实际 task4 未知材料命令 | 66 项候选、8 组假设；前后输入封存验证，`inference-final.log` |
| PASS | `python -B .cache/tests/ct-engine/20261009-task4-unknown-materials/verify-integrity.py` | 380 个 NEF、manifest 未改写；5 个旧科学产物及 46 项实验来源/代码哈希通过，`integrity.json` |
| BLOCKED | task4 唯一材料身份与定量分解验收 | 探测器响应尚未校准；模型敏感性与歧义门槛未通过。已完成实际尝试，不要求待测材料真值作为推断输入 |

详细命令和标定契约见 [材料成像策略](../decisions/stacked-scintillator-material-imaging.md)。原 NEF 和旧重建成果均保留，未连接设备、拍摄、安装或发布。
