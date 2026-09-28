# Micro-CT Workstation 前端重设计：线性图标 × Apple 毛玻璃 × 动效规范

版本：v0.7.0 前端改版（仅前端，后端契约未动）
实现位置：`src/icons.tsx`（新增）、`src/App.tsx`、`src/menuActions.ts`、`src/tokens.css`、`src/styles.css`、`module-map/modules.json`
门禁：`npm run typecheck`、`npm run test:sites`（100 项）、`npm run module-map:check` 全部 PASS

---

## 1. 现有结构与功能模块分析

技术栈：React 19 + TypeScript + Vite 6 + Tauri 2（桌面壳），3D 视口为 react-three-fiber/drei，样式为双层 CSS 令牌体系（`tokens.css` 语义变量 + `styles.css` 组件布局），引擎快照经 `useEngine` 单向驱动 UI。

布局基线 1920×1080 固定画布（`design-canvas`，按窗口等比 zoom）：

```
┌──────────────────────────────────────────────────────────┐
│ MenuBar（39px）：品牌 / File·Edit·Tools·Help / 窗口控制    │
├──────────┬───────────────────────────────┬───────────────┤
│ 左列 420  │ 中央 LiveScene（3D 视口）       │ 右列 450      │
│ DevicePanel│  + ControlDock（灵动岛控制坞） │ XrayPanel     │
│ ScanParams │  + 状态浮窗 / 角度读数          │ （12W 控制器） │
├──────────┴───────────────────┬───────────┤ OperationPanel │
│ BottomConsole（306px，5 页签）│            │ （306px）      │
├──────────────────────────────┴───────────┴───────────────┤
│ StatusBar（30px）：状态点 / 任务 / 引擎徽章 / 主题段控      │
└──────────────────────────────────────────────────────────┘
```

改造前的问题：
- 图标语言混杂：Dock 用彩色插画 SVG（绿三角、黄双杠），窗口按钮用 CSS 绘制，帮助提示是裸文字 "?"，菜单/面板/页签无图标；
- 视觉偏"工业平板"：纯白实底卡片 + 硬边框，无层级透明度与纵深；
- 动效单薄：仅 140–180ms 线性过渡与 Dock 缩放，无弹性反馈、无进出动画。

## 2. 线性图标系统（src/icons.tsx）

**规格**：24×24 网格，1.6px 描边（pause 双杠 2.2px），圆头端点/圆角连接，`fill:none; stroke:currentColor`。颜色一律由 CSS `color` 继承——语义色、明暗主题、禁用置灰自动同步，不在图形里烤死颜色。辐射、准星、仪表的中心点为有意的小实心点（≤1.8px），作为视觉锚。

| 图标 | 名称 | 用途 |
|---|---|---|
| 📄+ | `task-new` | File › New Scan Task |
| 文件夹 | `folder` / `folder-open` | Save Path 选择钮 / File › Open Image Folder |
| 历史 | `history` | Open Last Result / Restore Previous / Dock·Restore |
| 导出 | `export` | File › Export Session Log |
| 撤销/重做 | `undo` / `redo` | Edit 菜单 |
| 重置 | `reset` | Edit › Reset Parameters |
| 滑杆 | `sliders` | Preferences / Scan Parameters 面板头 |
| 盾牌勾 | `shield-check` | Tools › Run Preflight |
| 准星 | `crosshair` | Home All Axes / Dock·Home |
| 脉搏 | `pulse` | Device Diagnostics |
| 书 | `book` | User Guide |
| 警示 | `warning` | Safety Notes / 错误 toast |
| 信息 | `info` | About |
| 问号圈 | `question` | HelpTip（替换裸 "?"） |
| 播放/暂停/停止 | `play` / `pause` / `stop` | Dock 运行控制 |
| 辐射三叶 | `radiation` | X 射线源（面板头、设备行、日志页签） |
| 旋转 | `rotate` | 转台（设备行、日志页签） |
| 相机 | `camera` | Camera（设备行、日志页签） |
| 链条 | `link` | Device Connection Status 面板头 |
| 仪表 | `gauge` | Operation Status 面板头 |
| 终端 | `terminal` | Log Aggregation 页签 |
| 图片 | `image` | Image Preview 页签 |
| 立方体 | `cube` | Equipment View 工具条 |
| 太阳/月亮 | `sun` / `moon` | 主题段控 |

