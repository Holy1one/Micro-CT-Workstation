# 整机可靠性R0覆盖审计与补测

日期：2026-10-07（Asia/Shanghai）。**R0审计/补测已完成，整体系统硬件可靠性仍未验收通过。** 本轮没有设备连接、拍摄、运动或射线动作；以下PASS全部是离线软件证据。

## 1. 对象与门禁

分支native-device-controls-offline，HEAD 11eed6cfea8f3f5118fcd07e70319f98fc398b51，保留已有dirty修改；开始时逐文件hash保存在`tmp/tests/ct-engine/20261007-225628-system-r0-coverage/source-before.json`。当前host 56EC7FABF9770334302CF270B5504891CFCB12C1199621D2950D797A6F03ECA8、内嵌engine 6C2E587F3522046681E632C89CD889C063B3C800DFCB14FBF6CF27A61EE412C7及正式入口C19B828D8F18D13D3EF2811B95B5094863AEE0E494611140001BC69716C91334均未重建/覆盖。

| 命令 | 结果 |
| --- | --- |
| `cargo test --workspace` | PASS：280 lib + 2 main + 52 workstation = **334 passed / 0 failed / 7 ignored**，ignored不计通过 |
| `node --experimental-strip-types --test tests/*.test.mjs` | PASS：**166/166** |
| `npm.cmd run typecheck` | PASS，exit0 |
| `npm.cmd run module-map:check` | PASS，1871 files / 16 modules / 25 boundaries |

完整workspace结果为workspace-2.log；定向与首次失败日志原样保留。初期失败是本轮新增夹具的PathBuf类型名、参考文件目录层级和警告ON/OFF编码错误，均已修复。本轮没有以失败重跑掩盖生产问题，也没有为通过测试降低断言。

## 2. 覆盖矩阵

每条测试已核对源码位置、关键输入/断言与本轮workspace的实际ok记录；名称库存仅用于导航，不单独算覆盖。PASS只涵盖“范围”列；PARTIAL明确列出缺少的整机/实机组合证据。

