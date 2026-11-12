# Glint end-to-end scenario. Runs on a hidden desktop (see run.ps1), so nothing appears on screen.
# Uses its own config, data folder and daemon instance, and never touches the clipboard.
param([Parameter(Mandatory)] [string] $Bin)
$ErrorActionPreference = 'Stop'

Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint type);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, System.Text.StringBuilder s, int n);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@

# Posted mouse messages are rescaled between DPI-unaware and DPI-aware processes, so match Glint.
[W]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null

$work = Join-Path $env:TEMP "glint-e2e-$PID"
New-Item -ItemType Directory -Force "$work\out", "$work\local" | Out-Null
$env:LOCALAPPDATA = "$work\local"
$env:GLINT_CONFIG = "$work\config.toml"
$env:GLINT_TEST_DIR = "$work\out"
$env:GLINT_INSTANCE = "e2e-$PID"
# A hidden desktop has no screen image to copy, so the daemon paints a gradient instead.
# Real capture is covered by the glint-capture unit tests.
$env:GLINT_TEST_SYNTHETIC = '1'
# One theme for pixel checks.
$env:GLINT_THEME = 'dark'
# No global hotkeys in tests: they would steal keys from the owner's apps.
@"
preview_seconds = 30
[hotkeys]
region = ""
window = ""
fullscreen = ""
repeat = ""
"@ | Set-Content -Encoding utf8 $env:GLINT_CONFIG

$script:fails = 0
function Check([string] $name, [bool] $ok, [string] $detail = '') {
  if ($ok) { Write-Output "PASS  $name $detail" } else { Write-Output "FAIL  $name $detail"; $script:fails++ }
}
function Wait-Until([scriptblock] $cond, [int] $ms = 5000) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($sw.ElapsedMilliseconds -lt $ms) { if (& $cond) { return $true }; Start-Sleep -Milliseconds 25 }
  return $false
}
# The editor's window (its title carries the image size, so it is found by process).
function EdWin($p) { $p.Refresh(); $p.MainWindowHandle }
function Title([IntPtr] $h) { $sb = New-Object System.Text.StringBuilder 256; [W]::GetWindowText($h, $sb, 256) | Out-Null; $sb.ToString() }
function Png-Size([string] $path) {
  $b = [IO.File]::ReadAllBytes($path)
  $w = ([int]$b[16] -shl 24) -bor ([int]$b[17] -shl 16) -bor ([int]$b[18] -shl 8) -bor [int]$b[19]
  $h = ([int]$b[20] -shl 24) -bor ([int]$b[21] -shl 16) -bor ([int]$b[22] -shl 8) -bor [int]$b[23]
  return "${w}x${h}"
}
function Glint([string[]] $argv) { Start-Process -FilePath "$Bin\glint.exe" -ArgumentList $argv -Wait }
function Out-File-N([int] $n) { "$work\out\capture-$n.png" }
function LParam([int] $x, [int] $y) { [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF)) }
function Kib([long] $b) { [math]::Round($b / 1KB) }

