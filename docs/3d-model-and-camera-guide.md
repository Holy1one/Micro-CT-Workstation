# Micro-CT 三维模型与相机视角维护指南

本文面向后续维护三维设备场景的开发者，说明当前程序化几何、光轴坐标、相机预设，以及将现成 GLB/GLTF 模型接入 React Three Fiber 场景时应遵循的约束。

> 当前装配：`RingTrack` → `MountedEquipment` → `RailCarriage`（含共享的固定高度 `ScissorLift`）→ 原设备模型。射线源保持 `SOURCE.bodySize = [144, 126, 184]` 与模型内高度除以 2 的计算（机壳实际高 63）；相机和 `AirPod` 几何保持原版。两侧各有黑色上下台板、成对 X 形剪叉、银色铰接销、穿过螺母横梁的水平丝杆及与丝杆同轴的侧手轮。射线源台板从 y=-42.8 到机壳底面 y=-1.5；相机底面恰在 y=-42.8，因此相机侧剪叉置于 y=-68 至 -42.8。相机侧下台板占 y=-76 至 -68，承重轮占 y=-76 至 -56；相机承重轮沿切向移至台板两端以外，并由横向支承臂接回台板，避免轮胎穿板。台板切向×径向尺寸分别为射线源 144×112、相机 196×112，板厚 8；机构仅作固定高度展示，不参与运动或控制。尺寸均为展示坐标，不是制造依据。轮轨接触继续由 `trackContactPose` 求解，转台和耳机仍只跟随 `FeedbackAngle` 的确认反馈。

## 1. 当前三维场景入口

三维渲染链路如下：

1. `src/App.tsx` 根据 `useSceneFallback()` 的结果，在实时 Canvas 和静态降级图之间选择。
2. `src/scene/LiveSceneCanvas.tsx` 创建 React Three Fiber `Canvas`、正交相机和轨道控制器，并按投影包围盒自动取景（取景数学在 `src/scene/scene-fit.ts`）。
3. `src/scene/EquipmentScene.tsx` 是设备几何的主入口，负责光源、转台与样品、相机、光束和光轴线；`src/scene/stage-surroundings.tsx` 负责上半球球壁与桌面圆盘（当前半径 4000，桌面高度取 `RING_TRACK.bottomY`，即 y=-112）。
4. `src/scene/scene-config.ts` 集中保存光轴坐标、设备尺寸、转台参数、相机约束和视角预设。
5. `src/scene/StaticSceneFallback.tsx` 在 WebGL 不可用或 WebGL 上下文丢失时展示静态图片。

`EquipmentScene` 当前组合的程序化组件为：

- `SourceAssembly`：X 射线源机体、出射端和散热片；
- `TurntableAndSample`：转台底座、旋转盘、角度标记和样品；
- 闪烁体及背板：由场景内的几何直接组合，中心由 `SCINTILLATOR` 配置；
- `CameraAssembly`：相机机身、镜头和尾部面板；
- `Beam`：从焦点到闪烁体的半透明锥形光束；
- `Line`：依次连接焦点、样品、闪烁体和镜头中心的光轴辅助线。

若只替换某一设备模型，应优先在对应 Assembly 内替换几何，而不是绕过 `EquipmentScene` 新建第二套场景入口。

## 2. 坐标系与毫米单位约定

### 2.1 当前坐标方向

当前代码可按以下方向理解：

- **X 轴**：设备横向；当前主光轴上的关键点均为 `x = 0`。
- **Y 轴**：高度方向；主光轴高度为 `OPTICAL_AXIS_Y`。
- **Z 轴**：射线传播方向；从负 Z 的射线源，经过 Z=0 的样品，指向正 Z 的闪烁体和镜头。

建议把 Three.js 的 **1 个场景单位统一解释为 1 mm**。当前尺寸和距离（例如源焦点 `-320`、闪烁体 `205`、镜头中心 `292`）也符合毫米量级的设备场景表达。外部模型导出前应优先在 DCC/CAD 工具中统一成毫米；运行时只做一次确定性的整体换算。

### 2.2 光轴常量

