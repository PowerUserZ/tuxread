# Trial only: what a GUI process sees on a CI runner.
param([string]$Exe)
"session: $([System.Diagnostics.Process]::GetCurrentProcess().SessionId)"
"interactive: $([Environment]::UserInteractive)"
"user: $env:USERNAME"
$wv = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
"webview2 (machine): $($wv.pv)"
foreach ($k in 'HKLM:\SOFTWARE\Policies\Microsoft\Edge', 'HKCU:\SOFTWARE\Policies\Microsoft\Edge') {
  if (Test-Path $k) { "policy key $k"; Get-ChildItem $k -Recurse | ForEach-Object { "  $($_.Name)"; $_.Property | ForEach-Object { "    $_" } }; (Get-ItemProperty $k).PSObject.Properties | Where-Object Name -notlike 'PS*' | ForEach-Object { "  $($_.Name) = $($_.Value)" } }
}
netsh int ipv4 show excludedportrange protocol=tcp
$prof = Join-Path $env:RUNNER_TEMP "probe-profile"
New-Item -ItemType Directory $prof -Force | Out-Null
$env:WEBVIEW2_USER_DATA_FOLDER = $prof
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=0'
$p = Start-Process $Exe -PassThru
Start-Sleep 20
$p.Refresh()
"app exited: $($p.HasExited) title: '$($p.MainWindowTitle)'"
Get-ChildItem $prof -Recurse -Depth 1 | ForEach-Object { "profile: $($_.FullName)" }
$portFile = Join-Path $prof 'EBWebView\DevToolsActivePort'
if (Test-Path $portFile) {
  $port = (Get-Content $portFile)[0]
  "devtools port: $port"
  try { (Invoke-WebRequest "http://127.0.0.1:$port/json" -UseBasicParsing -TimeoutSec 5).Content } catch { "devtools: $($_.Exception.Message)" }
} else { "no DevToolsActivePort" }
Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Select-Object -First 1 | ForEach-Object { "webview cmdline: $($_.CommandLine)" }
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