**Dock 四键语义色**（线性图标 + currentColor）：Home=蓝 `--btnHome`，Start=绿 `--ok`，Restore=紫 `--violet`，Stop=红 `--danger`；随 `[data-state]` 切换：scanning→播放键变黄（Pause），paused→浅蓝（Resume）+ 淡蓝底，fault→播放键禁用。禁用键统一褪为 `--textMuted`，不再保留语义色。

**新增图标的方法**：在 `PATHS` 里按同一网格/描边规范加一条记录，`LineIconName` 联合类型加成员即可，组件零改动。

## 3. 视觉体系：精致工业仪器感（tokens.css / styles.css）

> v2 修订：初版「全面板毛玻璃」在实机上洗掉了控制台骨架、强调色与 3D 场景打架。
> 修订方向——**实心面板 + 清晰骨架，毛玻璃只留给浮层**，强调色回归与场景一致的深青。

配方（双主题各自一套令牌）：

1. **面板实心、骨架清晰**：面板用近白实底 `--panel`（dark `--panel #1D2733`）+ 可见发丝描边 `rgba(15,23,42,.13)`（dark `rgba(255,255,255,.11)`）+ 干脆的双层阴影（1px 接触影 + 6/16px 短弥散）。控制台读数区域必须"实"，不能飘。
2. **毛玻璃只用于浮层**：菜单栏、下拉菜单、灵动岛控制坞、状态浮窗、错误 toast、模态对话框保持 `backdrop-filter: blur(20–24px) saturate(150–180%)` 的半透明玻璃——浮起来的才玻璃，落下去的必须实。
3. **画布氛围**：`.viewport-shell` 保留极浅的多层渐变（浅色白光晕 / 深色海蓝微光），衬托实心面板的边缘，不做主角。
4. **唯一强调色 = 深青**：light `--accent #0E7490`、dark `#45B6CC`，与 3D 场景光路色 `--optics` 同族；`--accentSoft` 为 10%/16% 同色系浅底。ok/warn/danger 安全语义色不动。
5. **色彩身份**：
   - 面板头图标 = 22px 圆角芯片（`--accentSoft` 底 + 深青线性图标），每个面板一个可识别标记；
   - Dock 四键 = 10% 语义浅底键帽 + 语义色线性图标（Home 蓝 / Start 绿 / Restore 紫 / Stop 红），hover 加深到 18–20%，禁用键褪底褪色——恢复色彩个性且保持线性图标语言；
   - 扫描中播放键转黄底黄图标（Pause），暂停转浅蓝（Resume）。
6. **仪器读数**：数值井（功率/温度/计时/角度/SET 输入）统一 `font-variant-numeric: tabular-nums` 等宽数字。
7. **圆角与字体**：卡片 12–14px；字体优先 `-apple-system / SF Pro Text`，工程平台回退 Segoe UI Variable。

布局几何（1920×1080 基线、420/1fr/450 列、306px 底排、12px 节奏）全部保留——改造只动材质、不动骨架，`layout-geometry` 测试契约原样通过。

布局几何（1920×1080 基线、420/1fr/450 列、306px 底排、12px 节奏）全部保留——改造只动材质、不动骨架，`layout-geometry` 测试契约原样通过。

## 4. 动效系统总览

**动效令牌**（tokens.css `:root`）：

| 令牌 | 值 | 用途 |
|---|---|---|
| `--durFast` | 140ms | 悬停变色、按压 |
| `--durBase` | 200ms | 菜单/toast/日志进出 |
| `--durSlow` | 320ms | Dock 展开、对话框、主题切换 |
| `--easeOut` | cubic-bezier(0.22,1,0.36,1) | 先快后稳的进入 |
| `--easeSpring` | cubic-bezier(0.34,1.45,0.64,1) | 按压回弹、对话框 |
| `--easeSpringGentle` | cubic-bezier(0.3,1.3,0.5,1) | Dock 展开 |

