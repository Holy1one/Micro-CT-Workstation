/** English presentation of engine-provided native descriptor labels and reasons.
 *
 *  ct-engine keeps sending its raw native values and, for part of the D7100 catalog,
 *  Chinese labels: 15 of the 34 photographic `property.label`s, every enum
 *  `options[].label`, and the `disabledReason` / `options[].reason` /
 *  `capabilities.disabledReasons` texts. The renderer presents English and never
 *  mutates the record: raw codes, raw engine messages, the diagnostics JSON and the
 *  exported session log keep the engine's original text.
 *
 *  An unrecognised Chinese message is never machine-translated and never rendered as
 *  Chinese; it degrades to one neutral English sentence that points at the preserved
 *  raw text. An engine message that is already free of CJK passes through verbatim. */

/** Canonical English names for the 34 catalog keys (docs/frontend-functional-spec.md §4.1). */
export const NATIVE_PROPERTY_LABELS: Record<string, string> = {
  activeDLighting: "Active D-Lighting",
  aperture: "Aperture",
  autoIso: "Auto ISO",
  autoIsoHighLimit: "Auto ISO high limit",
  bracketing: "Automatic bracketing",
  burstNumber: "Burst number",
  centerWeightedArea: "Center-weighted area",
  colorSpace: "Color space",
  crop: "Capture area",
  driveMode: "Release mode",
  exposureCompensation: "Exposure compensation",
  exposureDelay: "Exposure delay",
  exposureMode: "Exposure mode",
  flashCompensation: "Flash compensation",
  flashMode: "Flash mode",
  flashSyncSpeed: "Flash sync speed",
  focusMode: "Focus mode",
  hdrExposureDifference: "HDR exposure difference",
  hdrMode: "HDR mode",
  hdrSmoothing: "HDR smoothing",
  highIsoNoiseReduction: "High ISO noise reduction",
  imageSize: "JPEG image size",
  iso: "ISO",
  longExposureNoiseReduction: "Long exposure noise reduction",
  meteringMode: "Metering mode",
  pictureControl: "Picture control",
  rawBitDepth: "RAW bit depth",
  rawCompression: "RAW compression",
  rawFormat: "Image format",
  recordingMedia: "Recording media",
  shutter: "Shutter",
  whiteBalance: "White balance",
  whiteBalanceFluorescent: "Fluorescent type",
  whiteBalanceTemperature: "White balance color temperature",
};

/** Enum labels keyed by native value. Values are the engine's own numeric codes, so a
 *  value the camera stops advertising simply stops being used. Language-neutral values
 *  (ISO, EV, f-numbers, Kelvin, seconds, millimetres) are absent on purpose and fall
 *  through to the engine label. */
