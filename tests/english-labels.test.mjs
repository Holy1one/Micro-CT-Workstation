/** The product interface is English even though part of the engine catalog is not:
 *  15 of the 34 native labels, every enum option label and every disabled reason arrive
 *  in Chinese from ct-engine. The renderer maps them; the raw record keeps its text. */
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { loadSourceModule } from "./load-source-module.mjs";

const CJK = /[\u3400-\u9fff\u3000-\u303f\uff00-\uffef]/u;
const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const firstLaunch = JSON.parse(read("tests/first-launch-response.jsonl").trim().split(/\r?\n/)[0]);
// The authoritative catalog is the engine's own table. (The first-launch response is
// captured before any camera is connected, so its property map is empty.)
const nikonSource = read("crates/ct-engine/src/devices/camera/nikon.rs");
const catalog = [...nikonSource
  .slice(nikonSource.indexOf("pub(crate) const NIKON_PROPERTIES"), nikonSource.indexOf("const STANDARD_PROPERTIES"))
  .matchAll(/\(\s*"([a-zA-Z]+)"/g)].map((match) => match[1]);
const nativeLabels = await loadSourceModule("src/engine/native-labels.ts");
const controls = await loadSourceModule("src/engine/deviceControls.ts");
const { CAMERA_PROPERTY_GROUPS } = await loadSourceModule("src/CameraSettingsDialog.tsx");
const {
  NATIVE_PROPERTY_LABELS, NATIVE_ENUM_LABELS, UNMAPPED_ENGINE_REASON, englishOptionLabel, englishPropertyLabel, englishReason,
} = nativeLabels;
const { cameraCapturePreparationReason, cameraPropertyLabel, cameraPropertyReason, cameraPropertyValueLabel } = controls;

test("the 34-key catalog is fully covered by the Advanced categories", () => {
  assert.equal(catalog.length, 34);
  const grouped = CAMERA_PROPERTY_GROUPS.flatMap((entry) => [...entry.keys]);
  assert.deepEqual([...grouped].sort(), [...catalog].sort());
  assert.equal(new Set(grouped).size, grouped.length, "a key must not be listed twice");
  assert.deepEqual(CAMERA_PROPERTY_GROUPS.map((entry) => entry.keys.length), [9, 5, 10, 7, 3]);
});

test("every native parameter has an English title and the tables stay keyed to the catalog", () => {
  for (const key of catalog) {
    const label = englishPropertyLabel(key, undefined);
    assert.ok(label && !CJK.test(label), `${key} needs an English title`);
  }
  for (const key of Object.keys(NATIVE_PROPERTY_LABELS)) assert.ok(catalog.includes(key), `${key} is not a catalog key`);
  for (const key of Object.keys(NATIVE_ENUM_LABELS)) assert.ok(catalog.includes(key), `${key} is not a catalog key`);
  // An already-English engine label is never replaced by the key fallback.
  assert.equal(englishPropertyLabel("iso", "ISO"), "ISO");
  // A Chinese engine label can never leak through.
  assert.equal(englishPropertyLabel("meteringMode", "曝光测光"), "Metering mode");
  assert.equal(englishPropertyLabel("unknownKey", "曝光测光"), "unknownKey");
});

test("white balance and the colour-temperature parent mode are named in English", () => {
  for (const value of ["2", "4", "5", "6", "7", "32784", "32785", "32786", "32787", "32790"]) {
    const label = englishOptionLabel("whiteBalance", value, "自动");
    assert.ok(label && !CJK.test(label), `whiteBalance ${value} needs an English label`);
  }
  assert.equal(englishOptionLabel("whiteBalance", 32786, "色温"), "Color temperature");
  assert.equal(englishOptionLabel("whiteBalance", 2, "自动"), "Auto");
  assert.equal(englishOptionLabel("recordingMedia", 1, "SDRAM 主机采集"), "SDRAM · host capture");
});

test("language-neutral engine labels pass through and unmapped Chinese never renders", () => {
  for (const value of ["ISO 1000", "+1.333 EV", "f/5.60", "6000x4000", "5000 K", "1/60.0000 s"]) {
    assert.equal(englishOptionLabel("iso", 1, value), value, "a neutral engine label is kept verbatim");
  }
  assert.doesNotMatch(englishOptionLabel("exposureMode", 32848, "原生值 32848"), CJK);
  assert.equal(englishOptionLabel("exposureMode", 32848, "原生值 32848"), "Native value 32848");
  assert.doesNotMatch(englishOptionLabel("unknownKey", "x", "完全未知"), CJK);
});

test("engine reasons are presented in English without inventing a translation", () => {
  assert.equal(englishReason("物理MF开关锁定；请在镜头或机身切换后重新读取"), "Physical MF is locked by the lens or body switch; change it there and Read again");
  assert.equal(englishReason("请先选择色温白平衡并Apply"), "Select Color temperature white balance and Apply first");
  assert.match(englishReason("liveViewMode实际值未确认，暂不可修改此参数"), /liveViewMode actual value is unconfirmed/);
  // Already-English text (readiness remedy, error codes) is returned verbatim.
  assert.equal(englishReason("Read the camera properties, then repeat Preflight"), "Read the camera properties, then repeat Preflight");
  assert.equal(englishReason(""), "");
  assert.equal(englishReason(null), "");
  // An unmapped Chinese message degrades to one neutral sentence, never to Chinese.
  const unknown = englishReason("这是一个未知的引擎消息");
  assert.equal(unknown, UNMAPPED_ENGINE_REASON);
  assert.doesNotMatch(unknown, CJK);
  assert.match(unknown, /preserved in the session log/);
});

test("the camera label helpers never return Chinese for a Chinese-descriptor property", () => {
  const property = {
    key: "whiteBalance", label: "白平衡", code: 0x5005, supported: true, readable: true, nativeWritable: true,
    effective: true, writable: false, disabledReason: "请先选择色温白平衡并Apply", confirmed: true, actual: 4,
    options: [{ value: 2, label: "自动", supported: true, enabled: true, reason: null }, { value: 4, label: "晴天", supported: true, enabled: true, reason: "物理MF开关锁定；请在镜头或机身切换后重新读取" }],
  };
  assert.equal(cameraPropertyLabel(property), "White balance");
  assert.equal(cameraPropertyValueLabel(property, 2), "Auto");
  assert.equal(cameraPropertyValueLabel(property, 4), "Daylight");
  assert.equal(cameraPropertyValueLabel(property, null), "UNKNOWN");
  assert.match(cameraPropertyReason(property), /Select Color temperature white balance and Apply first/);
  const capture = cameraCapturePreparationReason({ capabilities: { disabledReasons: { captureReady: "运行诊断尚未成功读取，拍摄前需重新确认" } }, properties: {} });
  assert.doesNotMatch(capture, CJK);
  assert.match(capture, /confirm them again before capture/);
});

test("engine-provided readiness evidence stays English and is presented as-is", () => {
  const readiness = firstLaunch.payload.cameraScanReadiness;
  if (readiness?.issues?.length) {
    for (const issue of readiness.issues) {
      assert.doesNotMatch(issue.requirement, CJK);
      assert.doesNotMatch(issue.remedy, CJK);
    }
  }
  assert.ok(read("src/ScanPreparationDialog.tsx").includes("{issue.requirement}"));
});

test("no product-interface module carries bare Chinese text", () => {
  const modules = [
    "src/CameraPanel.tsx", "src/CameraSettingsDialog.tsx", "src/ScanPreparationDialog.tsx",
    "src/WorkstationDialog.tsx", "src/FailureBanner.tsx", "src/TroubleshootingPane.tsx", "src/TurntablePanel.tsx",
    "src/DeviceConsoles.tsx", "src/WorkstationMedia.tsx", "src/ReconstructionConsole.tsx", "src/TaskRecoveryDialog.tsx",
    "src/UpdateDialog.tsx", "src/menuActions.ts", "src/camera-draft.ts", "src/platform/updater.ts",
    "src/engine/deviceControls.ts", "src/engine/workstationAdapter.ts", "src/engine/types.ts",
  ];
  for (const module of modules) {
    const match = CJK.exec(read(module));
    assert.equal(match, null, `${module} still renders Chinese text: ${match?.[0]}`);
  }
  // The one module allowed to hold Chinese is the presentation layer's own phrase table.
  assert.match(read("src/engine/native-labels.ts"), CJK);
});

test("App.tsx holds Chinese only inside its engine-message mapping table", () => {
  // The source escapes Chinese as \uXXXX; decode first so an escaped label cannot hide.
  const decoded = read("src/App.tsx").replace(/\\u([0-9a-fA-F]{4})/g, (_, hex) => String.fromCharCode(parseInt(hex, 16)));
  const start = decoded.indexOf("function reconstructionMessageEn(");
  assert.ok(start > 0, "the reconstruction message mapping must stay present");
  const nextFunction = decoded.indexOf("\nfunction ", start + 10);
  const end = nextFunction === -1 ? decoded.length : nextFunction;
  const outside = [];
  const re = new RegExp(CJK.source, "gu");
  let match;
  while ((match = re.exec(decoded))) if (match.index < start || match.index >= end) outside.push(`${match.index}:${match[0]}`);
  assert.deepEqual(outside, [], "App.tsx must not render engine Chinese outside that mapping");
});