**无障碍**：`prefers-reduced-motion: reduce` 时 tokens.css 将所有过渡/动画压至 0.01ms，styles.css 再对 Dock、菜单、对话框、日志、toast 等显式 `animation:none; transition:none`，Dock 改为常驻展开（既有逻辑），功能与读数完全一致。

## 5. 逐组件动效说明（可直接指导实现）

> 格式：触发条件 → 动画过程（时长/缓动/属性）→ 结束状态。实现均已随本次改版落地，下述行为即当前代码行为。

### 5.1 全局按钮（`button:not(.dock-key)`）
- **悬停**：边框色过渡到 `--accent`，140ms `--easeOut`；背景若有变化同速。
- **按压（:active）**：`transform: scale(0.96)` + 内阴影 `inset 0 1px 2px rgba(文字色,18%)`，140ms `--easeSpring`，松开后以同一缓动弹回 scale(1)，产生"按下去又回弹"的物理感。
- **禁用**：`opacity .62`，无变换。
- **焦点**：`outline: 2px solid var(--accent)`，offset 2px，即时出现无动画。

### 5.2 顶部菜单栏与下拉菜单（MenuBar / menu-dropdown）
- **菜单项悬停**：圆角 7px 底 `--neutralSoft` 淡入，140ms `--easeOut`；已打开项加粗。
- **下拉打开（menu-pop）**：200ms `--easeOut`，`opacity 0→1`、`translateY(-5px)→0`、`scale(0.97)→1`，`transform-origin: top left`（从菜单锚点长出）。
- **下拉项悬停**：背景 140ms 淡入；左侧线性图标颜色 140ms 由 `--textSecondary` 变为 `--accent`。
- **关闭**：组件直接卸载（无退场动画）；点击外部或 Esc 触发。

### 5.3 灵动岛控制坞（ControlDock）
- **接近涟漪（is-approaching）**：指针进入时，胶囊外两圈描边环执行 `island-ripple` 1.08s ease-out 无限循环（第二圈延迟 0.36s）：`opacity .48→0`、`scale(.94)→1.24`；指针离开 260ms 后停止。
- **展开（island-unfold）**：320ms `--easeSpringGentle`，`opacity .6→1`（前 62% 完成），横向 `scaleX(.52)→1.035(62%)→1(100%)`、纵向 `scaleY(.9)→1.02→1`——轻微过冲后回稳的弹性展开；同时四颗操作键从 `display:none` 变为栅格布局，胶囊文字/箭头隐藏。
- **折叠**：状态类切回 `is-collapsed`，键隐藏、恢复胶囊形态（无反向动画，瞬时）。
- **箭头**：胶囊 chevron 旋转 0→45°，跟随展开状态过渡。
- **操作键悬停**：背景变 `--islandKeyHover` + `translateY(-2px)` 上浮，140ms。
- **操作键按压**：`scale(0.9)`，140ms `--easeSpring`，松开回弹。
- **进度环**：`stroke-dashoffset` 150ms linear 跟随引擎提交的百分比；未知进度不画弧、显示 "–"。
- **Esc**：任意焦点位置折叠 Dock。

### 5.4 面板（panel）与主题切换
- **主题切换**：面板背景/边框/阴影、菜单栏、控制台、状态栏颜色统一 320ms `--easeOut` 交叉过渡；画布渐变即时替换（背景图不可过渡，由上层颜色的过渡掩盖跳变）。
- **面板无位移动画**：固定控制台布局，面板自身不做滑入，避免工业控制台的"不稳定感"。

### 5.5 底部控制台页签（console__tab）
- **切换**：选中页签背景 `--consoleTabActive` + 文字加粗 + 图标变 `--accent`，均 140ms `--easeOut`。
- **内容区换页**：日志列表整体重挂载，每行执行 `log-in`（见 5.6），形成"换页淡入"的页面切换感。
- **跟随开关（follow-toggle）**：背景/文字 140ms 过渡。

