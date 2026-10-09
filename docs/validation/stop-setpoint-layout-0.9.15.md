# 暂停/停止、设定值反馈与固定栏布局（0.9.15）

本次完成用户截图中的四项调整。未连接设备、打开串口、写入射线/相机设置、运动或出束；未安装应用。

## 实现

- Camera 的色温右侧增加 **Long exposure NR**，固定扫描要求 Off。使用 Advanced 的相同草稿、原生权限与实际读回，基本栏和高级页同步，不自动改机身参数。
- SEND V/I 在真实后台记录请求、设备读回确认的接受值和 OFF 状态，以及 12 W 限制引起的另一通道调整。电压/电流行增加灰色 Applied，位于 Measured 前；其值只取引擎已确认设定，未确认、离线和陈旧显示未知，输入草稿不冒充设备确认。
- 转台栏保持原有高度与栏目比例。位置读数、连接/HOME 和运动控件压缩至30px，状态行单行展示，底部合并为唯一 Prepare scan / Preflight 或 Resume / end retained task 入口及进度条。
- Pause 保留扫描线程和持久任务锁，可 Resume。操作员 Stop 等待设备清理、扫描线程退出和终态登记原子写入后结束当前采集、释放占用并清空任务选择；全部照片、清单和日志保留，随后可以准备新任务。未完成任务标为 abandoned，已经封存的任务保持 completed；旧位置不能恢复或追加。
- 重复 Stop 不取消 complete/abandon 的终态登记写入。Stop 抢在新任务登记之前时只允许匹配原登记的冻结前身；登记被其他来源修改则拒绝覆盖，保留锁和故障。关闭应用、断开和自动故障清理仍保留中断任务的恢复资格。

## 验证

过程记录在 `.cache/tests/ct-engine/20261009-stop-feedback/`，生产函数使用 scripted transport，相机/射线/转台测试数据仅存在隔离测试目录。

| 状态 | 命令/检查 | 结果 |
| --- | --- | --- |
| PASS | `cargo test --manifest-path backend/Cargo.toml --workspace` | 引擎306、主程序2、桌面54；7项既有忽略测试 |
| PASS | `npm.cmd run test:frontend` | 177项，含预览 Pause 保留 / Stop 结束与新任务准备 |
| PASS | `npm.cmd run test:desktop` | 27项，0.9.15版本与历史不可变MSI账本 |
| PASS | `npm.cmd run typecheck`、`npm.cmd run build` | 类型与前端构建通过 |
| PASS | 本机 React 浏览器 QA | 1366×768、1920×1080、3840×2160，浅/深两主题，Ready/Running/Paused/Retained/Stopping/Stopped六种状态，固定高度、无重叠或栏内滚动，无页面异常和中文显示 |
| PASS | 设定值和草稿检查 | 修改 SET 不改变 Applied；新设备确认只更新 Applied；Measured 遥测独立；确认撤销显示未知；基本/Advanced 的 NR 草稿一致 |

后端回归覆盖真实扫描线程在 Running/Paused 时 Stop、持久任务锁与文件保留、开始登记未写入前取消、重复 Stop 和登记冲突。成功停止后的旧 taskId、计数、预检、HOME、冻结配置与恢复入口均失效；新任务仍执行正常准入。

MSI构建、版本、来源哈希、包内身份与更新签名由发布工具记录至 `.cache/tests/release/0.9.15/`。`release/`只交付最新 Windows x64 MSI，Windows 10/11为兼容目标，保持原安装升级关系。构建与包验证不执行安装。

## 未验证边界

**仅离线验证。** 真实设备 SEND 接受值与输出读回、实际采集中 Pause/Stop 清理时序及重新开始仍需实机验收。Windows 10/11完整安装、Repair 和系统版本矩阵未在本次执行，不把浏览器或scripted transport结果当作硬件认证。
