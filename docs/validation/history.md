# 历史验收边界摘要

本页只保留仍影响当前判断的历史结论；不创建待办、不延续某次安装或设备操作授权。当前状态从 [验证索引](README.md) 和当前源码核对。

| 历史范围 | 保留的结论与证据 |
|---|---|
| 原生相机早期验收 | 指定候选取得独立 NEF、C101/C102、长度/hash、Live View 与部分参数证据，不能覆盖所有镜头、模式和持续工况。原 WPD 0x8007001F 后续出现成功拍摄，根因和重启因果未确证，不标根治。证据位于 .cache/tests/desktop/ 的对应构建目录。 |
| 0.9.10 → 0.9.11 安装 | 非系统卷 Repair 出现 Error 1926 / Error 5；清除 MSI SummaryInformation bit 8、保留 per-user 升级关系后，原目录升级和两次 Repair 通过。见 .cache/tests/release/20261008-installer-repair/；Windows 10 未在该轮验收。 |
| 0.9.12 首次射线发现 | 从未拥有射线会话的发现失败曾误触发全局 Stop 与刷屏；领域拒绝和日志去重已修正。该离线回归不证明已连接设备的通信故障已消失。见 .cache/tests/release/20261008-xray-discovery/。 |
| 0.9.13–0.9.20 参数、任务与显示 | 预检/profile、部分 Apply 拒绝、Stop 反馈、删除目录登记及关闭退出码修复分别有版本证据；历史测试数量不作为当前总数。当前行为以执行流程和错误码为准。 |
| task4 重构与桌面 | 原数据与旧缓存保留；颜色表示探测器响应，未标定不能当材料标签。定量识别限制保留在 [未知材料报告](task4-unknown-material-inference-2026-10-09.md)。 |
| 存卡迁移与 Nano 台架 | 已有明确授权的固件上传/读回及相机、转台分项记录；早期“RAM 无 NEF”或“全链未上机”是当时状态。记录在 .cache/tests/hardware/、.cache/tests/turntable/ 与 .cache/tests/camera/ 对应运行目录。物理角度精度仍不能从控制器 POS=0 推定。 |
| 早释放与吞吐 | 早期 L1 默认关闭和纯诊断环境开关已由默认开启、可关闭的生产调度替代；L2 多张在飞未实施。实测信号与严格串行基线分别保留在 [信号报告](camera-exposure-end-signal-2026-10-12.md)、[串行报告](camera-card-serial-multiexposure-2026-10-12.md)，不承诺 12 分钟完成 360 张。 |
| 卡容量与供电 | 0.9.43 曾把机身正数剩余张数作为硬上限，该判断已纠正。字节容量规划、0 张/未知拒绝及独立授权的旧全卡清理证据见 [容量复核](card-capacity-recheck-clear-2026-10-13.md)；不构成当前全卡删除授权。低电量和 WPD 阻塞不应被归因于 sleep，也未证明唯一物理根因。 |
| 0.9.36–0.9.41 联动 | 安装版曾完成三轮四投影扫描、导入与精确清卡及移动 Stop 分项检查；构建和工况边界见 [联动记录](installed-camera-validation-2026-10-13.md)。不能提升为当前版本全工况认证。 |
| 0.9.42 长错误布局 | 实际浏览器重现长诊断把工作区拉宽；修复后五种窗口与长无空格文本保持布局。原证据在 .cache/tests/ct-engine/20261013-card-capacity/。 |
| 0.9.45–0.9.50 通信与恢复 | 保留健康连接、SD 断点、监测恢复和暂停重试已落地。历史 task5 有明确授权删除的旧任务，与现在同名 task5 的 380 个文件不是同一次扫描；Moxtek 超时的物理原因未确证。证据在 .cache/tests/ct-engine/20261014-task5-fault-audit/、20261014-delivery-0950/ 及相应版本缓存。 |

本次退出 docs 的完整原文和路径清单在 .cache/tests/docs/20261015-docs-cleanup/before/ 与 before-manifest.json，逐文件 SHA-256 已核对；清理分类与原证据索引在 cleanup-summary.json。该本机快照包含未入 Git 的唯一报告，按验收证据保护，不当作可随意删除的构建缓存。已跟踪文档还可由 Git 历史追溯。当前维护文档不再保留复制用提示词、过期版本入口或重复流水。
