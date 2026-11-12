# Runs the Glint end-to-end scenario on a hidden Windows desktop, so no window ever takes the
# owner's screen or focus (pattern from Prism's tools/e2e/hidden.mjs). Under CI, or with -Same,
# it runs on the current desktop instead.
#
#   pwsh tools/e2e/run.ps1            # debug build
#   pwsh tools/e2e/run.ps1 -Release   # release build
param([switch] $Release, [switch] $Same)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "$root\target" }
$bin = if ($Release) { "$targetDir\release" } else { "$targetDir\debug" }
if (-not (Test-Path "$bin\glint.exe")) { throw "build first: $bin\glint.exe is missing" }
$scenario = "$PSScriptRoot\scenario.ps1"

if ($Same -or $env:CI) {
  & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $scenario -Bin $bin
  exit $LASTEXITCODE
}

New-Item -ItemType Directory -Force "$root\.e2e" | Out-Null
$log = "$root\.e2e\run.log"
Set-Content $log ''
$inner = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$scenario`" -Bin `"$bin`""
$cmd = "cmd.exe /d /c `"$inner > `"$log`" 2>&1`""
Add-Type -Path "$PSScriptRoot\HiddenDesktop.cs"
$code = [HiddenDesktop]::Run("glint-e2e-$PID", $cmd, $root)
Get-Content $log
exit $code
