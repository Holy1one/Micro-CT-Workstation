# 叠层闪烁体重建与材料分辨成像

## 已确认条件与目录

task4 为 360 角度 D7100 NEF 扫描。沿射线入射方向，前层为绿光闪烁体玻璃 1 mm，后层为蓝光闪烁体玻璃 3 mm；成分未知。层信息是探测器信息，不能当作被扫描样品的材料标签。

| 内容 | 位置 |
|---|---|
| 领域计算 | `backend/ct-engine/src/reconstruction{,_channels,_spectral,_preprocess}.rs`、`material_decomposition.rs` |
| 前端输入和展示 | 现有 `frontend/src/`、`frontend/src/scene/`；不创建第二套工作区 |
| 原图和旧结果 | 原扫描目录，保留且只读 |
| 新结果 | `<scan>/reconstruction/spectral/v2-<input-hash>/` |
| 旧扫描补充层配置 | `<scan>/reconstruction/detector-config.json`，绑定原 manifest SHA-256 |
| 实测标定建议 | 图像根下 `CalibrationData/stacked-green-blue/<calibration-id>/`，有实测数据后建立 |
| 过程、日志、比较图 | `.cache/tests/ct-engine/<run-id>/`；验收摘要归 `docs/validation/` |

## 已实现流程

1. 预检准备弹窗勾选 **Stacked scintillator scan**，展开 5 行。完整行按入射顺序压缩成实际层列表；空行忽略，半填行阻止提交。材料、颜色、厚度随扫描几何冻结和恢复，旧无叠层字段的扫描继续可读。
2. 对 NEF 整数 CFA 分别提取 R、G1、G2、B。先补偿 CFA 采样位置，再平均两个绿色子通道。直接使用线性值、各通道暗场/空场方差；不使用相机 JPEG、白平衡、gamma 或显示值反推能谱。原生饱和在降采样前传播。
3. **Monochrome** 根据中央参考区域的实测 SNR × 可用比例自动选择波段，保存单通道灰度结果。此准则优化参考信号可靠性；未知样品下不能保证普适的材料 CNR 最优。
4. **Multicolor** 按所有已填写层的发光颜色选择去重的 R/G/B 观测；黄色选 R/G，未知发光谱保留 RGB。绿蓝 task4 保留 G/B。主结构通道仍自动选取；所有通道共享 CFA 坐标和经独立角度验证的轴校正。默认采用 FDK，旧调用仍可读原方法缓存。
5. 轴搜索逐步扩大至 16/32/48 探测器像素，四分之一像素步长。每一级使用固定公共射线；拟合和独立角度误差均需下降超过 10%，最优位置一致，边界解不通过。
6. 有合格实测标定时，先做实测跨通道配准，再在投影域反演两种基材料的等效路径，分别重建其体数据。标定文件可附通用材料库，在双基体积上联合估计候选类别与未知密度幅值，输出未知/歧义掩码。没有标定时允许离线模型假设与候选匹配实验，其结果不进入生产界面的已验证材料标签。
7. 科学体数据保留带符号 float32；预览为独立显示窗的 uint8。量化覆盖掩码、噪声截断和饱和诊断单独保存；不裁掉原显示体积。公共量化覆盖外多色预览回到灰度。Tauri 校验全部 RGB 预览哈希并再次检查引擎会话与结果登记。

## 可实现的材料分辨策略

采用组装系统的**实测双基材料查找表**，把玻璃吸收、后层受到前层过滤、光学串色及相机响应包含在实测映射中：

`[t_A, t_B] -> [p_green, p_blue] -> 受约束反演 -> 两路厚度投影 -> 两路体重建`。

**待测物体的材料、厚度都不要求用户提供。** 两种已知基材料用于标定系统，是表达未知衰减的坐标；分解出来的 Al/Cu 等效系数不表示样品实际由 Al/Cu 组成。用户指定 Method 的现行脚本采用 Al–Cu，本次离线比较固定使用该组合，并对材料库全部 66 项扫描未知厚度，未根据样品描述挑选基材料或删减候选。

完整推断链为：`系统响应标定 → 未知物体双基等效路径 → 双基体积 → 通用材料库签名/轨迹匹配 → 候选材料、未知厚度/密度幅值、歧义和未知区域`。未知厚度采用逐候选拟合，而不是先填写真实厚度；薄响应签名只适用于分解后体积，投影比较保留谱积分和束硬化轨迹。单材质射线和混合体素的假设须分开；混合射线先重建两路基体积，再对有充分覆盖的体素匹配，不能把整条混合射线强行标为单一材料。

