# D7100 手册依赖与交接冲突修正（0.9.14）

本次依据工作区 `tmp/d7100-check/HANDOFF-multiple-exposure.md`、Nikon D7100官方英文手册和CCP2官方帮助，只做离线实现与验证。未连接设备、拍摄、写入机身设置、开启Live View、HOME或出束；未安装应用。

## 实现

- 在现有34项摄影目录中补齐P/S/A/M模式限制、HDR与RAW/包围/闪光互斥，以及P/A慢速闪光同步条件。保留RAW/JPEG、WB子参数、Auto ISO上限、中央重点、物理MF及LV的既有依赖；不试发未广告属性。
- 完整Apply目标先验证。只对显式提交的HDR/包围OFF排序提前，写后鲜读描述符并逐项再校验；不自动补关闭参数。NEF只互斥WB包围，不能据此禁用所有包围或推断D0C0 OFF。
- 前端按实际健康证据禁用控件，草稿不解锁控件。增加整个草稿的互斥校验；离线预览使用同一呈现规则并明确SIMULATED，不代替生产引擎。
- 高级设置各分组增加简短依赖说明；HDR显示为机身的HDR strength，纠正荧光灯原生索引4/5的英文标签对调。数值代码沿用有来源的原生映射，没有用手册菜单顺序猜PTP编码。
- 多重曝光只在Scan Preparation提示机身菜单OFF及指示器检查；无启动弹窗、确认复选框、人工记录、机器OFF声明或新增准入门禁。USB连接与菜单变灰均不证明OFF。兼容Nikon返回的D7100/NIKON D7100型号标签。
- 修正能力矩阵和前端功能规格中的三处旧Multi UNKNOWN阻扫描表述，补充[手册功能关系与证据边界](../architecture/native-camera-capability-matrix.md#手册功能关系与机身检查)。profile 6、固定策略hash及旧profile只读兼容保持原政策。

## 验证

过程记录归 `.cache/tests/camera/20261008-manual-dependencies/`，采用实际生产函数、scripted transport及React界面，不访问设备。

| 状态 | 命令/检查 | 结果 |
| --- | --- | --- |
| PASS | `cargo test --manifest-path backend/Cargo.toml --workspace` | 引擎300、主程序2、桌面54；7项既有忽略测试 |
| PASS | `npm.cmd run test:frontend` | 176项；包含模式/未知状态、互斥草稿、显式OFF顺序和提示不改变readiness |
| PASS | `npm.cmd run test:desktop` | 27项；0.9.14版本、MSI账本及原UpgradeCode一致 |
| PASS | `npm.cmd run typecheck`、`npm.cmd run build` | 类型与前端产物通过 |
| PASS | 本机浏览器React QA | 1366×768、1920×1080、3840×2160；7个高级分类及预检提示，栏目满高、底部操作可见、无页面异常和中文文字 |

MSI构建、来源、最终身份和更新签名验证记录分别由发布工具保存在 `.cache/tests/release/0.9.14/`。`release/`仅交付最新Windows x64 MSI，Windows 10/11为兼容目标，保持原安装升级关系。

## 未验证边界

**仅离线验证。** 机身实际依赖切换、PTP取值与写入、真实串口/联锁，以及一次物理曝光与C101/C102句柄的对应关系仍须本次实机验收。提示是操作流程，不证明当前机身状态，也不把历史分项实测升级为当前版本完整扫描认证。
