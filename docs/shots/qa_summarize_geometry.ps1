$ErrorActionPreference = 'Stop'
$path = 'E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App\docs\shots\window-geometry-summary.json'
$out = 'E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App\docs\shots\window-geometry-summary.txt'
$j = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
$lines = @()
$lines += "PRIMARY_WORK_AREA=$($j.primaryWorkArea -join ',')"
$lines += "HWND=$($j.hwnd)"
foreach ($s in $j.samples) {
    $lines += "---- $($s.label) ----"
    $lines += "  isZoomed=$($s.isZoomed) showCmd=$($s.showCmd) foregroundMatch=$($s.evidence.foregroundMatchesTarget)"
    $lines += "  osOuter=[$($s.osOuterRect -join ',')] size=$($s.osOuterSize -join 'x')"
    $lines += "  dwmVisible=[$($s.dwmVisibleFrame -join ',')]"
    $lines += "  clientScreen=[$($s.clientScreenRect -join ',')] size=$($s.clientSize -join 'x')"
    $lines += "  monitor rcMonitor=[$($s.monitor.rcMonitor -join ',')] rcWork=[$($s.monitor.rcWork -join ',')] primary=$($s.monitor.primary) dpi=$($s.monitor.dpiX) scale=$($s.monitor.scalePercent)%"
    $lines += "  dom inner=$($s.dom.innerWidth)x$($s.dom.innerHeight) dpr=$($s.dom.devicePixelRatio)"
    $lines += "  dom canvas=$([math]::Round($s.dom.designCanvas.width,2))x$([math]::Round($s.dom.designCanvas.height,2)) at x=$([math]::Round($s.dom.designCanvas.x,2)) y=$([math]::Round($s.dom.designCanvas.y,2)) zoom=$($s.dom.designCanvasZoom)"
    $lines += "  expected zoom=$($s.expectedCanvas.zoom) canvas=$([math]::Round($s.expectedCanvas.scaledWidth,2))x$([math]::Round($s.expectedCanvas.scaledHeight,2)) gutterX=$([math]::Round($s.expectedCanvas.gutterX,2)) gutterY=$([math]::Round($s.expectedCanvas.gutterY,2))"
    $lines += "  crop=$($s.evidence.windowCropPng)"
}
$lines | Set-Content -LiteralPath $out -Encoding UTF8
Write-Output "WROTE $out"