`src/scene/scene-config.ts` 当前定义：

| 常量 | 当前值 | 含义 |
|---|---:|---|
| `SOURCE_Z` | `-320` | X 射线源焦点的 Z 坐标 |
| `SAMPLE_Z` | `0` | 样品中心的 Z 坐标 |
| `SCINTILLATOR_Z` | `205` | 闪烁体中心的 Z 坐标 |
| `LENS_Z` | `292` | 镜头中心应使用的 Z 坐标 |
| `OPTICAL_AXIS_Y` | `30` | 焦点、样品、闪烁体和镜头中心共享的 Y 高度 |

关键中心点为：

```ts
SOURCE.focus       // [0, OPTICAL_AXIS_Y, SOURCE_Z]
SAMPLE.center      // [0, OPTICAL_AXIS_Y, SAMPLE_Z]
SCINTILLATOR.center // [0, OPTICAL_AXIS_Y, SCINTILLATOR_Z]
CAMERA.lensCenter  // [0, OPTICAL_AXIS_Y, 292]
```

当前 `CAMERA.lensCenter` 的第三项写为字面量 `292`，而 `LENS_Z` 参与 Z 顺序断言。维护时应保证二者始终一致；若后续修改镜头位置，建议同时把 `CAMERA.lensCenter` 改为引用 `LENS_Z`，避免常量与实际中心点分离。

模块加载时，`assertOpticalAxisConfiguration()` 会验证：

- 四个关键点都位于 `x = 0`；
- 四个关键点的 Y 坐标都等于 `OPTICAL_AXIS_Y`；
- Z 顺序满足 `SOURCE_Z < SAMPLE_Z < SCINTILLATOR_Z < LENS_Z`。

因此，不应为了“让单个模型看起来合适”而单独移动其光学中心或任意缩放各组件。源焦点、样品中心、闪烁体中心和镜头中心的共线关系是设备几何的核心约束。

## 3. 修改 position、rotation 和 scale 时分别会改变什么

React Three Fiber 中的变换与 Three.js 一致：

- `position={[x, y, z]}`：移动对象原点；X 为横向，Y 为高度，Z 为光轴前后位置。
- `rotation={[rx, ry, rz]}`：按 X/Y/Z 欧拉角旋转，单位为**弧度**。例如 `Math.PI / 2` 为 90°。
- `scale={[sx, sy, sz]}`：按模型局部 X/Y/Z 方向缩放；`scale={k}` 或 `[k, k, k]` 为等比缩放。

维护建议：

1. **先修正模型自身轴向和原点，再设置场景 position。** 不要用多层临时旋转和偏移叠加来掩盖导出问题。
2. **整台设备使用统一的毫米换算比例。** 不要分别把射线源、样品、闪烁体和镜头“缩放到看起来差不多”，否则真实距离、光束锥角和共线关系会失真。
3. 若外部模型必须缩放，应在导入模型的最外层组做一次等比缩放，并让模型内部用于对齐的光学基准点与 `scene-config.ts` 中的中心点对应。
4. 转台角度是绕 Y 轴旋转。`TurntableAndSample` 用 `FeedbackAngle` 接收有效的确认反馈，再把插值后的角度写入旋转组的 `rotation.y`；需要随转台转动的样品模型必须放进该组，固定底座与其他光学组件留在组外。

## 4. 相机视角和 OrbitControls

### 4.1 OrthographicCamera 与自动取景

`LiveSceneCanvas.tsx` 使用 `OrthographicCamera`（`near = -9000`、`far = 9000`），垂直视锥高度为常量 `FRUSTUM_HEIGHT = 1200`（即 `top = FRUSTUM_HEIGHT / 2`、`bottom = -FRUSTUM_HEIGHT / 2`，左右再按视口宽高比推出半宽）：

```tsx
<OrthographicCamera
  makeDefault
  near={-9000}
  far={9000}
  top={FRUSTUM_HEIGHT / 2}
  bottom={-FRUSTUM_HEIGHT / 2}
  left={-FRUSTUM_HEIGHT / 2}
  right={FRUSTUM_HEIGHT / 2}
  position={CAMERA_PRESETS.iso}
/>
```

