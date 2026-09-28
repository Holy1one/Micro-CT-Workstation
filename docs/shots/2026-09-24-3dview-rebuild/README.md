# 3Dview 日夜正交视觉验收

- 来源：`3Dview_rebuild` 分支，基线提交 `349e6dd Publish verified 0.7.0 portable executable`。**截图时本分支的视觉修改尚未提交**，因此这些图不是任何已发布提交的产物，仅代表当时工作树的状态。
- 环境：Windows 11、Chrome 无头模式本地离线预览、1920×1080 视口；未连接任何真实设备。
- 方法：`npm.cmd run dev -- --host 127.0.0.1 --port <port>` 起本地前端，再用 Node 脚本驱动无头 Chrome 经 DevTools Protocol 操作（`Page.navigate`、`Runtime.evaluate` 点击 ISO/FRONT/TOP 与 Light/Dark、`Page.captureScreenshot`）。
  - 早期一轮曾用 `agent-browser` 采集。该工具在本机新会话中出现过挂起，且批处理形态会剥掉 Windows 路径分隔符并把截图写到仓库根目录，因此**已停用**，不再作为采集手段。
  - 灰度图仅在浏览器会话内对 WebGL canvas 临时应用 `grayscale(1)`，截图后恢复。
- 选择理由：六张图分别显示灰度轮廓、白天三视角与夜间两视角；比保留整批调试截图更便于检查遮挡、裁切和灯光强度。
- 结果：所选截图中完整沙盒、设备与轨道均在画面内。像素测量（按元素色相统计）显示：三个预设下绿色装饰带距画布边缘 14–101 px、深色轨道环距边缘 21–23 px，**没有元素越过画布边界**；顶部操作坞展开时也不覆盖设备。
- 限制：
  - 这些是浏览器开发预览的视觉证据，**不证明**真实扫描动画、真实设备状态或低性能 GPU 的表现。
  - 静态回退图 `public/assets/3D-scene-*.png` 只表现静止中性姿态（拍摄时转台读数为 0.00°），不编码实时转台角度；`StaticSceneFallback.tsx` 在确认角度非零时改取 `-144` 文件，而两份内容相同，因此转动状态下静态图也不表现角度变化。
  - 触摸相关结论来自事件模拟，不是实体触屏验证。
  - 全部为离线验证，没有本次真实硬件证据。

## 被否决的反例

`new.png` 是用户回传的 TOP 灰模，**结构验收未通过**，保留在此仅作为被拒反例：

- 轮组相对车体过大，被画成外置的独立轮座；
- 用长斜连接杆连接车体与车轮，而实际机构是底盘两端的短承力连接臂；
- 轮胎没有清楚地压在承重跑道上。

被否决的原因与正确几何的对照见 `docs/decisions/3dview-rebuild.md` 的「灰度与上色验收」与「参考与下一轮方向」两节。任何生成图都不得作为几何、轮轨接触或设备状态证据。

## 图片清单

| 文件 | 内容 |
| --- | --- |
| `grey-iso-study.png` | 最终 WebGL 几何临时去色得到的灰度构图研究，用于明度与接触可读性检查；不是独立的材质灰模 |
| `iso-light-selected.png` | 白天 ISO |
| `front-light-selected.png` | 白天 FRONT |
| `iso-dark-selected.png` | 夜间 ISO |
| `top-dark-selected.png` | 夜间 TOP（TOP 视角目前只有夜间版本入库） |
| `new.png` | 被否决的 TOP 灰模反例，见上节 |

白天 TOP 视角尚未入库：早期生成的名叫 `top-light-selected.png` 的概念图从未进入本目录，其同名过程稿只存在于已忽略的 `tmp/screenshots/scene/20260924-imagegen-concepts/`。

## 本轮采集的完整截图集

本目录只保留经人工筛选的最小长期证据集。本轮为验收采集的完整 1920×1080 截图（ISO/FRONT/TOP × Light/Dark、减少动态、操作坞收起/展开）保存在被 Git 忽略的过程目录中，未纳入版本控制：

| 内容 | 位置 |
| --- | --- |
| 场景三视角 × 日夜、减少动态 | `tmp/screenshots/scene/20260926-040000-visual-review/` |
| 操作坞收起/展开、键盘焦点、日夜整页 | `tmp/screenshots/frontend/20260926-030000-console-feedback/` |
| 取景溢出测量与前后对比 | `tmp/tests/scene/20260926-070000-top-framing/` |

这些目录可再生成：按上面「方法」一节的步骤重跑即可。