$daemon = Start-Process -FilePath "$Bin\glint.exe" -ArgumentList '--background' -PassThru
try {
  $found = Wait-Until { [W]::FindWindow('GlintDaemon', [NullString]::Value) -ne [IntPtr]::Zero }
Check 'daemon starts' $found "(window $([W]::FindWindow('GlintDaemon', [NullString]::Value)))"

  Start-Sleep -Milliseconds 1500
  $daemon.Refresh()
  $idle = $daemon.PrivateMemorySize64
  Check 'idle private memory under 10 MB' ($idle -lt 10MB) "($(Kib $idle) KB private, $(Kib $daemon.WorkingSet64) KB working set)"

  # 1. A fixed rectangle, no overlay.
  Glint @('rect', '10', '20', '200', '100')
  Check 'rect capture written' (Wait-Until { Test-Path (Out-File-N 1) })
  if (Test-Path (Out-File-N 1)) { Check 'rect capture size' ((Png-Size (Out-File-N 1)) -eq '200x100') (Png-Size (Out-File-N 1)) }
  Check 'preview card shown' (Wait-Until { [W]::FindWindow('GlintPreview', [NullString]::Value) -ne [IntPtr]::Zero })

  # 2. Full screen of the monitor under the cursor.
  Glint @('fullscreen')
  Check 'fullscreen capture written' (Wait-Until { Test-Path (Out-File-N 2) })
  if (Test-Path (Out-File-N 2)) { Write-Output "      fullscreen size $(Png-Size (Out-File-N 2))" }

  # 3. Region overlay: drag from (100,100) to (400,300) in client pixels; letting go snips (default).
  Glint @('region')
  Check 'overlay opens' (Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero })
  $ov = [W]::FindWindow('GlintOverlay', [NullString]::Value)
  if ($env:GLINT_TEST_DUMP) {      # keep the overlay as painted (the hint with mouse icons)
    Start-Sleep -Milliseconds 300
    Copy-Item -Force "$work\out\overlay-paint.png" "$PSScriptRoot\..\..\.e2e\overlay-hint.png" -ErrorAction SilentlyContinue
  }
  [W]::PostMessage($ov, 0x0200, [IntPtr]0, (LParam 100 100)) | Out-Null   # WM_MOUSEMOVE
  [W]::PostMessage($ov, 0x0201, [IntPtr]1, (LParam 100 100)) | Out-Null   # WM_LBUTTONDOWN
  foreach ($i in 1..10) { [W]::PostMessage($ov, 0x0200, [IntPtr]1, (LParam (100 + 30 * $i) (100 + 20 * $i))) | Out-Null }
  [W]::PostMessage($ov, 0x0202, [IntPtr]0, (LParam 400 300)) | Out-Null   # WM_LBUTTONUP
  Check 'overlay closes on release' (Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -eq [IntPtr]::Zero })
  Check 'region capture written' (Wait-Until { Test-Path (Out-File-N 3) })
  if (Test-Path (Out-File-N 3)) { Check 'region capture size' ((Png-Size (Out-File-N 3)) -eq '301x201') (Png-Size (Out-File-N 3)) }

  # capture_on_release = false: the box waits for adjustments and Enter.
  (Get-Content $env:GLINT_CONFIG) -replace '^preview_seconds = 30$', "preview_seconds = 30`ncapture_on_release = false" | Set-Content -Encoding utf8 $env:GLINT_CONFIG
  (Get-Item $env:GLINT_CONFIG).LastWriteTime = (Get-Date).AddSeconds(5)
  Check 'settings reloaded' (Wait-Until { (Get-Content "$work\local\Glint\glint.log" -ErrorAction SilentlyContinue) -match 'settings reloaded' })

  # 4. Arrow keys adjust the selection: Right moves, Shift is not held so size stays.
  Glint @('region')
  Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero } | Out-Null
  $ov = [W]::FindWindow('GlintOverlay', [NullString]::Value)
  [W]::PostMessage($ov, 0x0201, [IntPtr]1, (LParam 50 50)) | Out-Null
  [W]::PostMessage($ov, 0x0200, [IntPtr]1, (LParam 149 99)) | Out-Null
  [W]::PostMessage($ov, 0x0202, [IntPtr]0, (LParam 149 99)) | Out-Null
  Start-Sleep -Milliseconds 100
  Check 'overlay waits for Enter' ([W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero)
  [W]::PostMessage($ov, 0x0100, [IntPtr]0x27, [IntPtr]0) | Out-Null        # Right arrow
  [W]::PostMessage($ov, 0x0100, [IntPtr]0x0D, [IntPtr]0) | Out-Null
  Check 'nudged capture written' (Wait-Until { Test-Path (Out-File-N 4) })
  if (Test-Path (Out-File-N 4)) { Check 'nudged capture size' ((Png-Size (Out-File-N 4)) -eq '100x50') (Png-Size (Out-File-N 4)) }

  # 5. Esc cancels: no new file.
  Glint @('region')
  Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero } | Out-Null
  [W]::PostMessage([W]::FindWindow('GlintOverlay', [NullString]::Value), 0x0100, [IntPtr]0x1B, [IntPtr]0) | Out-Null
  Check 'Esc closes the overlay' (Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -eq [IntPtr]::Zero })
  Start-Sleep -Milliseconds 300
  Check 'Esc writes nothing' (-not (Test-Path (Out-File-N 5)))

  # 6. Repeat the last region (the nudged one).
  Glint @('repeat')
  Check 'repeat capture written' (Wait-Until { Test-Path (Out-File-N 5) })
  if (Test-Path (Out-File-N 5)) { Check 'repeat capture size' ((Png-Size (Out-File-N 5)) -eq '100x50') (Png-Size (Out-File-N 5)) }

  # 6b. Middle-click in the overlay repeats the last area (100x50).
  Glint @('region')
  Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero } | Out-Null
  [W]::PostMessage([W]::FindWindow('GlintOverlay', [NullString]::Value), 0x0208, [IntPtr]0, (LParam 500 500)) | Out-Null  # WM_MBUTTONUP
  Check 'middle-click capture written' (Wait-Until { Test-Path (Out-File-N 6) })
  if (Test-Path (Out-File-N 6)) { Check 'middle-click repeats the last area' ((Png-Size (Out-File-N 6)) -eq '100x50') (Png-Size (Out-File-N 6)) }

  # 6c. Right-click in the overlay snips the whole monitor (same size as the fullscreen command).
  Glint @('region')
  Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero } | Out-Null
  $ov = [W]::FindWindow('GlintOverlay', [NullString]::Value)
  [W]::PostMessage($ov, 0x0200, [IntPtr]0, (LParam 300 300)) | Out-Null
  [W]::PostMessage($ov, 0x0205, [IntPtr]0, (LParam 300 300)) | Out-Null  # WM_RBUTTONUP
  Check 'right-click capture written' (Wait-Until { Test-Path (Out-File-N 7) })
  if ((Test-Path (Out-File-N 7)) -and (Test-Path (Out-File-N 2))) {
    Check 'right-click snips the monitor' ((Png-Size (Out-File-N 7)) -eq (Png-Size (Out-File-N 2))) (Png-Size (Out-File-N 7))
  }

  # 6d. The hotkey (region command) a second time closes the overlay without a capture.
  Glint @('region')
  Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero } | Out-Null
  Glint @('region')
  Check 'second press closes the overlay' (Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -eq [IntPtr]::Zero })
  Start-Sleep -Milliseconds 300
  Check 'second press captures nothing' (-not (Test-Path (Out-File-N 8)))

  # 7. Memory after captures, once the cards are gone.
  Start-Sleep -Milliseconds 500
  $daemon.Refresh()
  Write-Output "      after captures: $(Kib $daemon.PrivateMemorySize64) KB private, $(Kib $daemon.WorkingSet64) KB working set"

  $log = Get-Content "$work\local\Glint\glint.log" -ErrorAction SilentlyContinue
  $log | Select-String 'overlay shown|captured|error|failed' | ForEach-Object { Write-Output "      log: $($_.Line)" }


  # 8. The editor: open an image, draw an arrow and an ellipse with posted mouse input, Enter exports.
  Add-Type -AssemblyName System.Drawing
  $png = "$work\e2e-input.png"
  $bmp = New-Object System.Drawing.Bitmap 800, 500
  $g = [System.Drawing.Graphics]::FromImage($bmp); $g.Clear([System.Drawing.Color]::White); $g.Dispose()
  $bmp.Save($png, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
  $ed = Start-Process -FilePath "$Bin\glint-editor.exe" -ArgumentList @('--file', $png, '--name', 'e2e.png') -PassThru
  $okWin = Wait-Until { (EdWin $ed) -ne [IntPtr]::Zero } 10000
  Check 'editor window opens' $okWin
  if ($okWin) { $t = Title (EdWin $ed); Check 'title shows the image size' ($t -like 'e2e.png (800 * 500) - Glint') $t }
  if ($okWin) {
    Start-Sleep -Milliseconds 800
    $hw = EdWin $ed
    $rc = New-Object W+RECT; [W]::GetClientRect($hw, [ref]$rc) | Out-Null
    $s = [W]::GetDpiForWindow($hw) / 96.0
    $cw = $rc.R; $ch = $rc.B
    $top = 64 * $s; $areaH = $ch - (64 + 84) * $s
    $fit = [math]::Min(1, [math]::Min(($cw - 24 * $s) / 800, ($areaH - 24 * $s) / 500))
    $x0 = ($cw - 800 * $fit) / 2; $y0 = $top + ($areaH - 500 * $fit) / 2
    function ImgPt([double] $x, [double] $y) { LParam ([int]($x0 + $x * $fit)) ([int]($y0 + $y * $fit)) }
    function Key([int] $vk) {
      $l = [IntPtr](1 -bor ([W]::MapVirtualKey($vk, 0) -shl 16))
      [W]::PostMessage($hw, 0x0100, [IntPtr]$vk, $l) | Out-Null
      [W]::PostMessage($hw, 0x0101, [IntPtr]$vk, [IntPtr]([int64]$l -bor 0xC0000000)) | Out-Null
      Start-Sleep -Milliseconds 150
    }
    function DragImg([double] $ax, [double] $ay, [double] $bx, [double] $by) {
      [W]::PostMessage($hw, 0x0200, [IntPtr]0, (ImgPt $ax $ay)) | Out-Null
      [W]::PostMessage($hw, 0x0201, [IntPtr]1, (ImgPt $ax $ay)) | Out-Null
      foreach ($i in 1..8) { [W]::PostMessage($hw, 0x0200, [IntPtr]1, (ImgPt ($ax + ($bx - $ax) * $i / 8) ($ay + ($by - $ay) * $i / 8))) | Out-Null; Start-Sleep -Milliseconds 15 }
      [W]::PostMessage($hw, 0x0202, [IntPtr]0, (ImgPt $bx $by)) | Out-Null
      Start-Sleep -Milliseconds 200
    }
    # The toolbar tooltip (F11 test hook: a hidden desktop has no cursor to hover with).
    Key 0x7A
    Start-Sleep -Milliseconds 300
    Key 0x7B                       # F12 = window snapshot
    if (Wait-Until { Test-Path "$work\out\editor-window.png" }) {
      Start-Sleep -Milliseconds 200
      Move-Item -Force "$work\out\editor-window.png" "$work\out\editor-tooltip.png"
      New-Item -ItemType Directory -Force "$PSScriptRoot\..\..\.e2e" | Out-Null
      Copy-Item -Force "$work\out\editor-tooltip.png" "$PSScriptRoot\..\..\.e2e\editor-tooltip.png"
      $tb = New-Object System.Drawing.Bitmap "$work\out\editor-tooltip.png"
      $px = $tb.GetPixel([int](200 * $s), [int](230 * $s)); $tb.Dispose()   # tooltip below (200,200)
      Check 'toolbar tooltip draws' ($px.R -ge 0x24 -and $px.R -le 0x34 -and $px.B -ge 0x2a -and $px.B -le 0x3a) "($($px.R),$($px.G),$($px.B))"
    } else { Check 'tooltip snapshot written' $false }
    # The Copy button's checkmark (F10 test hook), kept for a look.
    Key 0x79
    Start-Sleep -Milliseconds 450
    Key 0x7B
    if (Wait-Until { Test-Path "$work\out\editor-window.png" }) {
      Start-Sleep -Milliseconds 200
      Move-Item -Force "$work\out\editor-window.png" "$PSScriptRoot\..\..\.e2e\editor-copied.png"
    }
    Start-Sleep -Milliseconds 1300
    DragImg 100 100 400 300        # arrow (default tool)
    Key 0x4F                       # O = ellipse
    DragImg 500 100 700 300
    Key 0x54                       # T = text
    [W]::PostMessage($hw, 0x0200, [IntPtr]0, (ImgPt 150 430)) | Out-Null
    [W]::PostMessage($hw, 0x0201, [IntPtr]1, (ImgPt 150 430)) | Out-Null
    [W]::PostMessage($hw, 0x0202, [IntPtr]0, (ImgPt 150 430)) | Out-Null
    Start-Sleep -Milliseconds 300
    foreach ($vk in 0x48, 0x49, 0x20, 0x54, 0x48, 0x45, 0x52, 0x45) { Key $vk }   # "hi there"
    Start-Sleep -Milliseconds 200
    Key 0x7B                       # F12 = window snapshot (test hook), taken while typing
    Key 0x0D                       # Enter finishes the text
    Check 'editor snapshot written' (Wait-Until { Test-Path "$work\out\editor-window.png" })
    Key 0x0D                       # Enter = copy (test hook: export file) and close
    Check 'editor export written' (Wait-Until { Test-Path "$work\out\editor-export.png" })
    Check 'editor closes after Enter' ($ed.WaitForExit(5000))
    if (Test-Path "$work\out\editor-export.png") {
      $out = New-Object System.Drawing.Bitmap "$work\out\editor-export.png"
      Check 'export keeps the size' ("$($out.Width)x$($out.Height)" -eq '800x500') "$($out.Width)x$($out.Height)"
      $mid = $out.GetPixel(250, 200)
      Check 'arrow drawn in red' ($mid.R -gt 200 -and $mid.G -lt 100) "($($mid.R),$($mid.G),$($mid.B))"
      $ring = $out.GetPixel(600, 100)
      Check 'ellipse drawn' ($ring.R -gt 200 -and $ring.G -lt 100) "($($ring.R),$($ring.G),$($ring.B))"
      $red = 0
      for ($x = 140; $x -lt 320; $x += 2) { for ($y = 395; $y -lt 460; $y += 2) { $p = $out.GetPixel($x, $y); if ($p.R -gt 200 -and $p.G -lt 100) { $red++ } } }
      Check 'text drawn' ($red -gt 40) "($red red samples)"
      $blank = $out.GetPixel(600, 200)
      Check 'ellipse is not filled' ($blank.R -eq 255 -and $blank.G -eq 255) "($($blank.R),$($blank.G),$($blank.B))"
      $out.Dispose()
    }
    if ($env:GLINT_E2E_KEEP) { Copy-Item "$work\out\editor-window.png" $env:GLINT_E2E_KEEP -ErrorAction SilentlyContinue }
  }
  if (-not $ed.HasExited) { $ed.Kill() }

  # 8b. A picture bigger than the screen: the window must still fit on the screen (it zooms out).
  $bigPng = "$work\e2e-big.png"
  $big = New-Object System.Drawing.Bitmap 5000, 3200
  $g = [System.Drawing.Graphics]::FromImage($big); $g.Clear([System.Drawing.Color]::FromArgb(30, 60, 90)); $g.Dispose()
  $big.Save($bigPng, [System.Drawing.Imaging.ImageFormat]::Png); $big.Dispose()
  $ed = Start-Process -FilePath "$Bin\glint-editor.exe" -ArgumentList @('--file', $bigPng, '--name', 'big.png') -PassThru
  $okWin = Wait-Until { (EdWin $ed) -ne [IntPtr]::Zero } 10000
  Check 'big image: editor opens' $okWin
  if ($okWin) {
    Start-Sleep -Milliseconds 1200
    $hw = EdWin $ed
    $wr = New-Object W+RECT; [W]::GetWindowRect($hw, [ref]$wr) | Out-Null
    $sw = [W]::GetSystemMetrics(0); $sh = [W]::GetSystemMetrics(1)
    Check 'big image: window fits on the screen' (($wr.R - $wr.L) -le $sw -and ($wr.B - $wr.T) -le $sh) "window $($wr.R - $wr.L)x$($wr.B - $wr.T), screen ${sw}x${sh}"
    Key 0x43                       # C = crop: the frame must show on all four sides
    Start-Sleep -Milliseconds 300
    Key 0x7B
    if (Wait-Until { Test-Path "$work\out\editor-window.png" }) {
      Start-Sleep -Milliseconds 200
      Copy-Item -Force "$work\out\editor-window.png" "$PSScriptRoot\..\..\.e2e\editor-big-crop.png"
      # The frame's bottom line must be on screen: a white row just above the image's bottom edge.
      $sn = New-Object System.Drawing.Bitmap "$PSScriptRoot\..\..\.e2e\editor-big-crop.png"
      $x = [int]($sn.Width / 4); $last = -1
      for ($y = 0; $y -lt $sn.Height; $y++) { $c = $sn.GetPixel($x, $y); if ($c.R -eq 30 -and $c.G -eq 60 -and $c.B -eq 90) { $last = $y } }
      $white = $false
      for ($y = $last + 1; $y -le [math]::Min($sn.Height - 1, $last + 8); $y++) { if ($sn.GetPixel($x, $y).R -gt 200) { $white = $true } }
      $sn.Dispose()
      Check 'big image: crop frame bottom line on screen' $white "image ends at row $last"
      Remove-Item "$work\out\editor-window.png"
    }
  }
  if (-not $ed.HasExited) { $ed.Kill() }

  # 9. Crop, iOS style: the frame starts around the image; drag the bottom-right corner in, let go,
  #    and it crops by itself after a pause (no Enter).
  Remove-Item "$work\out\editor-export.png", "$work\out\editor-window.png" -ErrorAction SilentlyContinue
  $ed = Start-Process -FilePath "$Bin\glint-editor.exe" -ArgumentList @('--file', $png, '--name', 'crop.png') -PassThru
  $okWin = Wait-Until { (EdWin $ed) -ne [IntPtr]::Zero } 10000
  Check 'crop: editor opens' $okWin
  if ($okWin) {
    Start-Sleep -Milliseconds 800
    $hw = EdWin $ed
    Key 0x43                       # C = crop
    [W]::PostMessage($hw, 0x0200, [IntPtr]0, (ImgPt 790 490)) | Out-Null
    [W]::PostMessage($hw, 0x0201, [IntPtr]1, (ImgPt 798 498)) | Out-Null
    foreach ($i in 1..8) { [W]::PostMessage($hw, 0x0200, [IntPtr]1, (ImgPt (798 - 25 * $i) (498 - 12.5 * $i))) | Out-Null; Start-Sleep -Milliseconds 15 }
    Start-Sleep -Milliseconds 100
    Key 0x7B                       # F12 snapshot while dragging: frame, handles, thirds grid
    [W]::PostMessage($hw, 0x0202, [IntPtr]0, (ImgPt 598 398)) | Out-Null
    if (Wait-Until { Test-Path "$work\out\editor-window.png" }) {
      Start-Sleep -Milliseconds 200
      Copy-Item -Force "$work\out\editor-window.png" "$PSScriptRoot\..\..\.e2e\editor-crop.png"
      Remove-Item "$work\out\editor-window.png"
    }
    Start-Sleep -Milliseconds 400
    Key 0x7B
    $early = $false
    if (Wait-Until { Test-Path "$work\out\editor-window.png" }) {
      Start-Sleep -Milliseconds 200
      $b = New-Object System.Drawing.Bitmap "$work\out\editor-window.png"; $early = $b.Width; $b.Dispose()
      Remove-Item "$work\out\editor-window.png"
    }
    Start-Sleep -Milliseconds 1200  # the pause (1 s) and the zoom
    Key 0x7B
    if (Wait-Until { Test-Path "$work\out\editor-window.png" }) {
      Start-Sleep -Milliseconds 200
      Copy-Item -Force "$work\out\editor-window.png" "$PSScriptRoot\..\..\.e2e\editor-cropped.png"
    }
    Key 0x0D                       # Enter = copy (test hook: export file) and close
    Check 'crop: export written' (Wait-Until { Test-Path "$work\out\editor-export.png" })
    if (Test-Path "$work\out\editor-export.png") {
      $out = New-Object System.Drawing.Bitmap "$work\out\editor-export.png"
      Check 'crop: applied after the pause, without Enter' ("$($out.Width)x$($out.Height)" -eq '598x398') "$($out.Width)x$($out.Height)"
      $out.Dispose()
    }
  }
  if (-not $ed.HasExited) { $ed.Kill() }

  # 10. Settings window: opens, saves a change to config.toml at once, Esc closes it.
  $st = Start-Process -FilePath "$Bin\glint-editor.exe" -ArgumentList @('--settings') -PassThru
  $okSet = Wait-Until { [W]::FindWindow([NullString]::Value, 'Glint settings') -ne [IntPtr]::Zero } 10000
  Check 'settings window opens' $okSet
  if ($okSet) {
    Start-Sleep -Milliseconds 700
    $hw = [W]::FindWindow([NullString]::Value, 'Glint settings')
    Key 0x7B                       # F12 = snapshot
    if (Wait-Until { Test-Path "$work\out\settings-window.png" }) {
      Start-Sleep -Milliseconds 200
      Copy-Item -Force "$work\out\settings-window.png" "$PSScriptRoot\..\..\.e2e\settings-window.png"
    }
    Key 0x78                       # F9 (test hook) = flip the capture sound
    Check 'settings saves at once' (Wait-Until { (Get-Content $env:GLINT_CONFIG -Raw) -match 'capture_sound = false' })
    Key 0x1B                       # Esc closes
    Check 'Esc closes settings' ($st.WaitForExit(5000))
  }
  if (-not $st.HasExited) { $st.Kill() }

  Glint @('quit')
  Check 'quit stops the daemon' ($daemon.WaitForExit(5000))
}
finally {
  if (-not $daemon.HasExited) { $daemon.Kill() }
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
if ($script:fails -gt 0) { Write-Output "$($script:fails) check(s) failed"; exit 1 }
Write-Output 'all checks passed'
exit 0