正交相机按 `zoom` 取景，因此预设只决定视线方向，实际取景由投影包围盒算出。切换预设时，`CameraRig` 会：

1. 从 `CAMERA_PRESETS[preset]` 读取相机位置，与 `CAMERA_CONSTRAINTS.target` 相减得到视线方向；
2. 用 `Box3` 量出需要保持在画面内的网格包围盒，把 8 个角点变换到相机空间，取投影后的水平与垂直跨度；
3. 取 `min(FRUSTUM_HEIGHT × aspect × FIT_WIDTH / 水平跨度, FRUSTUM_HEIGHT × FIT_HEIGHT / 垂直跨度)` 作为 `orthographic.zoom`，其中 `FIT_WIDTH = 0.88`、`FIT_HEIGHT = 0.82`，即预留 12% / 18% 边距；
4. 把相机与 orbit target 沿屏幕上方平移 `FRUSTUM_HEIGHT × VIEW_CENTER_SHIFT / zoom`（`VIEW_CENTER_SHIFT = 0.05`），让设备整体落在顶部视角按钮坞下方；
5. 把 `OrbitControls` 的 `minZoom` / `maxZoom` 设为 `zoom × CAMERA_CONSTRAINTS.minZoom`（0.6）与 `zoom × CAMERA_CONSTRAINTS.maxZoom`（2.2），再把 target 重设为 `CAMERA_CONSTRAINTS.target`；
6. 调用 `invalidate()` 触发按需渲染。

取景包围盒由 `src/scene/scene-fit.ts` 的 `collectSceneBounds` 收集。它遍历场景图，跳过不可见对象；祖先带 `userData.excludeFromFit` 的网格默认排除（背景半球与桌面圆盘就属于这一类，它们本应铺满画面而不参与取景），但若该网格自身标记 `userData.includeInFit`，仍会重新纳入取景，避免可见的装饰带被漏掉而出现欠覆盖。

计算被抽到 `src/scene/scene-fit.ts` 以便单元测试。`tests/scene-framing.test.mjs` 用合成轨道与装饰带包围盒、按三个预设的相机朝向断言投影跨度不越界且不超过 `FIT_WIDTH` / `FIT_HEIGHT` 留边。

当前预设（位置只决定视线方向）：

| 预设 | 相机位置（视线方向） |
|---|---|
| `iso` | `[760, 430, 760]` |
| `front` | `[780, 95, 0]` |
| `top` | `[0, 1050, 0.001]` |

`top` 的 Z 使用 `0.001` 而不是完全为 0，可避免相机朝向计算处于退化方向。`OrbitControls` 的 `minZoom` / `maxZoom` 属性上写的是 0.1 与 5，但 `CameraRig` 在每次取景后会把它覆盖为上面第 5 步的以 fit 为基准的范围。新增预设时，应同时更新 `ViewPreset` 类型、`CAMERA_PRESETS` 和对应 UI 入口。用户手动轨道后，`CameraRig` 通过 `userOrbited` 停止在 resize 时重新取景，直到再次点击某个预设才会复位。

### 4.2 OrbitControls

当前 `OrbitControls` 配置为：

- 启用阻尼，`dampingFactor = 0.08`；`enableDamping` 绑定 `!reducedMotion`，因此 `prefers-reduced-motion: reduce` 生效时会关闭阻尼；
- 禁止平移，避免用户把设备移出视野；
- 垂直旋转角限制为 12° 到 78°（`CAMERA_CONSTRAINTS.minPolarAngle` / `maxPolarAngle`）；
- 轨道缩放范围在每次取景后被改写为「本次 fit 的 zoom × 0.6」到「× 2.2」（`CAMERA_CONSTRAINTS.minZoom` / `maxZoom`）；
- 观察目标为 `[0, 20, 0]`。