### 5.6 日志行（log-line）
- **新记录进入（log-in）**：200ms `--easeOut`，`opacity 0→1`、`translateY(4px)→0`；以 `backwards` 填充避免首帧闪烁。每条新日志挂载时各播一次；跟随模式下滚动即时贴底（无平滑滚动，保证控制台读数的确定性）。

### 5.7 模态对话框（modal-backdrop / modal-card）
- **背板**：`fade-in` 200ms `--easeOut`，`opacity 0→1`。
- **卡片（modal-in）**：320ms `--easeSpring`，`opacity 0→1`、`translateY(14px)→0`、`scale(0.94)→1`，弹性落位无过冲感过强。
- **关闭**：Esc/× 即时卸载（无退场动画）。

### 5.8 错误提示（error-toast）
- **出现（toast-in）**：320ms `--easeSpring`，`opacity 0→1`、`translateY(-10px)→0`、`scale(0.96)→1`，从顶栏下方滑出；毛玻璃底（dangerSoft 82% + blur 16px）+ 警示图标。
- **消失**：条件清除即卸载。

### 5.9 表单控件
- **输入框**：边框/阴影随 focus/校验态即时切换；校验失败 `is-invalid` 红边 + 1px 红色外环，无动画（控制台要求读数确定）。
- **复选框（check-row）**：勾选时对号 `check-pop` 140ms `--easeSpring`：`opacity 0→1`、`scale(.4)→1.25(70%)→1(100%)`；框体背景/边色 140ms 过渡；indeterminate 显示 "−" 无动画。
- **射线/定时开关（switch-btn）**：背景/文字 140ms 过渡；开启态附加红色弥散外阴影 `0 2px 10px rgba(dangerSolid,36%)`（200ms 淡入）；按压遵循全局 scale(0.96) 弹性。
- **视角段控（scene-view-btn）**：选中胶囊底 140ms 淡入 + 1px 内描边；按压 scale(0.96) 弹性。

### 5.10 预检与进度
- **预检条（preflight__fill）**：宽度 110ms linear 紧跟引擎提交的检查进度（刻意不用缓动，避免落后于真实状态）；通过=绿、警告=黄、失败=整条红，换色即时。
- **扫描进度条（op-progress__bar span）**：宽度 120ms linear，同上原则。
- **环形读数**：见 5.3。

### 5.11 图像预览（image-tile）
- **已采集瓦片悬停**：`translateY(-2px)` + 浮层阴影，140ms `--easeSpring` 弹起；未采集瓦片无响应。
- **新瓦片出现**：随控制台换页统一淡入。

### 5.12 场景辅助元素
- **射线呼吸（scene-ray）**：仅 `[data-state=scanning]` 时 2.4s ease-in-out 无限循环 `brightness 1→1.18→1`；状态切换时透明度 180ms ease-out 过渡到 `--rayReady/--rayScanning/--rayPaused/--rayFault`。
- **状态浮窗/角度读数**：毛玻璃浮窗静态呈现；数值变化即时（不预测设备位置，安全契约）。

### 5.13 窗口控制按钮
- **悬停**：底 `rgba(文字色,9%)` 140ms 淡入；关闭钮悬停变 `--dangerSolid` 红底白字。无按压变换（系统级按钮保持稳定）。

## 6. 验证记录（仅离线验证）

- `npm run typecheck`：PASS
- `npm run test:sites`（vite build + 100 项测试，含 layout-geometry / menu-wiring / console-feedback / fault-banner 契约）：PASS 100/100
- `npm run module-map:check`：PASS（`src/icons.tsx` 归属 ui-shell 模块）
- 浏览器预览截图核对：默认浅色 / Dock 展开 / 菜单展开 / 深色主题 / About 对话框，五态均符合预期
- 未验证边界：未在真实设备上验证（无硬件）；便携版 EXE 需以 `npm.cmd run portable:build` 重新构建后分发
