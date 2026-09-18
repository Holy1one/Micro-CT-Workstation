$ErrorActionPreference = 'Stop'
$path = 'E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App\docs\shots\window-geometry-cross-monitor.json'
$out = 'E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App\docs\shots\window-geometry-cross-monitor.txt'
$j = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
$lines = @()
$lines += "HWND=$($j.hwnd)"
foreach ($s in $j.steps) {
    $sm = $s.sample
    $lines += "---- $($s.step) ----"
    $lines += "  isZoomed=$($sm.isZoomed) showCmd=$($sm.showCmd)"
    $lines += "  outer=[$($sm.osOuterRect -join ',')]"
    $lines += "  dwmVisible=[$($sm.dwmVisibleFrame -join ',')]"
    $lines += "  client=[$($sm.clientScreenRect -join ',')] size=$($sm.clientSize -join 'x')"
    $lines += "  monitor rcMonitor=[$($sm.monitor.rcMonitor -join ',')] rcWork=[$($sm.monitor.rcWork -join ',')] primary=$($sm.monitor.primary) dpiX=$($sm.monitor.dpiX)"
    $lines += "  dom inner=$($sm.dom.innerWidth)x$($sm.dom.innerHeight) dpr=$($sm.dom.devicePixelRatio) zoom=$($sm.dom.designCanvasZoom)"
    $lines += "  dom canvas=$([math]::Round($sm.dom.designCanvas.width,2))x$([math]::Round($sm.dom.designCanvas.height,2)) x=$([math]::Round($sm.dom.designCanvas.x,2)) y=$([math]::Round($sm.dom.designCanvas.y,2))"
    $lines += "  expected zoom=$($sm.expectedCanvas.zoom) gutterX=$([math]::Round($sm.expectedCanvas.gutterX,2)) gutterY=$([math]::Round($sm.expectedCanvas.gutterY,2))"
    $lines += "  foregroundMatch=$($sm.evidence.foregroundMatchesTarget) crop=$($sm.evidence.windowCropPng)"
}
$lines | Set-Content -LiteralPath $out -Encoding UTF8
Write-Output "WROTE $out"