`LiveSceneCanvas.tsx` 用 `usePrefersReducedMotion()` 监听 `window.matchMedia("(prefers-reduced-motion: reduce)")`，并在用户运行期改变该偏好时同步更新；同一个标志既驱动上面的 `enableDamping`，也传给 `EquipmentScene`，让转台角度直接跳到确认值而不做插值。为了可观测，`SceneCanvasSettings` 会把 `data-prefers-reduced-motion` 与 `data-orbit-damping` 写到 WebGL canvas 的 `dataset` 上。这些只影响展示，不改变任何实测读数。

如果模型整体尺寸发生改变，优先统一调整 `CAMERA_PRESETS`、`CAMERA_CONSTRAINTS.target` 或 `FIT_WIDTH` / `FIT_HEIGHT`，不要通过破坏设备各组件的相对比例来迁就视野。

## 5. 外部 GLB/GLTF 模型的目录建议

建议将现成模型放在：

```text
public/assets/models/micro-ct/
```

例如：

```text
public/assets/models/micro-ct/source.glb
public/assets/models/micro-ct/turntable.glb
public/assets/models/micro-ct/sample.glb
public/assets/models/micro-ct/scintillator.glb
public/assets/models/micro-ct/camera.glb
public/assets/models/micro-ct/textures/
```

放在 `public` 下的文件会以站点根路径提供，且会随 Vite 前端产物进入打包目录。路径命名建议只使用小写 ASCII、数字、短横线和正斜杠，避免桌面打包后出现大小写或 URL 编码差异。

如果模型较多，优先使用 GLB，将网格、材质和贴图打包到单个二进制文件；若使用分离式 `.gltf + .bin + textures`，必须保持相对路径结构完整。

## 6. useGLTF 导入示例

下面示例演示如何替换射线源的程序化外观，同时把场景定位和模型内部校正分开。示例仅用于接入方式说明，当前源码尚未采用它。

```tsx
import { useGLTF } from "@react-three/drei";
import { useMemo } from "react";
import * as THREE from "three";
import { SOURCE } from "./scene-config";

const SOURCE_MODEL_URL = "/assets/models/micro-ct/source.glb";

function SourceModel() {
  const { scene } = useGLTF(SOURCE_MODEL_URL);

  const model = useMemo(() => {
    const clonedScene = scene.clone(true);
    clonedScene.traverse((object) => {
      if (object instanceof THREE.Mesh) {
        object.castShadow = true;
        object.receiveShadow = true;
      }
    });
    return clonedScene;
  }, [scene]);

  return (
    <group position={SOURCE.bodyCenter}>
      <primitive
        object={model}
        rotation={[0, Math.PI, 0]}
        scale={[1, 1, 1]}
      />
    </group>
  );
}

useGLTF.preload(SOURCE_MODEL_URL);
```

注意：

- `useGLTF` 会挂起组件渲染。接入时应在 Canvas 内为模型区域提供 `Suspense` 边界；不能让一次模型加载失败破坏整个应用页面。
- 同一个 `scene` 对象不应同时挂载到多个位置；需要多实例时应克隆，带骨骼动画的模型应使用 Drei/Three.js 适合骨骼的克隆方式。
- 示例中的 `rotation` 只是演示。实际值必须由模型导出轴向决定，不能照抄。

## 7. 使用 Box3 做一次性尺寸归一化

外部模型常以米、厘米或任意 CAD 单位导出。可用 `THREE.Box3` 计算包围盒，再把模型按**统一目标尺寸**等比归一化。建议归一化逻辑在模型载入后只执行一次，并把得到的比例记录为模型资产元数据或常量。

```ts
import * as THREE from "three";

export function normalizeModelToMillimeters(
  object: THREE.Object3D,
  targetLongestSideMm: number,
): number {
  if (!Number.isFinite(targetLongestSideMm) || targetLongestSideMm <= 0) {
    throw new Error("targetLongestSideMm must be a positive finite number");
  }

  object.updateWorldMatrix(true, true);
  const bounds = new THREE.Box3().setFromObject(object);
  if (bounds.isEmpty()) {
    throw new Error("Cannot normalize a model with an empty bounding box");
  }

  const size = bounds.getSize(new THREE.Vector3());
  const longestSide = Math.max(size.x, size.y, size.z);
  if (!Number.isFinite(longestSide) || longestSide <= 0) {
    throw new Error("Cannot normalize a model with zero or invalid size");
  }

  const uniformScale = targetLongestSideMm / longestSide;
  object.scale.multiplyScalar(uniformScale);
  object.updateWorldMatrix(true, true);
  return uniformScale;
}
```

