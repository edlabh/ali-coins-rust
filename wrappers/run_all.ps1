# Execução unificada (check-in + tarefas) — wrapper Windows (PowerShell).
#
# Códigos: 0 sucesso · 1 falha · 2 sem ação · 3 lock ativo · 4 streak · 5 2FA
# Retenta uma vez (--no-delay) quando o exit é 1, como o wrapper .sh/oráculo.
#
# Uso:
#   powershell -ExecutionPolicy Bypass -File wrappers\run_all.ps1 --json
param(
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]] $PassThru
)

$ErrorActionPreference = "Continue"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

$Exe = Join-Path $Root "target\release\ali-coins.exe"
if (-not (Test-Path $Exe)) { $Exe = "ali-coins" }

& $Exe all @PassThru
$Code = $LASTEXITCODE

if ($Code -eq 1) {
  Write-Warning "[run_all] execução falhou (exit 1); retentando uma vez com --no-delay..."
  Start-Sleep -Seconds 10
  & $Exe all --no-delay @PassThru
  $Code = $LASTEXITCODE
}

$Stamp = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
Write-Output "[run_all] exit=$Code fim: $Stamp"
exit $Code
