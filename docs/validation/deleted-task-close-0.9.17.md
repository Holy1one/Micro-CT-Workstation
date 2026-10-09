# 删除采集目录与窗口关闭修复（0.9.17）

本次修复成功预检后 Start 返回 SCAN_OUTPUT_RETAINED，以及引擎异常退出后窗口关闭被阻止的问题。仅离线验证；未安装应用、连接设备、打开串口或执行任何机械、相机、射线动作。

## 原因和现场只读核对

此前占用由 `%LOCALAPPDATA%/MicroCTWorkstation/active-task.json` 的 active/retained 记录决定，结束任务永久保留位置；删除采集目录没有撤销该记录。Preflight 检查设备和采集条件，Start 才校验输出目录，因此预检可以通过而启动失败。只读检查确认 task2 已标为 abandoned，整个目录已不存在；task3 仍为 unfinished，目录中仍有 frames、references、acquisition.csv 和 manifest.json。本次不修改这些用户登记或文件。

桌面关闭路径等待引擎退出时把非零退出码当成“仍不能关闭”，即使 owned child 已退出也继续阻止 Close。截图中的原始错误码同时没有被 UI 提取，导致显示 no code in this message 和通用设备建议。

## 实现

- 保存根目录可读且目标目录的 symlink_metadata 明确返回 NotFound，才算整个任务目录已删除。现存空目录、缺清单、不可读根目录、掉线盘符和失效链接继续保持占用。
- 后台文件系统探测不阻塞领域循环。无采集/重构线程、操作占用、更新维护或未确认设备清理时，才原子撤销已删除目录的登记；保留真实设备故障和清理锁。活动扫描不会因外部删除文件夹而解锁。
- 新 Start 立即检查已结束任务目录，确认删除即可与新登记一起提交，同一 taskId 创建新的采集，无需等待后台核对。新任务仍需全部设备准入、预检与必要 HOME。冲突在保留名额之前检查；竞态只撤销经磁盘核对确实未持久化的新名额。目录重新出现或登记被其他来源修改时拒绝写入。
- 普通关闭等待实际 owned engine 退出；异常退出也结束桌面进程，保留非零返回码和日志，不宣称设备清理成功。当前Tauri runtime将RequestExit(code)映射为ControlFlow::Exit而丢失退出码；桌面在允许退出的ExitRequested事件中记录该码，使用run_return待框架清理结束后明确传给进程退出。仍运行且超时则显示英文 Shutdown needs attention 对话框。更新安装继续要求正常 exit0 及本会话维护报告。
- UI 提取原始 SCAN_OUTPUT_RETAINED 等代码，显示 Scan folder needs attention、实际代码和目录处理建议；failed/rejected 回执均可关闭提示。

## 验证

命令和结果保存在 `.cache/tests/ct-engine/20261009-deleted-task-close/`；测试使用隔离目录和 scripted transport。最终发布证据保存在 `.cache/tests/release/0.9.17/`。0.9.16候选包正常关闭通过，但异常退出注入发现桌面返回码丢失，验收为FAIL，不交付；其固定签名和失败报告保留，新修正使用0.9.17，未覆盖签名版本。

| 状态 | 命令/检查 | 结果 |
| --- | --- | --- |
| PASS | `cargo test --manifest-path backend/Cargo.toml --workspace` | 引擎315、主程序2、桌面55；7项既有忽略测试 |
| PASS | `npm.cmd run test:frontend` | 178项，含原始目录错误码、提示分类及处理建议 |
| PASS | `npm.cmd run test:desktop` | 27项，版本与不可变MSI账本 |
| PASS | `npm.cmd run typecheck`、`npm.cmd run build` | 类型和前端构建通过 |
| PASS | 本机 React 浏览器 QA | 浅/深主题，failed/rejected两类回执，真实页面错误码和目录建议、Dismiss后关闭、后续回执重新显示，无页面异常或中文文案 |

回归覆盖删除旧目录后立即复用同一 ID、删除 active 目录跨重启解锁、现存缺清单目录/不可读根目录保持锁、目录重新出现和旧登记竞态拒绝、未持久化保留名额回滚、Pause期间不撤锁，以及 owned child exit9允许普通关闭但拒绝更新。生产 Start 回归通过 scripted Nano/camera/Moxtek 执行，不使用真实设备。

最终交付还检查包内程序身份、签名与启动/关闭。正常退出和仅针对隔离候选引擎的异常退出注入结果由本机验收报告记录；安装器不在这些检查中执行安装。`release/`只保留最新Windows x64 MSI。

## 未验证边界

**仅离线验证。** 尚未在真实设备扫描中复现并验收删除后的新建采集。Windows 10/11完整安装、Repair及系统版本矩阵本次未执行。非零引擎退出只证明进程已结束，不证明射线OFF或硬件安全；不能用于授权更新。