Box3 只解决“整体尺寸”问题，不会自动知道：

- 哪个方向是模型前方；
- 哪个点是射线焦点、样品中心或镜头光心；
- 模型原点是否位于机械安装基准；
- 模型应当绕哪个轴旋转。

因此，不能只依靠“居中包围盒”完成光学装配。最可靠的流程是在建模软件中放置明确的空节点/基准点（例如 `OpticalCenter`、`RotationAxis`），导出后按节点对齐。

## 8. 轴向、原点和转台父组

### 8.1 推荐资产约定

建议所有模型遵循：

- Y 轴向上；
- Z 轴与本项目光轴方向一致，源到探测器为正 Z；
- 需要对准光轴的模型，其原点或命名基准点位于光学中心；
- 转台/样品模型的旋转轴穿过局部原点并沿 Y 轴；
- 应用变换后再导出，避免模型文件内部残留不可见的非均匀缩放。

如果资产无法重导出，应使用两层组分离职责：

```tsx
<group position={SAMPLE.center} rotation={[0, tableRotation, 0]}>
  {/* 内层只修正资产自身的原点和轴向 */}
  <group position={assetOriginCorrection} rotation={assetAxisCorrection}>
    <primitive object={sampleScene} scale={millimeterScale} />
  </group>
</group>
```

外层负责设备坐标和扫描旋转；内层只负责资产校正。不要把两类变换混在同一个难以追踪的 `position/rotation/scale` 中。

### 8.2 转台旋转边界

当前 `TurntableAndSample` 的旋转父组包含：

- 转盘；
- 角度标记；
- 样品。

转台底座位于父组外，因此不会随角度转动。导入 GLB 后也应保持这一层级。若一个 GLB 同时包含底座和转盘，建议在模型内保留可识别节点，并只把转盘节点及样品放入旋转组；否则整套转台底座会错误转动。

## 9. 材质和贴图

### 当前程序化设备造型

设备采用克制的日系卡通配色：射线源与相机主体为暖米白，盖板和剪叉支架为低饱和青绿，握把为柔和珊瑚色，圆轨保留可辨认的刻度与双轨接触面。金属件使用较高粗糙度和适度金属度，使轮组、丝杆、镜头和黄铜出束口在柔和灯光下仍有清晰层次。造型调整只涉及表面材质及剪叉臂圆角；光轴基准、源和相机比例、轨道半径、轮轨接触位置及反馈旋转组保持不变。

当前程序化几何使用 `meshStandardMaterial`、`meshPhysicalMaterial` 和 `meshBasicMaterial`，灯光由半球光和两盏方向光提供。外部模型建议优先使用 glTF 的金属度/粗糙度 PBR 工作流：

- Base Color：sRGB 色彩空间；
- Metallic/Roughness：线性数据贴图；
- Normal、AO：线性数据贴图；
- 发光贴图仅用于真正的指示灯或屏幕，不用它代替场景照明；
- 控制贴图分辨率，避免桌面应用首包和显存无必要增大；
- 尽量使用 2 的幂尺寸，并复用材质和贴图；
- 透明件需检查 `transparent`、`opacity`、`depthWrite` 和渲染顺序，避免与现有光束叠加时出现深度伪影。

如果需要让模型颜色随明暗主题变化，应明确哪些材质由主题覆盖。不要无条件替换 GLB 中所有材质，否则会丢失贴图、法线、金属度及美术设定。

## 10. 静态 fallback 必须同步维护

实时三维场景并非唯一显示路径。`StaticSceneFallback.tsx` 当前根据主题以及确认角度绝对值是否大于 0.005° 选择：

```text
/assets/3D-scene-light.png
/assets/3D-scene-light-144.png
/assets/3D-scene-dark.png
/assets/3D-scene-dark-144.png
```

