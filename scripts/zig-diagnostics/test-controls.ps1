Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/process-tree.ps1"

$script:Checks = 0
function Assert-Control([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw $Message }
    $script:Checks += 1
}

function New-FixtureRow([int] $ProcessId, [int] $ParentId, [long] $CreatedTicks) {
    [pscustomobject]@{
        ProcessId = $ProcessId; ParentId = $ParentId; CreatedTicks = $CreatedTicks
        Name = 'fixture'; CpuTicks = 10000L; WorkingSetBytes = 4096L
        ReadBytes = 0L; WriteBytes = 0L
    }
}

$scripts = @(Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1')
Assert-Control ($scripts.Count -eq 8) 'All eight PowerShell sources must be parsed.'
foreach ($script in $scripts) {
    $tokens = $null
    $errors = $null
    $null = [System.Management.Automation.Language.Parser]::ParseFile($script.FullName, [ref]$tokens, [ref]$errors)
    Assert-Control ($errors.Count -eq 0) "PowerShell parser rejected $($script.Name): $errors"
}

$root = New-FixtureRow 10 1 100
$state = New-ProcessTree $root 110
$snapshot = @($root, (New-FixtureRow 11 10 120), (New-FixtureRow 12 11 130), (New-FixtureRow 99 1 120))
Update-ProcessTree $state $snapshot 140
Assert-Control ($state.Count -eq 3) 'The root, child and grandchild must be observed.'
Assert-Control (-not $state.ContainsKey('99@120')) 'Unrelated processes must be rejected.'
Assert-Control ($state['12@130'].ParentIdentity -eq '11@120') 'Descendants must retain their parent identity.'

$snapshot = @((New-FixtureRow 10 1 200), (New-FixtureRow 20 10 210))
Update-ProcessTree $state $snapshot 220
Assert-Control ($state.Count -eq 3) 'A reused root PID must not adopt an unrelated process tree.'
Assert-Control ($state['10@100'].ExitObservedTicks -eq 220) 'Missing identities must receive an observed exit.'
Assert-Control (-not $state.ContainsKey('10@200')) 'Creation time must distinguish a reused PID.'

$state = New-ProcessTree $root 110
Update-ProcessTree $state @($root, (New-FixtureRow 11 10 120)) 140
Update-ProcessTree $state @($root, (New-FixtureRow 12 11 130)) 160
Assert-Control ($state.ContainsKey('12@130')) 'A child created before its observed parent exit must remain attributable.'
Assert-Control ($state['12@130'].ParentIdentity -eq '11@120') 'An exited parent must preserve its creation identity.'

$state = New-ProcessTree $root 110
Update-ProcessTree $state @($root, (New-FixtureRow 11 10 120)) 140
Update-ProcessTree $state @($root, (New-FixtureRow 11 1 150), (New-FixtureRow 12 11 155)) 160
Assert-Control (-not $state.ContainsKey('12@155')) 'A current unrelated parent with a reused PID must win over stale history.'

$state = New-ProcessTree $root 110
$updated = New-FixtureRow 10 1 100
$updated.CpuTicks = 310000L
$updated.WorkingSetBytes = 8192L
Update-ProcessTree $state @($updated) 150
Assert-Control ($state['10@100'].ObservedCpuMilliseconds -eq 30) 'CPU deltas must use 100ns counter units.'
Assert-Control ($state['10@100'].PeakSampledRssBytes -eq 8192) 'Sampled RSS must retain the peak.'
Assert-Control ($state['10@100'].Samples -eq 2) 'The initial and updated samples must both be counted.'
Assert-Control ((Get-SnapshotRss @()) -eq 0) 'An exited process tree must have zero sampled RSS.'
Assert-Control ((Get-SnapshotRss @($root, $updated)) -eq 12288) 'Sampled tree RSS must sum every observed process.'

. "$PSScriptRoot/receipts.ps1"
$receipts = foreach ($phase in @('cold', 'warm')) {
    foreach ($case in @('include_declared', 'include_missing', 'cwd_rebased', 'cwd_raw')) {
        [pscustomobject]@{
            phase = $phase; case = $case; qualified = $true; budget_seconds = 300
            elapsed_ms = 12; started_ms = 100
            cache_before = [pscustomobject]@{ files = $(if ($phase -eq 'cold') { 0 } else { 1 }); bytes = 5 }
            cache_after = [pscustomobject]@{ files = 1; bytes = 5 }
            expected = $(if ($case -in @('include_declared', 'cwd_rebased')) { 'pass' } else { 'fail' })
            actual = $(if ($case -in @('include_declared', 'cwd_rebased')) { 'pass' } else { 'fail' })
        }
    }
}
Assert-Control ((Assert-DiagnosticReceipts $receipts).Count -eq 8) 'Eight unique receipts must qualify.'
foreach ($mutation in @('empty', 'duplicate', 'unqualified', 'budget', 'warm_cache', 'wrong_status')) {
    $mutated = @($receipts | ConvertTo-Json -Depth 8 | ConvertFrom-Json)
    switch ($mutation) {
        'empty' { $mutated = @() }
        'duplicate' { $mutated[7].case = $mutated[6].case }
        'unqualified' { $mutated[0].qualified = $false }
        'budget' { $mutated[0].budget_seconds = 301 }
        'warm_cache' { $mutated[4].cache_before.files = 0 }
        'wrong_status' { $mutated[1].expected = 'pass'; $mutated[1].actual = 'pass' }
    }
    $rejected = $false
    try { Assert-DiagnosticReceipts $mutated | Out-Null } catch { $rejected = $true }
    Assert-Control $rejected "Receipt mutation '$mutation' must fail qualification."
}

. "$PSScriptRoot/windows-controls.ps1"
$proof = [pscustomobject]@{
    expected = 3; observed = 3; sources = Get-SamplerSourceHashes
    exit_codes = @{ Busy = 0; Idle = 0; Fail = 7 }; controls = @{}
}
foreach ($mode in @('Busy', 'Idle', 'Fail')) {
    $proof.controls[$mode] = [pscustomobject]@{
        Samples = 2; PeakSampledRssBytes = 4096; ExitObservedTicks = 10; ParentIdentity = 'parent@1'
        ObservedCpuMilliseconds = $(if ($mode -eq 'Busy') { 200 } else { 10 })
    }
}
$proof = $proof | ConvertTo-Json -Depth 8 | ConvertFrom-Json
Assert-Control (Assert-SamplerControlProof $proof) 'Current live-control proof must be accepted.'
foreach ($mutation in @('stale_source', 'missing_child', 'wrong_exit', 'no_cpu')) {
    $mutated = $proof | ConvertTo-Json -Depth 8 | ConvertFrom-Json
    switch ($mutation) {
        'stale_source' { $mutated.sources.'process-tree.ps1' = 'stale' }
        'missing_child' { $mutated.controls.PSObject.Properties.Remove('Busy') }
        'wrong_exit' { $mutated.exit_codes.Fail = 0 }
        'no_cpu' { $mutated.controls.Busy.ObservedCpuMilliseconds = 0 }
    }
    $rejected = $false
    try { Assert-SamplerControlProof $mutated | Out-Null } catch { $rejected = $true }
    Assert-Control $rejected "Live-control proof mutation '$mutation' must be rejected."
}
if ($script:Checks -ne 35) { throw "Expected 35 portable controls, ran $script:Checks." }
Write-Host "Portable sampler controls: 35/35 passed."
if ($IsWindows) {
    Invoke-WindowsControls
} else {
    Write-Host 'Windows CIM/process controls require the Windows CI job; they did not run here.'
}
