# Check-in diário (`ali-coins checkin`) — wrapper Windows (PowerShell).
param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]] $PassThru
)
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root
$Exe = Join-Path $Root "target\release\ali-coins.exe"
if (-not (Test-Path $Exe)) { $Exe = "ali-coins" }
& $Exe checkin @PassThru
exit $LASTEXITCODE