触发原因包括：

- `webgl-unavailable`；
- `context-lost`。

如果设备造型、构图、颜色或关键旋转状态发生明显变化，必须重新生成上述静态图，否则实时场景和降级场景会表现不一致。静态图仍应放在 `public/assets`，并保持 `StaticSceneFallback.tsx` 使用的文件名契约，除非同步修改代码和所有引用。

## 11. CSP 与桌面打包注意事项

当前 Tauri CSP 为：

```text
default-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' data:;
connect-src ipc: http://ipc.localhost
```

接入模型时应注意：

1. **优先加载应用内同源资产。** `public/assets/models/micro-ct` 会进入 Vite 构建产物，符合 `'self'` 策略。
2. 不要直接从任意外部 CDN 加载 GLB、纹理或 Draco 解码器。外部资源不仅可能被 CSP 阻止，还会引入离线不可用、版本漂移和供应链风险。
3. 若确需新增协议或远程域名，必须最小化修改 `src-tauri/tauri.conf.json` 的 CSP，并验证开发模式与打包后的 Tauri 模式；不要用宽泛的 `*` 或关闭 CSP。
4. GLB 内嵌纹理最容易随包分发。分离式 glTF 必须把 `.gltf`、`.bin`、贴图及可选解码器一起纳入前端产物。
5. 文件名大小写和相对 URI 在开发服务器中“碰巧可用”，打包后不一定可用；提交前必须在 `npm run build` 产物和 Tauri 运行环境中各验证一次。
6. 当前 Vite 输出目录是 `dist/client`，Tauri 的 `frontendDist` 指向该目录。清理 `dist` 后必须重新执行构建才能运行基于构建产物的预览或打包流程。

## 12. 推荐接入步骤

1. 在 DCC/CAD 中确认毫米单位、Y 向上、Z 向前、原点和光学基准点。
2. 将 GLB 放入 `public/assets/models/micro-ct`，保持稳定、可移植的相对路径。
3. 用 Box3 检查尺寸，只做一次等比单位归一化。
4. 在对应 Assembly 中替换外观，保留 `scene-config.ts` 作为位置和光轴的唯一配置来源。
5. 样品及转盘放入现有 Y 轴旋转父组；固定底座和其他光学组件留在父组外。
6. 检查模型材质、贴图色彩空间、透明件和灯光效果。
7. 验证 `iso`、`front`、`top` 三个相机预设以及 OrbitControls 缩放边界。
8. 验证源焦点、样品、闪烁体、镜头仍共线，且 Z 顺序未改变。
9. 更新四张静态 fallback 图片。
10. 分别执行浏览器开发、Vite 构建产物和 Tauri 桌面打包/运行验证。

## 13. 资料来源

- `src/scene/EquipmentScene.tsx:21-173`：光束、设备组件、转台旋转、光轴线和灯光实现。
- `src/scene/scene-config.ts:3-69`：光轴常量、设备尺寸、相机约束、视角预设和启动断言。
- `src/scene/LiveSceneCanvas.tsx`：正交相机（`FRUSTUM_HEIGHT = 1200`，`near`/`far` 为 ±9000）、按投影包围盒自动取景、以 fit 为基准的 OrbitControls 缩放范围、`prefers-reduced-motion` 对阻尼的取消，以及 WebGL 上下文丢失处理。
- `src/scene/scene-fit.ts`：`collectSceneBounds`（含 `excludeFromFit` / `includeInFit` 规则）与 `computeFitZoom`、`FRUSTUM_HEIGHT`、`FIT_WIDTH`、`FIT_HEIGHT`。
- `tests/scene-framing.test.mjs`：三个预设下取景不越界的数值回归测试。
- `src/scene/StaticSceneFallback.tsx:3-29`：静态降级原因和图片路径规则。
- `src/App.tsx:343-373`：实时场景与静态 fallback 的选择入口。
- `vite.config.mjs:4-7`：前端构建输出目录 `dist/client`。
- `src-tauri/tauri.conf.json:6-20`：Tauri 前端产物目录、CSP 和打包配置。
