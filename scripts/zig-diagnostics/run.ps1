Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/windows-controls.ps1"
. "$PSScriptRoot/receipts.ps1"
$proofPath = Join-Path (Get-EvidenceRoot) 'controls.json'
if (-not (Test-Path -LiteralPath $proofPath)) { Invoke-WindowsControls }
$proof = Get-Content -LiteralPath $proofPath -Raw | ConvertFrom-Json
Assert-SamplerControlProof $proof | Out-Null
$directory = Join-Path (Get-EvidenceRoot) 'qualification'
$capture = Invoke-ProcessCapture $directory 'Harness'
if ($capture.ExitCode -ne 0) { exit $capture.ExitCode }
if ($capture.OrphansStopped -ne 0) { throw 'The diagnostic left child processes behind.' }
$receipts = Assert-DiagnosticReceipts (Read-DiagnosticReceipts (Join-Path $directory 'stdout.log'))
$zigProcesses = @($capture.Processes | Where-Object {
    $_.Name -eq 'zig.exe' -and $_.Samples -ge 2 -and $_.ObservedCpuMilliseconds -gt 0
})
if ($zigProcesses.Count -eq 0) { throw 'No real Zig compiler process had repeated CPU/RSS samples.' }
$receipts | ConvertTo-Json -Depth 10 | Set-Content (Join-Path $directory 'receipts.json')
Write-Host "Qualified receipts: 8/8; measured Zig process identities: $($zigProcesses.Count)."