Al–Cu 的非负组合不能保证覆盖所有低 Z 材料或吸收边材料；越域比例是诊断，不能强制截到某种材料后宣称识别成功。后续基材料选择依据通用库覆盖与实测条件数验证，可比较 PMMA/Al、PMMA/Cu 等组合，选择过程不依赖待测物体标签。对近似共线的签名输出歧义，不把厚度补偿的最小残差当作唯一识别证据。

当前量化实现每条射线求两个独立基系数。5 层配置全部参与波段选择、记录、缓存身份和标定匹配，RGB 最多提供 3 个颜色观测；重复发光颜色、串色或相近响应会进一步降低有效秩。增加层数不会自动产生 5 个可独立求解的能量通道。未知玻璃成分时不能依据厚度推算各层吸收、有效能量或五层独立信号。

实测响应不独立时，改用窄带滤光、多相机读出、独立层读出或改变吸收响应的玻璃组合，再建立标定；不能依靠颜色渲染解决测量不可辨识性。

## 标定采集与输入

真实拍摄和射线操作须另行授权。离线阶段可以准备下列采集矩阵：

- 同一组装顺序、厚度、镜头、距离、方向、59.9 kV、曝光、ISO 和相机原生配置；分别采暗场与空场。
- 通用基材料各自的阶梯厚度，以及交叉叠放的混合厚度；覆盖系统预期路径范围，无需预先知道待测物体。网格含零点，每轴 2–32 个厚度，响应来自未做显示转换的 G/B 投影。
- 在两颜色均可见的边缘标记上测跨通道配准。提交从公共绿坐标到蓝采样坐标的 affine（即逆向取样变换）及独立标记 RMS。变换基于本实现的 256²、已应用 mirror 的探测器坐标。
- 至少 3 个独立、互异且不在训练网格节点上的已知混合厚度，建议增加重复和不同厚度范围；不能把训练点复制成验证点。记录 NEF 来源、厚度测量、重复和误差。
- 候选门槛：配准 RMS ≤0.5 探测器像素、映射单调且 Jacobian 条件数 ≤100、独立厚度误差 ≤各基材料标定范围的 5%；至少 80% 合格候选射线在标定域内。这些是软件候选门槛，需要实际模体验证，不能视为已达到材料测量精度。

选择的 JSON 格式如下。`null` 是待测位置，模板不能被引擎当成有效标定：

```json
{
  "schemaVersion": 1,
  "measurement": "pending",
  "captureProfileHash": "原扫描 captureProfile.parametersHash",
  "voltageKv": 59.899520874,
  "layers": [
    {"materialName":"scintillator glass (composition unknown)","emissionColor":"green","thicknessMm":1},
    {"materialName":"scintillator glass (composition unknown)","emissionColor":"blue","thicknessMm":3}
  ],
  "basisNames": ["Al", "Cu"],
  "thicknessGridMm": [[0,1,2],[0,1,2]],
  "logAttenuation": null,
  "validationPoints": null,
  "blueToGreenAffine": null,
  "registrationRmsPixels": null,
  "geometryMm": [40,25,40],
  "mirrorX": false,
  "rotationDirection": "counterclockwise",
  "centerOffsetMm": [0,0],
  "detectorShape": [256,256],
  "sourceDescription": "待填写实测文件、厚度与验证来源"
}
```

`logAttenuation` 为长度 `nx*ny` 的 `[p_green,p_blue]` 列表，A 轴索引最快；零厚度响应为 `[0,0]`。`validationPoints` 每项为 `{"thicknessMm":[t_A,t_B],"logAttenuation":[p_G,p_B]}`。只有实测、来源完整且校验通过后才将 measurement 设为 `measured`。界面 Multicolor → Material calibration… 选择该文件；引擎匹配采集 profile、kV、全部层信息及几何，禁止合成标定为真实扫描贴标签。

### 标定文件中的通用候选库

可选字段 `materialLibrary` 支持未知材料匹配，不增加待测材料输入。库的 `basisNames` 必须与查找表一致；`candidates` 每项为 `{name,basisSignature:[a,b]}`，签名表示该材料在参考密度下的等效基体积分数。最多 200 个候选，重复名、负/零签名、错配基材料与非正定协方差均拒绝。

