# Runs only in the disposable GitHub Windows runner.
param([Parameter(Mandatory = $true)][string]$Installer)
$ErrorActionPreference = 'Stop'
$product = 'Drifting Next Alpha'
$dir = Join-Path $env:LOCALAPPDATA $product
$exe = Join-Path $dir 'drifting.exe'
$installed = Start-Process $Installer -ArgumentList '/S' -Wait -PassThru
if ($installed.ExitCode -ne 0) { throw "NSIS install failed ($($installed.ExitCode))" }
$app = $null
try {
  foreach ($file in @('drifting.exe', 'uninstall.exe', 'fonts\DriftingSans-Regular.ttf', 'fonts\DriftingSerif-Regular.ttf', 'fonts\SarasaMonoSC-Regular.ttf')) {
    if (-not (Test-Path (Join-Path $dir $file))) { throw "Installed file missing: $file" }
  }
  $key = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$product"
  if ((Get-ItemProperty $key).DisplayName -ne $product) { throw 'Incorrect Alpha installation identity' }
  $app = Start-Process $exe -PassThru
  $visible = $false
  foreach ($i in 1..30) {
    Start-Sleep -Seconds 1
    $app.Refresh()
    if ($app.HasExited) { throw "Alpha app exited during startup ($($app.ExitCode))" }
    if ($app.MainWindowHandle -ne 0) { $visible = $true; break }
  }
  if (-not $visible) { throw 'No Alpha application window appeared' }
  if (-not $app.CloseMainWindow()) { throw 'Application did not accept closing its window' }
  if (-not $app.WaitForExit(20000)) { throw 'Application did not save and close within 20 seconds' }
} finally {
  if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force }
  $uninstaller = Join-Path $dir 'uninstall.exe'
  if (Test-Path $uninstaller) {
    $remove = Start-Process $uninstaller -ArgumentList @('/S', "_?=$dir") -Wait -PassThru
    if ($remove.ExitCode -ne 0) { throw "NSIS uninstall failed ($($remove.ExitCode))" }
  }
}
if (Test-Path $exe) { throw 'Uninstall retained the application executable' }
