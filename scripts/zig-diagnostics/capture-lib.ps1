Set-StrictMode -Version Latest
. "$PSScriptRoot/process-tree.ps1"

function Start-DiagnosticRunner([string] $Directory, [string] $Mode) {
    $info = [System.Diagnostics.ProcessStartInfo]::new()
    $info.FileName = (Get-Process -Id $PID).Path
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($argument in @('-NoProfile', '-File', "$PSScriptRoot/runner.ps1",
        '-Directory', $Directory, '-Mode', $Mode)) { $info.ArgumentList.Add($argument) }
    $process = [System.Diagnostics.Process]::Start($info)
    [pscustomobject]@{
        Process = $process; Output = $process.StandardOutput.ReadToEndAsync()
        Errors = $process.StandardError.ReadToEndAsync()
    }
}

function Initialize-DiagnosticTree($Runner, [string] $Directory) {
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        if ($Runner.Process.HasExited) { throw 'The diagnostic runner exited before observation.' }
        $snapshot = Get-WindowsSnapshot
        $root = @($snapshot | Where-Object { $_.ProcessId -eq $Runner.Process.Id })
        if ($root.Count -eq 1) {
            $state = New-ProcessTree $root[0] ([DateTime]::UtcNow.Ticks)
            [IO.File]::WriteAllText((Join-Path $Directory 'observer-ready'), 'ready')
            return $state
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'CIM never observed the waiting diagnostic runner.'
}

function Get-OwnedSnapshot([hashtable] $State, [object[]] $Snapshot) {
    @($Snapshot | Where-Object { $State.ContainsKey((Get-ProcessIdentity $_)) })
}

function Write-ProcessSample($Writer, [object[]] $Owned, [long] $ObservedTicks) {
    $Writer.WriteLine(([pscustomobject]@{
        observed_ticks = $ObservedTicks; processes = $Owned
    } | ConvertTo-Json -Depth 6 -Compress))
}

function Stop-OwnedProcesses([hashtable] $State) {
    $stopped = 0
    foreach ($row in (Get-OwnedSnapshot $State (Get-WindowsSnapshot))) {
        try { $process = [System.Diagnostics.Process]::GetProcessById($row.ProcessId) }
        catch [ArgumentException] { continue }
        try {
            # ~keep CIM truncates creation times to microseconds. Compare the opened handle's
            # ~keep start time before stopping it, so PID reuse cannot redirect cleanup.
            $difference = [Math]::Abs($process.StartTime.ToUniversalTime().Ticks - $row.CreatedTicks)
            if (-not $process.HasExited -and $difference -lt 10) {
                $process.Kill($true)
                $stopped += 1
            }
        } finally { $process.Dispose() }
    }
    $stopped
}

function Watch-DiagnosticTree($Runner, [hashtable] $State, $Writer) {
    $samples = 0
    $peakRss = 0L
    $exitObserved = $null
    $heartbeat = [DateTime]::UtcNow
    while ($true) {
        $snapshot = Get-WindowsSnapshot
        $now = [DateTime]::UtcNow
        Update-ProcessTree $State $snapshot $now.Ticks
        $owned = @(Get-OwnedSnapshot $State $snapshot)
        Write-ProcessSample $Writer $owned $now.Ticks
        $samples += 1
        $rss = Get-SnapshotRss $owned
        $peakRss = [Math]::Max($peakRss, $rss)
        if ($Runner.Process.HasExited) {
            if ($null -eq $exitObserved) { $exitObserved = $now }
            $children = @($owned | Where-Object { $_.ProcessId -ne $Runner.Process.Id })
            if ($children.Count -eq 0 -or ($now - $exitObserved).TotalSeconds -ge 5) { break }
        }
        if (($now - $heartbeat).TotalSeconds -ge 30) {
            Write-Host "Diagnostic sampler: $samples snapshots, $($State.Count) process identities."
            $heartbeat = $now
        }
        Start-Sleep -Milliseconds 200
    }
    [pscustomobject]@{ Samples = $samples; PeakSampledTreeRssBytes = $peakRss }
}

function Invoke-ProcessCapture([string] $Directory, [string] $Mode) {
    if (-not $IsWindows) { throw 'Live process capture requires Windows CIM.' }
    if (Test-Path -LiteralPath $Directory) { throw "Evidence directory already exists: $Directory" }
    [IO.Directory]::CreateDirectory($Directory) | Out-Null
    $runner = Start-DiagnosticRunner $Directory $Mode
    $writer = [IO.StreamWriter]::new((Join-Path $Directory 'samples.jsonl'), $false)
    $writer.AutoFlush = $true
    $state = @{}
    try {
        $state = Initialize-DiagnosticTree $runner $Directory
        $measurement = Watch-DiagnosticTree $runner $state $writer
        $runner.Process.WaitForExit()
        $stopped = Stop-OwnedProcesses $state
        $output = $runner.Output.GetAwaiter().GetResult()
        $errors = $runner.Errors.GetAwaiter().GetResult()
        [IO.File]::WriteAllText((Join-Path $Directory 'stdout.log'), $output)
        [IO.File]::WriteAllText((Join-Path $Directory 'stderr.log'), $errors)
        Write-Host $output
        Write-Host $errors
        $summary = [pscustomobject]@{
            ExitCode = $runner.Process.ExitCode; Samples = $measurement.Samples
            PeakSampledTreeRssBytes = $measurement.PeakSampledTreeRssBytes
            Processes = @($state.Values | Sort-Object CreatedTicks, ProcessId)
            OrphansStopped = $stopped
            Limitations = 'Polling reports observed CPU deltas and sampled RSS/I/O. Processes ending between snapshots may be absent; CPU is a lower bound, not total command CPU.'
        }
        $summary | ConvertTo-Json -Depth 8 | Set-Content (Join-Path $Directory 'summary.json')
        $summary
    } finally {
        $writer.Dispose()
        if (-not $runner.Process.HasExited) { $runner.Process.Kill($true) }
        if ($state.Count -gt 0) { Stop-OwnedProcesses $state | Out-Null }
        $runner.Process.Dispose()
    }
}
