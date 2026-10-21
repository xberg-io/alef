Set-StrictMode -Version Latest
. "$PSScriptRoot/capture-lib.ps1"

function Get-EvidenceRoot {
    if ($env:ALEF_ZIG_DIAGNOSTIC_DIR) { return [IO.Path]::GetFullPath($env:ALEF_ZIG_DIAGNOSTIC_DIR) }
    Join-Path ([IO.Path]::GetTempPath()) "alef-zig-diagnostics-$PID"
}

function Get-SamplerSourceHashes {
    $sources = @{}
    foreach ($source in (Get-ChildItem -LiteralPath $PSScriptRoot -Filter '*.ps1')) {
        $sources[$source.Name] = (Get-FileHash -LiteralPath $source.FullName -Algorithm SHA256).Hash
    }
    if ($sources.Count -ne 8) { throw 'Expected exactly eight sampler sources.' }
    $sources
}

function Assert-SamplerControlProof($Proof) {
    if ($Proof.expected -ne 3 -or $Proof.observed -ne 3) { throw 'Live controls did not execute 3/3 cases.' }
    $current = Get-SamplerSourceHashes
    if (@($Proof.sources.PSObject.Properties).Count -ne $current.Count) { throw 'Sampler source count changed.' }
    foreach ($name in $current.Keys) {
        if ($Proof.sources.$name -ne $current[$name]) { throw "Stale sampler control proof: $name" }
    }
    foreach ($mode in @('Busy', 'Idle', 'Fail')) {
        $record = $Proof.controls.$mode
        $expectedExit = if ($mode -eq 'Fail') { 7 } else { 0 }
        if ($Proof.exit_codes.$mode -ne $expectedExit -or $record.Samples -lt 2 -or
            $record.PeakSampledRssBytes -le 0 -or $null -eq $record.ExitObservedTicks -or
            -not $record.ParentIdentity) { throw "Unqualified live control: $mode" }
    }
    if ($Proof.controls.Busy.ObservedCpuMilliseconds -lt 100 -or
        $Proof.controls.Busy.ObservedCpuMilliseconds -le $Proof.controls.Idle.ObservedCpuMilliseconds) {
        throw 'The live controls did not distinguish busy CPU from idle CPU.'
    }
    $true
}

function Invoke-WindowsControls {
    $results = @{}
    $exitCodes = @{}
    $expectedControls = 3
    foreach ($mode in @('Busy', 'Idle', 'Fail')) {
        $directory = Join-Path (Get-EvidenceRoot) "controls/$mode"
        $capture = Invoke-ProcessCapture $directory $mode
        $child = Get-Content (Join-Path $directory 'control-child.json') -Raw | ConvertFrom-Json
        $identity = '{0}@{1}' -f $child.ProcessId, $child.CreatedTicks
        $records = @($capture.Processes | Where-Object { $_.Identity -eq $identity })
        if ($records.Count -ne 1 -or $records[0].Samples -lt 2 -or $records[0].PeakSampledRssBytes -le 0) {
            throw "The sampler did not observe the actual $mode child with repeat samples and RSS."
        }
        $expectedExit = if ($mode -eq 'Fail') { 7 } else { 0 }
        if ($capture.ExitCode -ne $expectedExit -or $capture.OrphansStopped -ne 0) {
            throw "The $mode control lost its exit status or leaked a process."
        }
        if ($null -eq $records[0].ExitObservedTicks) { throw "The $mode child exit was not observed." }
        $results[$mode] = $records[0]
        $exitCodes[$mode] = $capture.ExitCode
    }
    if ($results.Count -ne $expectedControls) { throw 'The sampler did not execute all three live controls.' }
    if ($results.Busy.ObservedCpuMilliseconds -lt 100 -or
        $results.Busy.ObservedCpuMilliseconds -le $results.Idle.ObservedCpuMilliseconds) {
        throw 'The known busy process did not consume measurable CPU beyond the idle control.'
    }
    [pscustomobject]@{
        expected = 3; observed = $results.Count; controls = $results
        exit_codes = $exitCodes; sources = Get-SamplerSourceHashes
    } |
        ConvertTo-Json -Depth 8 | Set-Content (Join-Path (Get-EvidenceRoot) 'controls.json')
    Write-Host 'Windows live sampler controls: 3/3 passed; busy CPU, idle CPU, RSS, ancestry and exit status qualified.'
}
