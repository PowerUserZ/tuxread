# Trial only: what a GUI process sees on a CI runner.
param([string]$Exe)
"session: $([System.Diagnostics.Process]::GetCurrentProcess().SessionId)"
"interactive: $([Environment]::UserInteractive)"
"user: $env:USERNAME"
$wv = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}' -ErrorAction SilentlyContinue
"webview2 (machine): $($wv.pv)"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9333'
$p = Start-Process $Exe -PassThru
Start-Sleep 20
$p.Refresh()
"app exited: $($p.HasExited) title: '$($p.MainWindowTitle)'"
Get-Process TuxRead, msedgewebview2 -ErrorAction SilentlyContinue | ForEach-Object { "process: $($_.ProcessName) $($_.Id) session=$($_.SessionId) title='$($_.MainWindowTitle)'" }
try { (Invoke-WebRequest http://127.0.0.1:9333/json -UseBasicParsing -TimeoutSec 5).Content } catch { "devtools: $($_.Exception.Message)" }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$c = New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $p.Id)
foreach ($w in $A::RootElement.FindAll('Children', $c)) { "window: '$($w.Current.Name)' class=$($w.Current.ClassName)"; foreach ($e in $w.FindAll('Descendants', [System.Windows.Automation.Condition]::TrueCondition)) { if ($e.Current.Name) { "   $($e.Current.Name)" } } }
Get-ChildItem "$env:LOCALAPPDATA\io.github.poweruserz.tuxread\logs" -ErrorAction SilentlyContinue | ForEach-Object { Get-Content $_.FullName -Tail 5 }
if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force }
