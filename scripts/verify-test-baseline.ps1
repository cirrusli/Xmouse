[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$sourceFiles = Get-ChildItem -LiteralPath (Join-Path $projectRoot 'src') -Recurse -Filter '*.rs' -File

$regularTests = 0
$ignoredTests = 0
foreach ($sourceFile in $sourceFiles) {
    $content = Get-Content -LiteralPath $sourceFile.FullName -Raw
    $allTests = [regex]::Matches($content, '(?m)^\s*#\[test\]\s*$').Count
    $ignored = [regex]::Matches(
        $content,
        '(?ms)^\s*#\[test\]\s*\r?\n\s*#\[ignore(?:\s*=\s*"[^"]*")?\]\s*$'
    ).Count
    $regularTests += $allTests - $ignored
    $ignoredTests += $ignored
}

$minimumRegularTests = 51
$minimumIgnoredTests = 1
if ($regularTests -lt $minimumRegularTests) {
    throw "Regular Rust test count regressed: $regularTests found, minimum is $minimumRegularTests."
}
if ($ignoredTests -lt $minimumIgnoredTests) {
    throw "Interactive regression test count regressed: $ignoredTests found, minimum is $minimumIgnoredTests."
}

Write-Host "Regression test census passed: $regularTests regular, $ignoredTests interactive/ignored."