export const NATIVE_ENUM_LABELS: Record<string, Record<string, string>> = {
  activeDLighting: { "0": "Extra high", "1": "High", "2": "Normal", "3": "Low", "4": "Off", "5": "Auto" },
  autoIso: { "0": "Off", "1": "On" },
  bracketing: { "0": "Off", "1": "On" },
  longExposureNoiseReduction: { "0": "Off", "1": "On" },
  centerWeightedArea: { "4": "Entire frame average" },
  driveMode: {
    "1": "Single", "2": "CH · Continuous high", "32784": "CL · Continuous low",
    "32785": "Self-timer", "32786": "Mirror up", "32790": "Quiet", "32791": "Remote",
  },
  exposureDelay: { "0": "3 s", "1": "2 s", "2": "1 s", "3": "Off" },
  exposureMode: {
    "1": "M · Manual", "2": "P · Program auto", "3": "A · Aperture priority", "4": "S · Shutter priority",
    "32784": "AUTO", "32785": "Portrait", "32786": "Landscape", "32787": "Macro", "32788": "Sports",
    "32790": "Auto (flash off)", "32791": "Child", "32792": "SCENE", "32793": "EFFECTS",
  },
  flashMode: {
    "2": "Flash off", "4": "Red-eye reduction", "32784": "Front-curtain sync",
    "32785": "Slow sync", "32786": "Rear-curtain sync", "32787": "Red-eye reduction slow sync",
  },
  flashSyncSpeed: { "0": "1/320 s auto FP", "1": "1/250 s auto FP" },
  focusMode: { "3": "Physical MF", "4": "Software MF" },
  hdrExposureDifference: { "0": "Auto" },
  hdrMode: { "0": "Off", "1": "On · single", "2": "On · series" },
  hdrSmoothing: { "0": "High", "1": "Normal", "2": "Low", "3": "Auto", "4": "Extra high (only when advertised)" },
  highIsoNoiseReduction: { "0": "Off", "1": "Low", "2": "Normal", "3": "High" },
  meteringMode: { "2": "Center-weighted", "3": "Matrix", "4": "Spot", "32784": "Highlight-weighted (only when advertised)" },
  pictureControl: {
    "1": "Standard", "2": "Neutral", "3": "Vivid", "4": "Monochrome", "5": "Portrait", "6": "Landscape",
    "201": "Custom Picture Control 1 (name not read)", "202": "Custom Picture Control 2 (name not read)",
    "203": "Custom Picture Control 3 (name not read)", "204": "Custom Picture Control 4 (name not read)",
    "205": "Custom Picture Control 5 (name not read)", "206": "Custom Picture Control 6 (name not read)",
    "207": "Custom Picture Control 7 (name not read)", "208": "Custom Picture Control 8 (name not read)",
    "209": "Custom Picture Control 9 (name not read)",
  },
  rawCompression: { "0": "Lossless compressed", "1": "Compressed", "2": "Uncompressed (only when advertised)" },
  rawFormat: {
    "3": "TIFF RGB (only when advertised)", "4": "NEF (RAW)",
    "5": "NEF + JPEG Basic", "6": "NEF + JPEG Normal", "7": "NEF + JPEG Fine",
  },
  recordingMedia: { "0": "Camera card (host capture unavailable)", "1": "SDRAM · host capture" },
  shutter: {
    "4294967293": "Unsupported special shutter value",
    "4294967294": "Time (not supported)",
    "4294967295": "Bulb (not supported)",
  },
  whiteBalance: {
    "2": "Auto", "4": "Daylight", "5": "Fluorescent", "6": "Incandescent", "7": "Flash",
    "32784": "Cloudy", "32785": "Shade", "32786": "Color temperature", "32787": "Preset manual",
    "32790": "Natural light auto (only when the camera advertises it)",
  },
  whiteBalanceFluorescent: {
    "0": "Sodium vapor", "1": "Cool-white fluorescent", "2": "Warm-white fluorescent",
    "3": "White fluorescent", "4": "Daylight fluorescent", "5": "Day white fluorescent",
    "6": "High color-temperature mercury vapor",
  },
};

const CJK = /[\u3400-\u9fff\u3000-\u303f\uff00-\uffef]/u;
export function hasCjk(text: string | null | undefined): boolean {
  return Boolean(text) && CJK.test(text!);
}

export function englishPropertyLabel(key: string, fallback?: string | null): string {
  const mapped = NATIVE_PROPERTY_LABELS[key];
  if (mapped) return mapped;
  return fallback && !hasCjk(fallback) ? fallback : key;
}

/** Engine option labels that carry no CJK are kept verbatim (ISO 1000, +1.333 EV, f/5.60 …).
 *  An unmapped Chinese label becomes a plain native-value readout rather than Chinese text. */
export function englishOptionLabel(key: string, value: number | string, fallback: string | undefined): string {
  const mapped = NATIVE_ENUM_LABELS[key]?.[String(value)];
  if (mapped) return mapped;
  return fallback && !hasCjk(fallback) ? fallback : `Native value ${value}`;
}

