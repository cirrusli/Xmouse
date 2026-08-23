[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = 'Stop'
$report = Resolve-Path -LiteralPath $Path
$totals = @{
    LF = 0
    LH = 0
    FNF = 0
    FNH = 0
    BRF = 0
    BRH = 0
}

foreach ($line in Get-Content -LiteralPath $report.Path) {
    if ($line -match '^(LF|LH|FNF|FNH|BRF|BRH):(\d+)$') {
        $totals[$matches[1]] += [int]$matches[2]
    }
}

if ($totals.LF -eq 0) {
    throw 'LCOV report does not contain line totals.'
}

$linePercent = [math]::Round(100 * $totals.LH / $totals.LF, 2)
$functionPercent = if ($totals.FNF -gt 0) {
    [math]::Round(100 * $totals.FNH / $totals.FNF, 2)
} else {
    0
}
$summary = "Line coverage: $linePercent% ($($totals.LH)/$($totals.LF)); function coverage: $functionPercent% ($($totals.FNH)/$($totals.FNF))."
Write-Host $summary

if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_STEP_SUMMARY)) {
    Add-Content -LiteralPath $env:GITHUB_STEP_SUMMARY -Value "## Xmouse coverage`n`n$summary"
}