库同时需要 `covariance:[[c00,c01],[c10,c11]]`、`maximumSquaredError`、`ambiguitySquaredErrorGap`、`sourceDescription`。协方差和门槛来自独立基体积模体验证；不能把相机 RGB 噪声直接填成重建基系数协方差。引擎逐候选优化未知幅值 `s≥0`，最小化 `(x-s·signature)ᵀΣ⁻¹(x-s·signature)`。误差过大或负重建值输出未知；前两名差距不足输出歧义。通过者也保留为材料候选，仍需独立材料模体验证。

结果增量文件为 `material-candidates.u16le`、`material-match-error.f32le`、`material-density-scale.f32le`、`material-matching.json`，纳入原谱结果哈希封存。类别编码 0=未知/缺乏覆盖，65535=歧义，其他为库索引；匹配仅使用两路共同量化覆盖。前端暂保持基材料预览，不把候选图自动着色为已确认身份。

## 未知样品离线尝试

工具 `tools/verify/infer-unknown-materials.py` 和共用计算 `tools/lib/material_inference.py` 只读取封存的绿/蓝投影及 Method 资料，不连接设备、不改写源数据。它执行通用 Al–Cu 非负分解，将材料库轨迹映射到同一基坐标做匹配，同时以原始两通道谱积分的未知厚度拟合作独立诊断，避免只看基坐标投影误差。

```powershell
& 'E:\Application\anaconda3\envs\pytorch311\python.exe' tools/verify/infer-unknown-materials.py `
  --method 'E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\Method' `
  --cache 'E:\Main\Pictures\Camera Roll\task4\reconstruction\spectral\v2-04ec1329e2f55b770ca0' `
  --output '.cache/tests/ct-engine/20261009-task4-unknown-materials/new-run'
```

运行要求 NumPy、SciPy、Matplotlib 和本机 xrayatten 的 NIST 系数包；也可用 `--coeff-dir` 显式指定同格式元素表目录。既有输出非空时拒绝覆盖。输出包括候选/等效路径/基匹配 CSV、NPZ 射线坐标与误差、谱散点图、8 组响应假设敏感性以及输入和代码 SHA-256；均属于假设推断，不能代替 task4 实测标定。参见 [本次尝试与结果](../validation/task4-unknown-material-inference-2026-10-09.md)。

## 实验边界与后续验收

本轮使用 256³ 体积和 128³ 预览完成几何与通道基线。task4 仍有中心环状伪影，层深度、光学配准、散射、束硬化及圆轨迹 FDK 近似尚不能仅靠现有数据分别归因。增加到 512³ 并不证明实际分辨率提升；待已知模体补齐后，再比较 256/512 的 MTF、材料 ROI CNR、分解偏差和噪声，并决定抑环或正则化参数。禁止通过删除环状结构宣称保真提升。

玻璃组成不是实测映射的前置条件；如果要预测 X 射线能谱和逐层吸收，则需要**探测器玻璃**的成分、密度、吸收曲线，并最好提供发射谱、光产额和衰减数据。所需补充证据为系统源谱/组装响应及通用基材料模体标定；待测物体的组成与厚度继续保持未知。已知材料模体用于校准和独立验收，不能与待识别物体混淆。

## 依据

- [Modgil 等，双层双色探测器的材料识别和分解，2015](https://pmc.ncbi.nlm.nih.gov/articles/PMC4666511/)：系统响应与校准是材料识别依据。
- [Maier 等，微焦点双层双色探测器，2017](https://pubmed.ncbi.nlm.nih.gov/28381034/)：独立双色读出实验不能直接等同于本项目单相机 RGB 通道。
- [双色闪烁体与能量判别研究](https://doi.org/10.1126/sciadv.adh5081)：需要验证两层的能量响应差异，颜色差异本身不能证明可分解性。
- [NIST 质量衰减模型与混合物加和](https://physics.nist.gov/PhysRefData/XrayMassCoef/chap2.html)：按质量分数组合元素系数，未知面密度参与 Beer–Lambert 指数。多色响应先积分透射再取负对数。
- 用户提供的 `Method/Method-材料属性-v7.md`、`outputs/10_v7_test2_four_bin_open_interp/scripts/run_v7_test2_four_bin_open_interp.py`、`help_datas/common_materials_db.json`：未知厚度轨迹拟合、Al–Cu 基坐标及通用材料候选。文件内容和 SHA-256 收录于本次离线报告，不依赖已失效 README 中的 `00_data/` 路径。
