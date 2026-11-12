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
