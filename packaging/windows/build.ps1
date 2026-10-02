# SPDX-License-Identifier: MIT OR Apache-2.0
# build.ps1 - stage and compile the installer for one architecture.
#
#   build.ps1 -Arch x64|arm64 -QemuDir DIR [-Out DIR] [-Stage DIR] [-Version V]
#             [-Binary DIR] [-SkipStage]
#
# Output: <Out>\CDJ3K-Emulator-<version>-windows-<arch>.exe (Out defaults to
# dist\). Needs Inno Setup 6.6 or newer.
#
# Signing is on when CDJ3K_SIGN_CERT_SHA1 holds a certificate thumbprint:
#   CDJ3K_SIGN_CERT_SHA1     thumbprint of a code-signing certificate
#   CDJ3K_SIGN_TIMESTAMP_URL RFC 3161 server (default http://timestamp.digicert.com)
#   CDJ3K_SIGNTOOL           signtool.exe (default: newest under Windows Kits)
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Arch,
    [Parameter(Mandatory)][string]$QemuDir,
    [string]$Out,
    [string]$Stage,
    [string]$Version,
    [string]$Binary,
    [switch]$SkipStage
)
$ErrorActionPreference = 'Stop'

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Out) { $Out = Join-Path $RepoRoot 'dist' }
if (-not $Stage) { $Stage = Join-Path $RepoRoot "dist\windows-$Arch" }
if (-not $Version) {
    $line = Select-String -Path (Join-Path $RepoRoot 'Cargo.toml') -Pattern '^version\s*=\s*"(.*)"' | Select-Object -First 1
    if (-not $line) { throw 'ERROR: no version in Cargo.toml' }
    $Version = $line.Matches[0].Groups[1].Value
}

function Find-Iscc {
    $onPath = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    foreach ($dir in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, (Join-Path $env:LOCALAPPDATA 'Programs'))) {
        if (-not $dir) { continue }
        $candidate = Join-Path $dir 'Inno Setup 6\ISCC.exe'
        if (Test-Path $candidate) { return $candidate }
    }
    throw 'ERROR: ISCC.exe not found; install Inno Setup 6.6 or newer'
}

function Find-SignTool {
    if ($env:CDJ3K_SIGNTOOL) { return $env:CDJ3K_SIGNTOOL }
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $found = Get-ChildItem $kits -Recurse -Filter signtool.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match "\\$Arch\\" } |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $found) { throw 'ERROR: signtool.exe not found; set CDJ3K_SIGNTOOL' }
    return $found.FullName
}

if (-not $SkipStage) {
    $stageArgs = @{ Arch = $Arch; QemuDir = $QemuDir; Out = $Stage }
    if ($Binary) { $stageArgs.Binary = $Binary }
    & (Join-Path $PSScriptRoot 'stage.ps1') @stageArgs
}
if (-not (Test-Path (Join-Path $Stage 'bin\cdj3k-emu.exe'))) {
    throw "ERROR: $Stage is not a staged tree - run packaging\windows\stage.ps1"
}

$iscc = Find-Iscc
$isccArgs = @(
    "/DAppVersion=$Version", "/DArch=$Arch", "/DStageDir=$Stage", "/O$Out"
)
if ($env:CDJ3K_SIGN_CERT_SHA1) {
    $timestamp = if ($env:CDJ3K_SIGN_TIMESTAMP_URL) { $env:CDJ3K_SIGN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
    $signtool = Find-SignTool
    # $q is ISCC's escape for a quote, $f the file to sign.
    $isccArgs += '/DSign'
    $isccArgs += "/Scdj3k=`$q$signtool`$q sign /fd sha256 /td sha256 /tr $timestamp /sha1 $($env:CDJ3K_SIGN_CERT_SHA1) `$q`$f`$q"
    Write-Host '==> Signing on'
} else {
    Write-Host '==> No CDJ3K_SIGN_CERT_SHA1; the installer is unsigned'
}
$isccArgs += (Join-Path $PSScriptRoot 'cdj3k-emu.iss')

New-Item -ItemType Directory -Force -Path $Out | Out-Null
Write-Host "==> ISCC ($Arch, $Version)"
& $iscc @isccArgs
if ($LASTEXITCODE -ne 0) { throw "ERROR: ISCC exited with $LASTEXITCODE" }

$artefact = Join-Path $Out "CDJ3K-Emulator-$Version-windows-$Arch.exe"
if (-not (Test-Path $artefact)) { throw "ERROR: $artefact was not produced" }
Write-Host "==> $artefact"
