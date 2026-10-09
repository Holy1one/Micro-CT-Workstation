# Windows 任务栏图标修正（0.9.9）

结论：PASS，仅离线验证。最终 MSI 为 release/micro-ct-workstation_0.9.9_windows_x86_64.msi（11,997,672 字节），SHA-256 为 658c8d4eebe5377880c914fea7cc02551c4ec8f5f84615fd029948f1f2e3407b。本次没有安装应用，没有连接设备；release/ 只保留此安装包。签名、源码构建身份和原生绘制证据位于本机 .cache/tests/release/。

## 发现与实现

用户任务栏截图中 Chrome、Edge 等图标轮廓完整，原应用图标较小且模糊。原 ICO 虽有多个尺寸，但第一帧为 16px；本机 Tauri 2.11.5 / tauri-codegen 2.6.3 的 new_ico 直接读取 entries()[0]。实际 0.9.7 窗口的 WM_GETICON 也只返回 16×16 小图标，未提供大图标，任务栏因此需要放大。

1. tools/build/prepare-windows-icons.py 从用户透明母版直接生成各尺寸，裁除多余透明边距，内容最长边占画布约 88%；以预乘透明度缩小，保持原轮廓和渐变。ICO 第一帧为 256px，随后是 192、128、96、80、64、48、40、32、24、20、16px，全部为 32 位透明 PNG。
2. tools/build/windows-icon.mjs 在每次暂存资源时校验首帧、全部 DPI 尺寸、RGBA 格式、文件边界和 256px 桌面 PNG 一致性。回归测试将有效 ICO 改成 16px 首帧，确认构建门禁会拒绝原故障。
3. backend/src-tauri/src/windows_icons.rs 使用 LoadImageW 从 EXE 的 32512 图标资源分别加载大/小图标，尺寸来自 GetSystemMetricsForDpi；DPI 变化后刷新。句柄在两张图标成功替换后释放，错误时保留旧图标；只销毁本模块拥有的句柄。不新增前端权限、设备命令或领域状态。
4. UI 浅色 Logo 独立为 app-logo-light-256.png，继续与深色 Logo 使用相同原始画布留白，保持既有主题切换。Windows 图标单独使用更紧凑的画布。两款 4096px 母版字节和透明通道保留；浅色母版 SHA-256 仍为 4cce89067d5d059042f9288e232463bb3996f4592e197776615bd6bd8a967bfa。

0.9.8 为本次检查过程的中间构建：256px 首帧已修正，但窗口仍只使用同一张 256px RGBA 图标，未按 Windows 大/小图标角色选择 DPI 帧。最终交付为 0.9.9；历史版本分配保持不变。

## 实际窗口验证

环境为 Windows 11 Pro，版本 10.0.26200；实际窗口 DPI 240，即 250%。使用 WiX dark 提取 MSI 中的应用与相邻引擎，在独立 E 盘缓存数据目录启动，读取实际 WM_GETICON / GetIconInfo，通过原生兼容 DC 与 DrawIconEx 绘制，不使用模拟图标替代应用结果。

| 构建 | 小图标 / SMALL2 | 大图标 | 正常退出 |
|---|---|---|---|
| 0.9.7 原版 | 16×16 | 缺失 | 应用 0、引擎 0 |
| 0.9.8 中间版 | 256×256 | 缺失 | 应用 0、引擎 0 |
| 0.9.9 最终版 | 40×40 | 80×80 | 应用 0、引擎 0 |

最终大/小图标在浅色、深色背景上均与对应 PNG 的透明合成结果一致，最大颜色通道误差为 1/255，超过 2/255 容差的像素为 0。该检查覆盖实际尺寸和透明边缘。早期 GDI+ GetHdc 离屏预览本身曾引入黑点，已改用原生兼容 DC 并重新生成三版证据；不把无效预览当作产品缺陷。

直观对比图：.cache/tests/release/20261008-taskbar-icon/taskbar-icon-before-after.png。顶部是用户原始任务栏截图，下方是实际应用图标句柄的 Windows 绘制结果与放大细节，不宣称是重新截取的任务栏。

## 门禁与交付身份

| 状态 | 命令 / 检查 | 结果 |
|---|---|---|
| PASS | npm.cmd run typecheck | TypeScript 无错误 |
| PASS | npm.cmd run test:frontend | 166 通过 |
| PASS | npm.cmd run test:desktop | 27 通过，包含 3 条图标回归 |
| PASS | npm.cmd run module-map:check | 208 个文件、16 个模块、39 条目录边界，图谱与源码一致 |
| PASS | cargo test --manifest-path backend/Cargo.toml --workspace | 353 通过，7 条原有硬件测试忽略 |
| PASS | npm.cmd run installer:build | 0.9.9 前端、桌面壳、同源引擎与 MSI 构建通过 |
| PASS | Python check-assets.py / compare-native.py | 12 个透明尺寸、母版哈希、主题 Alpha 及实际绘制比对通过 |
| PASS | check-native-icon.ps1 -Version 0.9.9 | 大/小尺寸匹配 DPI，应用与 owned engine 正常退出 |
| PASS | record-bundle-identities.mjs / sign-update-release.mjs | MSI 内应用/引擎与本次构建一致，发行签名与更新容器验证通过 |

MSI 内应用 SHA-256：2d6828f4015e456eaa10559633fced1042657f14abd6bc5688d78a298671132f；引擎 SHA-256：72dfd67843e142ba18c55ef9cf042fc775bfd1e3df13e79c0dce0fd8eca005a8。完整身份归 .cache/tests/release/0.9.9/bundle-identities.json。

Windows 10、不同缩放比例的实际任务栏、多屏 DPI 切换与安装后的固定快捷方式缓存未在本次实机验证。Windows 10 / 11 仍为兼容目标；本次未改变安装、修复、卸载或快捷方式配置。设备、曝光、转台固件与扫描精度的硬件验收边界保持原记录。

## 实现参考

- [Microsoft app icon construction](https://learn.microsoft.com/en-us/windows/apps/design/style/iconography/app-icon-construction)：提供原生尺寸、DPI 尺寸与透明资源。
- [Tauri 2.11.5 icon codegen](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.5/crates/tauri-codegen/src/image.rs)：第一帧解码实现。
- [Windows LoadImageW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-loadimagew)：从 EXE 资源加载指定尺寸的原生图标。