| ID | 场景 | 结论 | 具体测试 | 已证明范围与限制 |
| --- | --- | --- | --- | --- |
| R0-01 | 参数原子准备与无文件/锁副作用 | **PASS（离线）** | [atomic_preparation_uses_confirmed_native_shutter_and_has_no_task_lock_or_disk_capture](../crates/ct-engine/src/device_controls.rs) | 验证完整几何、确认快门来源和取消/拒绝边界；不证明设备动作 |
| R0-02 | 预检/Start鲜读与漂移拒绝 | **PASS（离线）** | [preflight_and_start_fresh_read_without_camera_writes_or_implicit_live_view_stop](../crates/ct-engine/src/device_controls.rs)<br>[rejected_start_drift_or_live_view_does_not_register_new_task_and_apply_remains_available](../crates/ct-engine/src/device_controls.rs) | 拒绝漂移/LV且不隐式写入，不制造任务锁 |
| R0-03 | 正常完整协调器及持续出束组 | **PASS（离线）** | [production_coordinator_two_rounds_preserve_continuous_groups_and_end_beam_before_final_file](../crates/ct-engine/src/scan.rs) | 真实生产协调器和三个worker跑两轮10+10参考/3投影；末张事件关束早于文件确认 |
| R0-04 | 会话/操作关联与重复曝光结束拒绝 | **PASS（离线）** | [exposure_boundary_requires_this_operations_event_and_rejects_old_or_duplicate_events](../crates/ct-engine/src/scan.rs)<br>[duplicate_capture_completion_never_emits_end_or_commits_a_file](../crates/ct-engine/src/devices/camera/tests.rs) | 关联事件验证，不用时间或文件出现推定ExposureEnded |
| R0-05 | 坏对象/截断NEF/多对象不提交 | **PASS（离线）** | [duplicate_or_unassociated_events_and_truncated_raw_never_commit](../crates/ct-engine/src/devices/camera/tests.rs)<br>[short_unknown_read_known_truncation_and_end_failure_are_distinct](../crates/ct-engine/src/devices/camera/wpd.rs) | Native capture/WPD有界传输断言，覆盖文件拒绝 |
| R0-06 | 出束投影中相机传输失联 | **PASS（离线）** | [r0_camera_loss_during_projection_isolates_camera_and_never_commits](../crates/ct-engine/src/scan.rs) | 新增：关束确认、Nano STOP、相机隔离/清理未知、0投影提交、20参考文件hash不变 |
| R0-07 | 投影传输中Stop与文件取消 | **PASS（离线）** | [r0_stop_during_projection_transfer_confirms_off_before_transfer_and_never_commits](../crates/ct-engine/src/scan.rs)<br>[queued_camera_transfer_cannot_block_snapshot_stop_or_moxtek_off](../crates/ct-engine/src/lib.rs) | 新增协调器场景+既有领域入口；脚本传输可协作取消，不等同真实COM卡死时序 |
| R0-08 | 投影中联锁触发 | **PASS（离线）** | [r0_interlock_during_projection_confirms_off_and_never_commits](../crates/ct-engine/src/scan.rs)<br>[owner_monitors_interlock_without_snapshot_or_scan_polling](../crates/ct-engine/src/devices/xray/moxtek.rs) | 新增：设备worker独立监测、协调器故障、确认OFF/STOP、拒绝提交 |
| R0-09 | 温度/通信异常的安全处置 | **PASS（离线）** | [active_emission_temperature_fault_forces_output_off](../crates/ct-engine/src/devices/xray/moxtek.rs)<br>[status_timeout_after_enable_forces_off_without_reenabling](../crates/ct-engine/src/devices/xray/moxtek.rs)<br>[observed_transport_failure_survives_a_concurrent_stop_and_successful_off](../crates/ct-engine/src/devices/xray/moxtek.rs) | 设备owner层有注入和状态断言；不外推全部协调器阶段组合 |
| R0-10 | OFF ACK成功但输出未关闭 | **PASS（离线）** | [r0_unconfirmed_off_retains_warning_and_cannot_report_safe_completion](../crates/ct-engine/src/scan.rs)<br>[failed_off_confirmation_remains_unknown_and_latches_fault](../crates/ct-engine/src/lib.rs) | 新增：假设备忽略OFF，独立读回拒绝、安全完成为fault、警告ON、0投影 |
| R0-11 | 暂停事务边界与继续 | **PASS（离线）** | [r0_pause_commits_current_projection_before_resume_without_repeating_files](../crates/ct-engine/src/scan.rs) | 新增：暂停前恰好1张完整提交，恢复最终3张、23次capture、3次ON块、2s间隔不绕过 |
| R0-12 | 投影清单写入失败 | **PASS（离线）** | [r0_manifest_write_failure_retains_nef_and_refuses_projection_progress](../crates/ct-engine/src/scan.rs) | 新增：匹配CAPTURE_DONE后CSV打开失败；保留NEF/参考hash，manifest不Completed且进度0 |
| R0-13 | Nano到位、ticket释放与Stop | **PASS（离线）** | [rearm_home_status_and_stop_are_verified](../crates/ct-engine/src/devices/turntable/nano.rs)<br>[emergency_stop_preempts_a_pending_move](../crates/ct-engine/src/devices/turntable/nano.rs)<br>[stop_invalidates_queued_home_rearm_and_motion_admission](../crates/ct-engine/src/devices/turntable/nano.rs) | 真实Nano worker对脚本行协议；没有物理方向/位置精度证据 |
| R0-14 | 位置显示不把目标或旧帧当实测 | **PASS（离线）** | [pending_move_keeps_old_angle_until_device_confirms_arrival](../crates/ct-engine/src/scan.rs)<br>[committing_an_older_frame_does_not_rewind_prefetched_live_position](../crates/ct-engine/src/scan.rs) | 确认前位置冻结、旧帧提交不回写新的确认位置 |
| R0-15 | 迟到结果、取消代次和锁 | **PASS（离线）** | [stop_invalidates_late_domain_completion_without_unlocking_or_beam_enable](../crates/ct-engine/src/lib.rs)<br>[cancelled_late_cleanup_failure_preserves_fault_and_abandon_lock_without_applying_stale_success](../crates/ct-engine/src/device_controls.rs) | 旧成功mutation不执行；清理失败证据/故障锁保留 |
| R0-16 | 跨重启任务锁、恢复与放弃 | **PASS（离线）** | [unfinished_lock_survives_restart_and_task_id_change](../crates/ct-engine/src/task_registry.rs)<br>[unfinished_task_restarts_connects_and_restores_only_through_confirmed_frozen_preflight](../crates/ct-engine/src/lib.rs)<br>[abandon_persists_unlock_keeps_files_and_prohibits_reuse](../crates/ct-engine/src/task_registry.rs) | 当前冻结profile/断点/锁的离线领域链，不证明跨HOME物理角度配准 |
| R0-17 | 损坏登记与持久写入取消/过时计划 | **PASS（离线）** | [corrupt_register_and_missing_or_corrupt_manifest_stay_locked](../crates/ct-engine/src/task_registry.rs)<br>[cancelled_and_stale_plans_keep_the_reserved_lock_and_never_overwrite_newer_state](../crates/ct-engine/src/task_registry.rs) | 不以缺失/损坏材料解除锁，不覆盖较新登记 |
| R0-18 | EOF/退出与未确认清理 | **PASS（离线）** | [normal_eof_cannot_hide_unconfirmed_shutdown](../crates/ct-engine/src/main.rs)<br>[sidecar_eof_cleanup_cannot_be_blocked_by_an_unread_stdout_pipe](../src-tauri/src/engine_client.rs)<br>[shutdown_deadline_does_not_promote_unfinished_cleanup_to_success](../crates/ct-engine/src/lib.rs) | 进程/领域关闭门禁；未确认不能被exit0隐藏，不等同真实设备退出认证 |
| R0-19 | 更新维护占用与清理证据 | **PASS（离线）** | [owned_devices_need_fresh_cleanup_and_ordinary_stop_cannot_clear_maintenance](../crates/ct-engine/src/software_update.rs)<br>[unknown_or_failed_cleanup_stays_blocked_and_cannot_be_cancelled_into_safety](../crates/ct-engine/src/software_update.rs)<br>[cancel_while_preparing_waits_for_real_cleanup_before_releasing_maintenance](../crates/ct-engine/src/software_update.rs) | 领域报告/session/op/锁/未知清理；当前已连接实机更新仍未测 |
| R0-20 | 连续上限、再开束与冷却策略 | **PARTIAL（离线）** | [owner_enforces_continuous_output_deadline_without_domain_polling](../crates/ct-engine/src/devices/xray/moxtek.rs)<br>[beam_enable_waits_two_seconds_after_confirmed_off](../crates/ct-engine/src/devices/xray/moxtek.rs)<br>[short_beam_budget_cools_before_starting_the_next_exposure](../crates/ct-engine/src/scan.rs)<br>[cooldown_countdown_uses_monotonic_elapsed_time](../crates/ct-engine/src/scan.rs) | owner硬截止/2s门禁/预算与单调倒计时已测；没有整机实机300s间隔与长时证据 |
| R0-21 | 真实WPD无响应/拔线等时序 | **PARTIAL（离线）** | [stalled_native_call_does_not_block_status_or_stop_acceptance](../crates/ct-engine/src/devices/camera/tests.rs)<br>[cancelling_stalled_transfer_prevents_late_file_and_old_session_reuse](../crates/ct-engine/src/devices/camera/tests.rs) | 组件硬阻塞/迟到模拟有覆盖；当前整机真实WPD异常时序与F-12具体根因仍缺证据 |

