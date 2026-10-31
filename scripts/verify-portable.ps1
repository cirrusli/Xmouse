[CmdletBinding()]
param([string]$Executable = (Join-Path $PSScriptRoot '../target/release/xmouse.exe'))
$ErrorActionPreference = 'Stop'
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
$peBytes = [IO.File]::ReadAllBytes($resolvedExecutable)
$peOffset = [BitConverter]::ToInt32($peBytes, 0x3c)
if ([BitConverter]::ToUInt16($peBytes, $peOffset + 4) -ne 0x8664) { throw 'Expected Windows x64 target' }
$optional = $peOffset + 24
if ([BitConverter]::ToUInt16($peBytes, $optional) -ne 0x20b) { throw 'Expected x64 PE executable' }
$sectionCount = [BitConverter]::ToUInt16($peBytes, $peOffset + 6)
$sectionTable = $optional + [BitConverter]::ToUInt16($peBytes, $peOffset + 20)
function Resolve-PeRva([uint32]$rva) {
    for ($i = 0; $i -lt $sectionCount; $i++) {
        $s = $sectionTable + 40 * $i
        $virtual = [BitConverter]::ToUInt32($peBytes, $s + 12)
        $extent = [Math]::Max([BitConverter]::ToUInt32($peBytes, $s + 8), [BitConverter]::ToUInt32($peBytes, $s + 16))
        if ($rva -ge $virtual -and $rva -lt $virtual + $extent) {
            return [int]([BitConverter]::ToUInt32($peBytes, $s + 20) + $rva - $virtual)
        }
    }
    throw "Unmapped PE RVA: $rva"
}
$importOffset = Resolve-PeRva ([BitConverter]::ToUInt32($peBytes, $optional + 120))
$imports = @()
while ([BitConverter]::ToUInt32($peBytes, $importOffset + 12) -ne 0) {
    $nameOffset = Resolve-PeRva ([BitConverter]::ToUInt32($peBytes, $importOffset + 12))
    $end = $nameOffset
    while ($peBytes[$end] -ne 0) { $end++ }
    $imports += [Text.Encoding]::ASCII.GetString($peBytes, $nameOffset, $end - $nameOffset)
    $importOffset += 20
}
if ($imports -match '(?i)sqlite|libgcc|libstdc\+\+|libwinpthread|vcruntime|msvcp1') { throw "External runtime dependency: $($imports -join ', ')" }
$projectRoot = Split-Path -Parent $PSScriptRoot
$probeDirectory = Join-Path $projectRoot ('work/portable 自检 ' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $probeDirectory -Force | Out-Null
$probeExecutable = Join-Path $probeDirectory '独立 Xmouse.exe'
Copy-Item -LiteralPath $resolvedExecutable -Destination $probeExecutable
$stdout = Join-Path $probeDirectory 'stdout.txt'
$stderr = Join-Path $probeDirectory 'stderr.txt'
$process = Start-Process -FilePath $probeExecutable -ArgumentList '--self-test' -WorkingDirectory $probeDirectory -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
if (-not $process.WaitForExit(15000)) {
    Stop-Process -Id $process.Id
    throw 'Portable self-test timed out'
}
$process.Refresh()
if ($process.ExitCode -ne 0) { throw "Portable self-test failed: $($process.ExitCode): $(Get-Content -LiteralPath $stderr -Raw)" }
$result = Get-Content -LiteralPath $stdout -Raw
if ($result -notmatch 'self-test OK') { throw "Unexpected self-test output: $result" }
$manifest = Get-Content -LiteralPath (Join-Path $projectRoot 'Cargo.toml') -Raw
$expectedVersion = [regex]::Match($manifest, '(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"').Groups[1].Value
if ($result -notmatch ('^Xmouse ' + [regex]::Escape($expectedVersion) + ' self-test OK;')) {
    throw "Release executable does not match Cargo.toml version $expectedVersion`: $result"
}
Write-Host $result.Trim()
Write-Host "Portable probe passed without sqlite3.dll: $probeDirectory"
