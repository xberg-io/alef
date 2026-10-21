Set-StrictMode -Version Latest

function Assert-DiagnosticReceipts([object[]] $Receipts) {
    if ($Receipts.Count -ne 8) { throw "Expected 8 diagnostic receipts; observed $($Receipts.Count)." }
    $identities = @{}
    foreach ($receipt in $Receipts) {
        $identity = '{0}/{1}' -f $receipt.phase, $receipt.case
        if ($identities.ContainsKey($identity)) { throw "Duplicate receipt: $identity" }
        $identities[$identity] = $true
        if ($receipt.phase -notin @('cold', 'warm') -or
            $receipt.case -notin @('include_declared', 'include_missing', 'cwd_rebased', 'cwd_raw')) {
            throw "Unexpected diagnostic case: $identity"
        }
        if (-not $receipt.qualified -or $receipt.budget_seconds -ne 300 -or
            $receipt.actual -ne $receipt.expected) { throw "Unqualified diagnostic receipt: $identity" }
        $expected = if ($receipt.case -in @('include_declared', 'cwd_rebased')) { 'pass' } else { 'fail' }
        if ($receipt.expected -ne $expected) { throw "Incorrect control expectation: $identity" }
        if ($receipt.phase -eq 'cold' -and $receipt.cache_before.files -ne 0) {
            throw "Cold cache was populated: $identity"
        }
        if ($receipt.phase -eq 'warm' -and $receipt.cache_before.files -le 0) {
            throw "Warm cache was empty: $identity"
        }
    }
    $Receipts
}

function Read-DiagnosticReceipts([string] $LogPath) {
    $prefix = 'ALEF_ZIG_DIAGNOSTIC_RECEIPT '
    @(Get-Content -LiteralPath $LogPath | Where-Object { $_.StartsWith($prefix) } | ForEach-Object {
        $_.Substring($prefix.Length) | ConvertFrom-Json
    })
}