## 3. 六项新增协调器证据

新增`scan::tests::r0_*`运行真实生产扫描协调器、相机owner、Nano串口worker和Moxtek串口worker；仅物理传输替换为脚本。先完整完成前暗/前空场，再在第一投影的实际阶段注入故障或请求暂停。profile的测试专用多重曝光证明不进入生产构建。

- 相机失联：匹配已接受的RAM触发后，使GetEvent返回Unavailable；确认关束/Nano STOP、相机隔离且清理未知、零投影与无晚到NEF。
- 联锁：已进入投影出束后触发脚本联锁；独立owner监测及协调器故障处理均生效。
- 传输中Stop：两秒可协作取消传输中请求Stop，OFF写在该延迟上限前完成；没有晚到文件/投影提交。该毫秒断言不是实机时序认证。
- OFF未确认：脚本回ACK却保持输出，读回必须失败；fault、警告ON和零进度，不报告安全完成。
- 清单写入失败：匹配CAPTURE_DONE后，使acquisition.csv无法打开；保留完整NEF、20张参考hash，原manifest不Completed且进度不增加。
- 暂停/继续：暂停前提交恰好一张；继续完成三张，无重复文件/触发，保持原20参考hash和至少两秒再次ON门禁。

六个本轮通过run：

- [camera-loss](../tmp/tests/ct-engine/20261007-233719-system-coordinator-camera-loss-1791387439294127400/result.json)
- [interlock](../tmp/tests/ct-engine/20261007-233719-system-coordinator-interlock-1791387439365867300/result.json)
- [manifest-failure](../tmp/tests/ct-engine/20261007-233719-system-coordinator-manifest-failure-1791387439720761000/result.json)
- [off-unconfirmed](../tmp/tests/ct-engine/20261007-233720-system-coordinator-off-unconfirmed-1791387440317522000/result.json)
- [pause-resume](../tmp/tests/ct-engine/20261007-233720-system-coordinator-pause-resume-1791387440211511900/result.json)
- [stop-transfer](../tmp/tests/ct-engine/20261007-233720-system-coordinator-stop-transfer-1791387440304610400/result.json)

每个run单独保存result/命令、合成NEF和扫描日志，均标fullLinkedHardwareCertified=false；真实原始扫描数据未被使用或修改。

## 4. 修改边界

只扩展`#[cfg(test)]`：scan.rs测试模块、camera/tests.rs的脚本probe和moxtek.rs测试模块。生产部分与开工副本逐字一致（source-after.json），IPC/JSONL/TS/设备协议与安全准入没有变化；既有未提交修改完整保留。未改变UI，无新增截图需求。没有发布、安装、替换正式入口或重建候选EXE。

## 5. 下一步与剩余条件

1. R0已完成当前覆盖审计和六项缺失协调器补测。矩阵的实机/长时不足继续进入[整体验收计划](system-reliability-acceptance-plan-2026-10-07.md)的R2–R5，不将此文档升级为整机PASS。
2. 下一项R1：当前生产profile的多重曝光证据恒UNKNOWN，完整扫描仍BLOCKED；需要可靠机器证据策略，或使用者明确批准独立操作员证据策略。没有覆盖入口，本轮未放宽。
3. 真实HOME/MOVE、射线连接/参数/开束仍需本次动作授权和现场条件。
4. F-12保留“未复现、根因未确证”，其通用传输失联后果已有本轮离线协调器保护链证据；没有声称复现原WPD故障或解决其根因。
5. 当前候选的已连接设备更新维护、完整真实任务、长时稳定性和实际成像/重构精度仍待验。
