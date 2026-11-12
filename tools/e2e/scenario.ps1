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

  # 3. Region overlay: drag from (100,100) to (400,300) in client pixels, then Enter.
  Glint @('region')
  Check 'overlay opens' (Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero })
  $ov = [W]::FindWindow('GlintOverlay', [NullString]::Value)
  [W]::PostMessage($ov, 0x0200, [IntPtr]0, (LParam 100 100)) | Out-Null   # WM_MOUSEMOVE
  [W]::PostMessage($ov, 0x0201, [IntPtr]1, (LParam 100 100)) | Out-Null   # WM_LBUTTONDOWN
  foreach ($i in 1..10) { [W]::PostMessage($ov, 0x0200, [IntPtr]1, (LParam (100 + 30 * $i) (100 + 20 * $i))) | Out-Null }
  [W]::PostMessage($ov, 0x0202, [IntPtr]0, (LParam 400 300)) | Out-Null   # WM_LBUTTONUP
  Start-Sleep -Milliseconds 100
  Check 'overlay waits for Enter' ([W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero)
  [W]::PostMessage($ov, 0x0100, [IntPtr]0x0D, [IntPtr]0) | Out-Null        # WM_KEYDOWN Enter
  Check 'overlay closes on Enter' (Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -eq [IntPtr]::Zero })
  Check 'region capture written' (Wait-Until { Test-Path (Out-File-N 3) })
  if (Test-Path (Out-File-N 3)) { Check 'region capture size' ((Png-Size (Out-File-N 3)) -eq '301x201') (Png-Size (Out-File-N 3)) }

  # 4. Arrow keys adjust the selection: Right moves, Shift is not held so size stays.
  Glint @('region')
  Wait-Until { [W]::FindWindow('GlintOverlay', [NullString]::Value) -ne [IntPtr]::Zero } | Out-Null
  $ov = [W]::FindWindow('GlintOverlay', [NullString]::Value)
  [W]::PostMessage($ov, 0x0201, [IntPtr]1, (LParam 50 50)) | Out-Null
  [W]::PostMessage($ov, 0x0200, [IntPtr]1, (LParam 149 99)) | Out-Null
  [W]::PostMessage($ov, 0x0202, [IntPtr]0, (LParam 149 99)) | Out-Null
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
  $okWin = Wait-Until { [W]::FindWindow([NullString]::Value, 'e2e.png - Glint') -ne [IntPtr]::Zero } 10000
  Check 'editor window opens' $okWin
  if ($okWin) {
    Start-Sleep -Milliseconds 800
    $hw = [W]::FindWindow([NullString]::Value, 'e2e.png - Glint')
    $rc = New-Object W+RECT; [W]::GetClientRect($hw, [ref]$rc) | Out-Null
    $s = [W]::GetDpiForWindow($hw) / 96.0
    $cw = $rc.R; $ch = $rc.B
    $top = 50 * $s; $areaH = $ch - $top - 26 * $s
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
