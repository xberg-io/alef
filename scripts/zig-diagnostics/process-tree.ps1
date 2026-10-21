Set-StrictMode -Version Latest

function Get-ProcessIdentity($Row) {
    '{0}@{1}' -f $Row.ProcessId, $Row.CreatedTicks
}

function New-ProcessRecord($Row, [long] $ObservedTicks, $ParentIdentity) {
    [pscustomobject]@{
        Identity = Get-ProcessIdentity $Row
        ProcessId = $Row.ProcessId; CreatedTicks = $Row.CreatedTicks; Name = $Row.Name
        ParentIdentity = $ParentIdentity; FirstObservedTicks = $ObservedTicks
        LastObservedTicks = $ObservedTicks; ExitObservedTicks = $null; Samples = 1
        InitialCpuTicks = $Row.CpuTicks; ObservedCpuMilliseconds = 0.0
        InitialReadBytes = $Row.ReadBytes; InitialWriteBytes = $Row.WriteBytes
        ObservedReadBytes = 0L; ObservedWriteBytes = 0L
        PeakSampledRssBytes = $Row.WorkingSetBytes
    }
}

function New-ProcessTree($Root, [long] $ObservedTicks) {
    $state = @{}
    $state[(Get-ProcessIdentity $Root)] = New-ProcessRecord $Root $ObservedTicks $null
    $state
}

function Update-ProcessRecord($Record, $Row, [long] $ObservedTicks) {
    $Record.LastObservedTicks = $ObservedTicks
    $Record.Samples += 1
    $Record.ObservedCpuMilliseconds = [Math]::Max(0, ($Row.CpuTicks - $Record.InitialCpuTicks) / 10000.0)
    $Record.ObservedReadBytes = [Math]::Max(0, $Row.ReadBytes - $Record.InitialReadBytes)
    $Record.ObservedWriteBytes = [Math]::Max(0, $Row.WriteBytes - $Record.InitialWriteBytes)
    $Record.PeakSampledRssBytes = [Math]::Max($Record.PeakSampledRssBytes, $Row.WorkingSetBytes)
}

function Find-KnownParent($Row, [hashtable] $ById, [hashtable] $State) {
    if ($ById.ContainsKey($Row.ParentId)) {
        $current = $ById[$Row.ParentId]
        if ($current.CreatedTicks -le $Row.CreatedTicks) {
            $identity = Get-ProcessIdentity $current
            if ($State.ContainsKey($identity)) { return $identity }
            return $null
        }
    }
    # ~keep Windows retains the original PPID after exit. Bound historical adoption by the
    # ~keep observed lifetime, and reject a newer unrelated PID generation above.
    $parents = @($State.Values | Where-Object {
        $_.ProcessId -eq $Row.ParentId -and $_.CreatedTicks -le $Row.CreatedTicks -and
        ($null -eq $_.ExitObservedTicks -or $Row.CreatedTicks -le $_.ExitObservedTicks)
    } | Sort-Object CreatedTicks -Descending)
    if ($parents.Count -gt 0) { return $parents[0].Identity }
    $null
}

function Update-ProcessTree([hashtable] $State, [object[]] $Snapshot, [long] $ObservedTicks) {
    $byId = @{}
    $byIdentity = @{}
    foreach ($row in $Snapshot) {
        $byId[$row.ProcessId] = $row
        $byIdentity[(Get-ProcessIdentity $row)] = $row
    }
    foreach ($record in @($State.Values)) {
        if ($byIdentity.ContainsKey($record.Identity)) {
            Update-ProcessRecord $record $byIdentity[$record.Identity] $ObservedTicks
        } elseif ($null -eq $record.ExitObservedTicks) {
            $record.ExitObservedTicks = $ObservedTicks
        }
    }
    do {
        $previousCount = $State.Count
        foreach ($row in $Snapshot) {
            $identity = Get-ProcessIdentity $row
            if ($State.ContainsKey($identity)) { continue }
            $parent = Find-KnownParent $row $byId $State
            if ($null -ne $parent) {
                $State[$identity] = New-ProcessRecord $row $ObservedTicks $parent
            }
        }
    } while ($State.Count -gt $previousCount)
}

function Get-SnapshotRss([object[]] $Owned) {
    $rss = 0L
    foreach ($row in $Owned) { $rss += $row.WorkingSetBytes }
    $rss
}

function Get-WindowsSnapshot {
    @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $null -ne $_.CreationDate } | ForEach-Object {
        [pscustomobject]@{
            ProcessId = [int]$_.ProcessId; ParentId = [int]$_.ParentProcessId
            CreatedTicks = $_.CreationDate.ToUniversalTime().Ticks; Name = $_.Name
            CpuTicks = [long]$_.KernelModeTime + [long]$_.UserModeTime
            WorkingSetBytes = [long]$_.WorkingSetSize
            ReadBytes = [long]$_.ReadTransferCount; WriteBytes = [long]$_.WriteTransferCount
        }
    })
}
