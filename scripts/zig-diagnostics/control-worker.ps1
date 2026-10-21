param(
    [Parameter(Mandatory)][string] $Directory,
    [ValidateSet('Busy', 'Idle', 'Fail')][string] $Mode
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$self = Get-CimInstance Win32_Process -Filter "ProcessId = $PID"
[pscustomobject]@{ ProcessId = $PID; CreatedTicks = $self.CreationDate.ToUniversalTime().Ticks } |
    ConvertTo-Json | Set-Content (Join-Path $Directory 'control-child.json')
if ($Mode -eq 'Busy') {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $deadline) { $null = [Math]::Sqrt(987654321.0) }
} else { Start-Sleep -Seconds 5 }
if ($Mode -eq 'Fail') { exit 7 }
