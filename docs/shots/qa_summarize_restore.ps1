$ErrorActionPreference = 'Stop'
$path = 'E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App\docs\shots\window-geometry-restore-minmax.json'
$out = 'E:\Main\OneDrive\LanZhouUniv\Class\mluti-energy imaging\CT\Micro-CT-App\docs\shots\window-geometry-restore-minmax.txt'
$j = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
$lines = @()
$lines += "HWND=$($j.hwnd)"
foreach ($s in $j.steps) {
    $st = $s.state
    $sm = $s.sample
    $lines += "---- $($s.step) ----"
    $lines += "  isZoomed=$($st.isZoomed) WS_MAXIMIZE=$($st.WS_MAXIMIZE) showCmd=$($st.showCmd)"
    $lines += "  outer=[$($st.outer -join ',')] size=$($st.outerSize -join 'x')"
    $lines += "  client=$($st.clientSize -join 'x')"
    $lines += "  rcNormalPosition=[$($st.rcNormalPosition -join ',')]"
    $lines += "  dwmVisible=[$($sm.dwmVisibleFrame -join ',')]"
    $lines += "  rcWork=[$($sm.monitor.rcWork -join ',')]"
    $lines += "  dom inner=$($sm.dom.innerWidth)x$($sm.dom.innerHeight) zoom=$($sm.dom.designCanvasZoom)"
    $lines += "  dom canvas=$([math]::Round($sm.dom.designCanvas.width,2))x$([math]::Round($sm.dom.designCanvas.height,2)) x=$([math]::Round($sm.dom.designCanvas.x,2)) y=$([math]::Round($sm.dom.designCanvas.y,2))"
    $lines += "  expected zoom=$($sm.expectedCanvas.zoom) gutterX=$([math]::Round($sm.expectedCanvas.gutterX,2)) gutterY=$([math]::Round($sm.expectedCanvas.gutterY,2))"
    $lines += "  foregroundMatch=$($sm.evidence.foregroundMatchesTarget) crop=$($sm.evidence.windowCropPng)"
}
$lines | Set-Content -LiteralPath $out -Encoding UTF8
Write-Output "WROTE $out"
