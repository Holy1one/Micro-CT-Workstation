# 扫描项目加载进度与时间提示

## 结果

PASS，仅离线验证。原来的文件校验提示增加进度条、百分比、已核验文件数和剩余时间；重构方法与图像显示布局保持既有调用。截图来自实际 App、ReconstructionConsole 和样式的离线渲染，使用指定进度快照展示两个状态，不代表 task4 的实测加载时间。本轮未发布、安装或连接设备。

## 实现依据

- `scan_project.rs` 沿既有只读封存校验流程加载项目；`reconstruction.rs` 汇总前暗场、前空场、投影的 manifest 字节数，每读取 64 KiB 校验取消，按 100 ms 和文件完成边界发布进度。文件计数仅在大小/SHA-256 均符合后增加。
- `operations.rs` 将后台观察值同步到同一操作回执；后台最高 99%，唯一引擎成功提交才确认 100%，取消/失败清除 ETA，迟到更新不改变终态。无该字段的既有操作保持兼容。
- `engine/types.ts` 声明可选进度；生产 `tauriAdapter.ts` 校验数值、文件计数及终态后接受。Tauri 继续透传完整快照，不增加状态所有者。
- `ReconstructionConsole.tsx` 按引擎进度显示百分比和文件数。总量尚未知时显示准备条及 `Estimating remaining time…`；超过 10 秒向上取整到 10 秒（31 → `About 40s remaining`），10 秒以内显示 `Finishing soon…`。取消/失联显示等待原因，不继续展示完成预测。
- ETA 使用至少 250 ms 的实际读取样本估算，会随磁盘速度变化；最终清单复核和提交仍决定完成。前端没有计时器推进进度。

## 验证

证据目录：`.cache/tests/frontend/20261010-project-load-progress/`。

| 命令 / 检查 | 结果 |
| --- | --- |
| `cargo test --manifest-path backend/Cargo.toml --workspace -- --test-threads=1` | PASS：引擎 349，入口 2，桌面 56；既有忽略项合计 9 |
| `npm.cmd run typecheck` | PASS |
| `npm.cmd run build` | PASS；既有大 bundle 提示保留 |
| `npm.cmd run test:frontend` | PASS：191 |
| `npm.cmd run test:desktop` | PASS：27 |
| `npm.cmd run module-map:generate` / `module-map:check` | PASS：241 个文件、16 模块、39 个目录边界；module-map.log |
| headless Edge，1920×1080 / 1366×768，浅色/深色 | PASS：64%/40s 与 96%/即将完成；提示和控件均在面板内；零设备调用 |

针对本次变化的检查覆盖：字节进度单调、23 个 fixture 文件的实际大小/hash 和完成回执、同尺寸损坏不增加核验文件数、文件内取消、取消后迟到进度、成功提交与拒绝终态、未知 ETA、10 秒取整边界、失联/取消抑制预计完成、生产快照拒绝畸形进度。原始 fixture 内容未被读取校验改写；现有设备与扫描所有权回归全部通过。

视觉证据：`loading-1920.png` 为完整窗口，`loading-detail-1920.png` 为 64%/预计 40 秒，`finishing-detail-1920.png` 为 96%/即将完成；1366 及深色状态同目录保留。真实存储介质上的 ETA 偏差未单独测量。
