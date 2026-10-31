[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$cargoText = Get-Content -LiteralPath (Join-Path $projectRoot 'Cargo.toml') -Raw
$version = [regex]::Match($cargoText, '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
if (-not $version) { throw 'Version was not found in Cargo.toml' }
$executable = Join-Path $projectRoot 'target/release/xmouse.exe'
& (Join-Path $PSScriptRoot 'verify-portable.ps1') -Executable $executable

# A source archive must describe the exact committed source, not a stale HEAD.
Push-Location $projectRoot
try {
    if (git status --porcelain) { throw 'Commit source changes before packaging.' }
    if ($LASTEXITCODE -ne 0) { throw 'git status failed' }
    $staging = Join-Path $projectRoot ('work/package-' + $version + '-' + [Guid]::NewGuid().ToString('N'))
    $portable = Join-Path $staging 'Xmouse'
    New-Item -ItemType Directory -Path $portable -Force | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $portable 'Xmouse.exe')
    $documents = @{
        'README.md' = 'PROJECT-README.md'
        'LICENSE' = 'LICENSE'
        'THIRD-PARTY-NOTICES.md' = 'THIRD-PARTY-NOTICES.md'
        'CHANGELOG.md' = 'CHANGELOG.md'
        'config.example.json' = 'config.example.json'
        "docs/RELEASE-NOTES-$version.md" = 'RELEASE-NOTES.md'
        'docs/ARCHITECTURE.md' = 'ARCHITECTURE.md'
        'docs/REGRESSION-TEST-PLAN.md' = 'REGRESSION-TEST-PLAN.md'
        'docs/CURRENT-STATE-AND-ROADMAP.md' = 'CURRENT-STATE-AND-ROADMAP.md'
        'docs/TECHNICAL-REVIEW-2026-10-31.md' = 'TECHNICAL-REVIEW.md'
    }
    foreach ($entry in $documents.GetEnumerator()) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $entry.Key) -Destination (Join-Path $portable $entry.Value)
    }
    $zip = Join-Path $staging "Xmouse-$version-windows-x64.zip"
    Compress-Archive -LiteralPath $portable -DestinationPath $zip
    $source = Join-Path $staging "Xmouse-$version-source.zip"
    git archive --format=zip --output=$source HEAD
    if ($LASTEXITCODE -ne 0) { throw 'git archive failed' }
    $hashes = @($zip, $source) | ForEach-Object {
        $hash = Get-FileHash -LiteralPath $_ -Algorithm SHA256
        "$($hash.Hash.ToLowerInvariant())  $([IO.Path]::GetFileName($_))"
    }
    $hashes | Set-Content -LiteralPath (Join-Path $staging 'SHA256SUMS.txt') -Encoding ascii
    Write-Host "Verified package: $staging"
    Write-Host 'Promote the Xmouse directory to latest only after stopping the installed instance; archive the previous release first.'
}
finally {
    Pop-Location
}