/** One neutral sentence for a Chinese reason this build does not map yet. It never
 *  invents a translation and never claims the condition is resolved. */
export const UNMAPPED_ENGINE_REASON =
  "The engine reports this parameter as unavailable or unconfirmed in the current mode. The original engine text, code and stage are preserved in the session log and in Camera → Advanced → Read-only diagnostics.";

const REASON_RULES: ReadonlyArray<readonly [RegExp, (match: RegExpExecArray) => string]> = [
  // Dependency reasons (properties.rs dependency_reason) — the common configuration order.
  [/^(.+)实际值未确认，暂不可修改此参数$/, m => `${m[1]} actual value is unconfirmed, so this parameter cannot be changed yet`],
  [/^当前模式由相机决定快门；M\/S模式可手动设置$/, () => "The current mode lets the camera choose the shutter; M/S enables a manual shutter"],
  [/^当前模式由相机决定光圈；M\/A模式可手动设置$/, () => "The current mode lets the camera choose the aperture; M/A enables a manual aperture"],
  [/^当前图像格式不包含NEF，RAW参数不生效$/, () => "The current image format contains no NEF, so RAW parameters have no effect"],
  [/^NEF使用固定RAW尺寸；此项只控制JPEG图像尺寸$/, () => "NEF uses a fixed RAW size; this setting only affects the JPEG image size"],
  [/^请先选择荧光灯白平衡并Apply$/, () => "Select Fluorescent white balance and Apply first"],
  [/^请先选择色温白平衡并Apply$/, () => "Select Color temperature white balance and Apply first"],
  [/^自动ISO关闭，自动ISO上限不生效$/, () => "Auto ISO is off, so the Auto ISO high limit has no effect"],
  [/^请先选择中央重点测光并Apply$/, () => "Select Center-weighted metering and Apply first"],
  [/^NEF格式不可使用HDR$/, () => "HDR is unavailable with NEF"],
  [/^HDR关闭，HDR子参数不生效$/, () => "HDR is off, so its sub-parameters have no effect"],
  [/^HDR不能与NEF\/NEF\+JPEG格式同时启用；OFF仍可用$/, () => "HDR cannot be enabled together with NEF / NEF + JPEG; OFF stays available"],
  [/^图像格式实际值未确认，不能启用HDR$/, () => "The image format's actual value is unconfirmed, so HDR cannot be enabled"],
  [/^D7100包围属性原生代码\/UINT8类型未确认，禁止修改或推断OFF$/, () => "The D7100 bracketing property's native code / UINT8 type is unconfirmed: it cannot be modified and OFF cannot be inferred"],
  [/^D7100包围属性原生代码\/UINT8类型未确认$/, () => "The D7100 bracketing property's native code / UINT8 type is unconfirmed"],
  [/^D7100包围属性必须经当前广告及UINT8描述验证后明确Apply$/, () => "D7100 bracketing has to be explicitly applied once the current advertisement and UINT8 description are verified"],
  [/^物理MF开关锁定；请在镜头或机身切换后重新读取$/, () => "Physical MF is locked by the lens or body switch; change it there and Read again"],
  [/^物理MF开关锁定，禁止用草稿目标绕过硬件模式$/, () => "Physical MF is locked by a hardware switch; a draft target cannot bypass it"],
  [/^物理MF由机身\/镜头开关选择，只读显示；软件目标请使用受支持的软件MF$/, () => "Physical MF is selected on the body or lens and is display-only; use the supported Software MF for a software target"],
  [/^Bulb\/Time\/闪光同步特殊值未实现有界主机曝光，暂不支持$/, () => "Bulb, Time and flash-sync special values have no bounded host exposure and are not supported"],
  // Live View control and capability reasons.
  [/^当前Live View会话已停止\/未确认$/, () => "The current Live View session is stopped or unconfirmed"],
  [/^LV物理MF开关锁定；不能用目标值绕过$/, () => "Live View physical MF is locked by a switch; a target value cannot bypass it"],
  [/^LV焦点模式原生值语义未知，禁止修改$/, () => "The Live View focus mode's native value has unknown semantics and cannot be changed"],
  [/^LV实际读回未确认或与目标不一致；请停止后重新启动Live View建立新控制会话$/, () => "The Live View readback is unconfirmed or differs from the target; stop Live View, start it again and re-read"],
  [/^LV实际读回与本次目标不一致；请停止后重新启动Live View建立新控制会话$/, () => "The Live View readback differs from this target; stop Live View, start it again and re-read"],
  [/^设备LV描述只读或缺少可编辑枚举\/范围$/, () => "The device describes this Live View control as read-only, or without an editable enum or range"],
  [/^当前LV枚举\/范围未支持该值$/, () => "The current Live View enum or range does not support this value"],
  [/^该LV模式只读或语义不支持修改$/, () => "This Live View mode is read-only or its semantics do not support changes"],
  [/^当前Live View会话不匹配或已停止$/, () => "The Live View session does not match or has stopped"],
  [/^LV状态读取资格已失效；当前会话隔离$/, () => "Live View status eligibility expired; the current session is isolated"],
  [/^相机读回Live View已停止；旧控制会话失效$/, () => "The camera reports Live View stopped; the old control session is retired"],
  [/^相机读回Live View已停止；请重新启动建立新会话$/, () => "The camera reports Live View stopped; start it again to establish a new session"],
  [/^LV状态读回未知；当前会话隔离$/, () => "Live View status readback is unknown; the current session is isolated"],
  [/^当前设备未广告此LV属性或读取操作；未试探属性$/, () => "The device does not advertise this Live View property or read operation; no probing was attempted"],
  [/^LV控制描述不是严格UINT8；不读取、写入或猜测值$/, () => "The Live View control is not described as a strict UINT8; nothing is read, written or guessed"],
  [/^LV控制实际值不符合UINT8$/, () => "The Live View control's actual value is not a valid UINT8"],
  [/^本次设备未广告此诊断；未发送读取或写入$/, () => "The device did not advertise this diagnostic in this session; no read or write was sent"],
  [/^本次设备未广告此诊断；不试读$/, () => "The device did not advertise this diagnostic in this session; no trial read was sent"],
  [/^LV设备身份\/能力重读失败，当前会话隔离：(.+)$/, m => `Live View identity/capability re-read failed and the session is isolated: ${m[1]}`],
  [/^LV专用命令只接受cameraZoom\/focusMode，至少一项$/, () => "Live View-only commands accept cameraZoom and focusMode, and at least one of them"],
  [/^LV焦点模式只允许有据的AF-S\/AF-F\/软件MF目标；物理MF只读$/, () => "Live View focus mode accepts only evidenced AF-S / AF-F / Software MF targets; Physical MF is read-only"],
  [/^LV值未获实际描述\/当前资格支持$/, () => "This Live View value is not backed by an actual description or current eligibility"],
  [/^LV SET只能编码一字节UINT8$/, () => "A Live View SET can encode exactly one UINT8 byte"],
  [/^当前LV镜头\/照片选择器\/D061资格未确认，或物理MF\/AF-F禁止该驱动$/, () => "Live View lens / photo-selector / D061 eligibility is unconfirmed, or physical MF / AF-F prohibits this drive"],
  [/^镜头步进需要当前Live View及鲜读D061\/镜头资格$/, () => "Lens stepping needs an active Live View plus freshly read D061 / lens eligibility"],
  [/^正常焦点模式或镜头驱动资格未确认$/, () => "Normal focus mode or lens drive eligibility is unconfirmed"],
  // Ordinary media / capture admission.
  [/^Live View开启期间禁止参数Apply；请先明确停止预览$/, () => "Parameter Apply is prohibited while Live View is active; stop the preview explicitly first"],
  [/^Live View开启期间禁止普通摄影参数Apply；请先明确停止预览$/, () => "Photographic parameter Apply is prohibited while Live View is active; stop the preview explicitly first"],
  [/^Live View开启期间禁止拍摄；请先明确停止预览$/, () => "Capture is prohibited while Live View is active; stop the preview explicitly first"],
  [/^相机未声明此属性；未发送读取或写入操作$/, () => "The camera does not describe this property; no read or write was sent"],
  [/^当前相机未提供可验证的属性描述$/, () => "The camera provides no verifiable property description"],
  [/^相机会话不可用，需重新连接并读取$/, () => "The camera session is unavailable; connect again and read"],
  [/^属性实际读回未确认，需成功重新读取后修改$/, () => "The property's actual readback is unconfirmed; a successful read is required before a change"],
  [/^相机未声明属性写入操作$/, () => "The camera does not advertise a property write operation"],
  [/^相机当前属性描述为只读；需检查机身模式、镜头或控制状态$/, () => "The camera describes this property as read-only; check the body mode, lens or control state"],
  [/^相机未描述可安全编辑的枚举或范围$/, () => "The camera describes no enum or range that is safe to edit"],
  [/^Live View开启或相机未声明属性写入操作$/, () => "Live View is active, or the camera does not advertise a property write operation"],
  [/^属性(.+)没有可验证描述$/, m => `Property ${m[1]} has no verifiable description`],
  [/^属性(.+)不可写或选项未获当前设备描述支持；模式变化请先Apply后重新读取$/, m => `Property ${m[1]} is not writable, or its options are not backed by the current device description; for a mode change, Apply first and read again`],
  [/^属性(.+)：(.+)$/, m => `Property ${m[1]}: ${m[2]}`],
  [/^运行诊断尚未成功读取，拍摄前需重新确认$/, () => "The run diagnostics have not been read successfully; confirm them again before capture"],
  [/^运行诊断的类型、只读描述或实际读回未确认$/, () => "The run diagnostics' type, read-only description or actual readback is unconfirmed"],
  [/^只读运行诊断；不进入摄影Apply或预设$/, () => "Read-only run diagnostic; it is not part of photographic Apply or presets"],
  [/^NEF主机采集要求已声明的HDR属性确认关闭；不得用RAW设置覆盖矛盾\/未知HDR状态$/, () => "NEF host capture requires a declared HDR property confirmed off; a RAW setting may not override a contradictory or unknown HDR state"],
  // Label-side fallbacks produced by value_label.
  [/^原生值 (.+)$/, m => `Native value ${m[1]}`],
  [/^原生值 (.+)（包围类型未验证）$/, m => `Native value ${m[1]} (bracketing type unverified)`],
  [/^(.+)张$/, m => `${m[1]} frame(s)`],
  [/^特殊快门值 (.+)（暂不支持）$/, m => `Special shutter value ${m[1]} (not supported)`],
  [/^自定义Picture Control (.+)（名称未读取）$/, m => `Custom Picture Control ${m[1]} (name not read)`],
  [/^ISO（自动ISO基准值，非实测曝光ISO）$/, () => "ISO (Auto ISO baseline, not the measured exposure ISO)"],
  [/^(.+)（仅设备声明时可用）$/, m => `${m[1]} (only when the camera advertises it)`],
];

/** English presentation of an engine reason. A message without CJK passes through
 *  verbatim; an unmapped Chinese message degrades to one neutral sentence (the raw
 *  text stays in the log, the diagnostics pane and the export). */
export function englishReason(message: string | null | undefined): string {
  if (!message) return "";
  for (const [pattern, render] of REASON_RULES) {
    const match = pattern.exec(message);
    if (match) return render(match);
  }
  return hasCjk(message) ? UNMAPPED_ENGINE_REASON : message;
}
