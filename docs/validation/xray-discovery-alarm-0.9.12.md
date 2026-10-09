# 0.9.12 首次射线连接失败误报警与日志刷屏

## 现场现象与证据

2026-10-08，用户报告 Moxtek 连接后转台/Nano 蜂鸣器响，日志每约 10 ms 重复 `Moxtek adapter is not connected`。用户明确说明射线源当时仅接电源、未接电脑 USB；声音来自 Nano，而非射线源本体。不能据此推定射线真实输出为 ON 或 OFF。

只读 Windows 设备枚举发现 CH340 COM7 正常、未找到 Moxtek VID 277B / PID 07D1 的可识别设备，另有一个 `USB\VID_0000&PID_0002` 未知设备、Code 43（设备描述符请求失败）；未知设备无法证明属于 Moxtek。未打开串口、连接设备或发送控制指令。

随后 21:36（Asia/Shanghai）的只读枚举已识别 `12 Watt Digital Tube Controller` 和 `12 Watt TC Serial Port (COM9)`，VID 277B / PID 07D1 / serial 183616（VCP 子接口 183616A），两者状态 OK、错误码 0；CH340 COM7 仍正常。证据保存在 `pnp-readonly.json`。这只证明 Windows 识别恢复，不证明串口通信、OFF 读回或设备动作已验收。

## 根因与修复

- 旧代码在 `enqueue_xray_connect` 入队前将 `ever_xray` 置 true。首次 USB 发现失败也被当成已拥有射线会话，随后进入全局 Stop；未使用的 Moxtek 代理无法确认 OFF，清理流程对 UNKNOWN 开启 Nano 警报。
- 领域观察器每个 tick 重复记录同一个 cached health 错误，填满 100 条日志并挤掉首次发现失败原因。串口 worker 实际独立按既定频率监测，10 ms 重复记录不是 10 ms 串口轮询。
- `ever_xray` 改为成功连接或实际端口证据后置 true。仅当本次是首次 discovery error、从未拥有射线会话、当前及缓存均无端口/连接/出束/故障证据、且无清理未知时，回执为 rejected，保留错误并退役未使用的代理，不触发全局 Stop 或 Nano 警报。
- 输出仍为 UNKNOWN、禁止开束、预检失效。任何既有会话、端口证据、通信/协议故障和关束未确认继续触发故障、STOP/OFF、警报和清理锁；普通 Stop 的 UNKNOWN 警报规则保留。
- 同一健康错误仅在错误文本、连接或关束/出束证据变化时记录；监测和故障门禁持续执行。

## 验证

仅离线验证，使用生产领域逻辑、真实 Moxtek/Nano worker 和脚本传输，不连接真实硬件。脚本、结果归 `.cache/tests/release/20261008-xray-discovery/`，最终包与身份归 `.cache/tests/release/0.9.12/`。

- PASS：修改前 `unopened_moxtek_discovery_failure_does_not_stop_or_alarm` 失败（failed，而非 rejected）；修改后通过，验证无 STOP/警报/HOME/MOVE、可继续手动操作、射线仍 UNKNOWN 且不可开束。
- PASS：修改前 `repeated_xray_health_errors_preserve_the_first_diagnostic` 失败（重复错误挤满 100 条日志）；修改后通过，500 次相同观察只记录一次、首次诊断保留，不同传输故障仍新增记录。
- PASS：`discovery_failure_cannot_erase_a_previously_owned_xray_session` 覆盖既有会话标记和缓存端口证据两种情况；继续触发 Stop、Nano 警报、UNKNOWN 与未确认清理锁。原 `source_unknown_warning_on_follows_delayed_nano_stop_confirmation` 保持通过，普通 Stop 的警报不被本次例外关闭。
- PASS：`cargo test --manifest-path backend/Cargo.toml --workspace`，301 + 2 + 54 = 357 passed，7 ignored，0 failed；`npm.cmd run test:desktop`，27 passed；`npm.cmd run module-map:check` 通过。
- PASS：`node tools/release/build-update-release.mjs` 编译前端、生产引擎、桌面壳及 MSI；实际 MSI 21 个对话框、200 个控件及权限门禁通过，保留 0.9.11 的允许提升权限/当前用户安装配置。
- PASS：WiX dark 提取最终 MSI，`record-bundle-identities.mjs` 核对实际应用及相邻引擎与当前源码构建一致；`sign-update-release.mjs` 完成 MSI/清单签名、实际包及内部更新容器签名验证，无 GitHub 发布。
- PASS：`check-candidate-app.ps1` 启动 MSI 提取的 0.9.12，使用隔离引擎与 WebView 数据目录，无设备连接。Control service Ready，无启动失败提示；应用 PID 46040、所属引擎 PID 60880 正常退出，均 exit 0。首次 QA 脚本对尚未公布路径的启动进程比较失败，脚本已改为等待路径可读并规范化；第一次候选实例也正常关闭，未修改最终 MSI。
- PASS：`release/` 仅保留最终 0.9.12 MSI；0.9.11 的固定包和安装验收证据留在版本缓存中。

真实 Nano 蜂鸣器、Moxtek 串口通信及 OFF 读回尚未验收，Windows USB 识别已恢复但未打开端口，当前安装仍为原目录 0.9.11。0.9.11 的安装器权限修复保留在本次源码中；本次不重新安装或修改 Windows 设备配置，也不替未知设备指定驱动。Windows 10 安装与真实设备能力仍待相应验收。

## 交付身份

- 文件：`release/micro-ct-workstation_0.9.12_windows_x86_64.msi`，17,019,319 bytes。
- MSI SHA-256：`47dcea5309a26bf4446c7cc97c7628ba525639715a553fa5ce8a17ea3a0976a9`。
- 构建来源 commit：`1e98cf3b10d88d839b7ea367fb45a3a31977451b`（含本机未提交改动）；sourceTreeSha256：`5acf58d92a97ff1cd5c51b72095f2f6069f39874061f3911630aa76ff7e7c833`。
- 引擎 SHA-256：`01b3b5f521c0989a8131ae0ab74023a99f2ffe948bd3cd2aed9be19783ab55c4`。
- 最终包固定身份并签名后未重新构建或替换；`build-info.json` 保持仅离线验证、hardwareCertified=false。

## Windows USB 依据

[Microsoft：Unknown USB Device 枚举失败排查](https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/case-study--troubleshooting-an-unknown-usb-device-by-using-etw-and-netmon)说明描述符请求失败是 USB 枚举失败的一种；软件日志修复不能代替 USB 识别恢复或射线 OFF 读回。
