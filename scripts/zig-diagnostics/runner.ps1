param(
    [Parameter(Mandatory)][string] $Directory,
    [ValidateSet('Harness', 'Busy', 'Idle', 'Fail')][string] $Mode = 'Harness'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (-not (Test-Path -LiteralPath (Join-Path $Directory 'observer-ready'))) {
    if ([DateTime]::UtcNow -ge $deadline) { throw 'The process observer never released its startup barrier.' }
    Start-Sleep -Milliseconds 50
}
if ($Mode -eq 'Harness') {
    & task test:run -- --lib snippets::validators::zig::diagnostic_tests::qualify_cold_and_warm_real_consumer_cases -- --ignored --exact --nocapture --test-threads=1
    exit $LASTEXITCODE
}
$info = [System.Diagnostics.ProcessStartInfo]::new()
$info.FileName = (Get-Process -Id $PID).Path
$info.UseShellExecute = $false
foreach ($argument in @('-NoProfile', '-File', "$PSScriptRoot/control-worker.ps1",
    '-Directory', $Directory, '-Mode', $Mode)) { $info.ArgumentList.Add($argument) }
$child = [System.Diagnostics.Process]::Start($info)
try {
    $child.WaitForExit()
    exit $child.ExitCode
} finally { $child.Dispose() }
