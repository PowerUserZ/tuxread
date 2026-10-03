# Spec §9: without the WebView2 runtime, TuxRead shows a message box with the download address
# and exits. WEBVIEW2_BROWSER_EXECUTABLE_FOLDER pointing at an empty folder makes the runtime
# missing for one process. Usage: scripts/check-no-webview2.ps1 <path to TuxRead.exe>
param([Parameter(Mandatory = $true)][string]$Exe)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]

$empty = Join-Path ([System.IO.Path]::GetTempPath()) "tuxread-no-webview2-$PID"
New-Item -ItemType Directory -Force $empty | Out-Null
$env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $empty
$app = Start-Process (Resolve-Path $Exe) -PassThru
Remove-Item Env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER
try {
    $byProcess = New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $app.Id)
    $box = $null
    for ($i = 0; $i -lt 100 -and -not $box; $i++) {
        Start-Sleep -Milliseconds 100
        # A message box is a dialog window (class #32770); Tauri's own "Error" dialog does not count.
        $box = $A::RootElement.FindAll('Children', $byProcess) |
            Where-Object { $_.Current.ClassName -eq '#32770' -and $_.Current.Name -eq 'TuxRead' } |
            Select-Object -First 1
    }
    if (-not $box) { throw 'no message box titled "TuxRead" appeared' }
    $parts = $box.FindAll('Descendants', [System.Windows.Automation.Condition]::TrueCondition)
    $text = ($parts | ForEach-Object { $_.Current.Name }) -join "`n"
    if ($text -notmatch 'https://developer\.microsoft\.com/microsoft-edge/webview2') { throw "the message has no download address:`n$text" }
    if ($text -notmatch 'WebView2 Runtime') { throw "the message does not name the runtime:`n$text" }
    # The box has only OK, so closing it is the same as pressing OK.
    $box.GetCurrentPattern([System.Windows.Automation.WindowPattern]::Pattern).Close()
    if (-not $app.WaitForExit(10000)) { throw 'TuxRead kept running after the message box was closed' }
    if ($app.ExitCode -ne 1) { throw "TuxRead exited with $($app.ExitCode), not 1" }
    'ok: the message box names WebView2 and its download address, and TuxRead exits with 1'
} finally {
    if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
    Remove-Item -Recurse -Force $empty -ErrorAction SilentlyContinue
}
