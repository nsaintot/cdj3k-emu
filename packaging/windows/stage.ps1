# SPDX-License-Identifier: MIT OR Apache-2.0
# stage.ps1 - assemble the tree the Windows installer ships.
#
#   stage.ps1 -Arch x64|arm64 -QemuDir DIR [-Out DIR] [-Binary DIR]
#
# One tree per architecture, installed under %ProgramFiles%\cdj3k-emu:
#
#   bin\                 cdj3k-emu.exe and its DLLs, qemu-system-aarch64.exe,
#                        qemu-img.exe, and the DLLs QemuDir carries
#   share\cdj3k-emu\     Image, patch\, tools\, tap-windows6\ (OemVista.inf,
#                        tap0901.sys, tap0901.cat)
#   installer\           setup-helper.ps1
#   licenses\            LICENSE-*, NOTICE, QEMU's COPYING, the TAP notice
#   cdj3k-emu.ico
#
# bundled::resources() finds share\cdj3k-emu as <exe>\..\share\cdj3k-emu, the
# same rule as the Linux prefix, so the install directory can move.
#
# Prerequisites, all from the repo's own builds:
#   Binary (default dist\windows-bin-<arch>)        - cross-build.sh
#   QemuDir                                          - the QEMU Windows build
#   build\Image, build\docker-out\, guest\out\       - build.sh
# Needs Git for Windows (its bash builds the patch dispatcher) and a Windows
# host (System.Drawing makes the icon).
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Arch,
    [Parameter(Mandatory)][string]$QemuDir,
    [string]$Out,
    [string]$Binary
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Out) { $Out = Join-Path $RepoRoot "dist\windows-$Arch" }

# TAP-Windows6, OpenVPN's Microsoft-attested driver. sha256 of dist.win10.zip.
$TapVersion = '9.27.0'
$TapSha256  = '36e2609b7ceefedcb978ce5c48caf9e0e5af83423717c4e2e3c1d7ebca8f62a5'
$TapUrl     = "https://github.com/OpenVPN/tap-windows6/releases/download/$TapVersion/dist.win10.zip"
$TapArch    = @{ x64 = 'amd64'; arm64 = 'arm64' }[$Arch]

function Fail([string]$Message) { throw "ERROR: $Message" }

function Need-File([string]$Path, [string]$Hint) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { Fail "$Path not found - $Hint" }
}

function Invoke-Native([string]$Exe, [string[]]$Arguments) {
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) { Fail "$Exe exited with $LASTEXITCODE" }
}

function Find-GitBash {
    $git = Get-Command git.exe -ErrorAction SilentlyContinue
    if (-not $git) { Fail 'Git for Windows not found (its bash builds the patch dispatcher)' }
    $root = Split-Path (Split-Path $git.Source)
    foreach ($rel in 'bin\bash.exe', 'usr\bin\bash.exe') {
        $candidate = Join-Path $root $rel
        if (Test-Path $candidate) { return $candidate }
    }
    Fail "no bash.exe under $root"
}

function Copy-File([string]$Source, [string]$Dest) {
    New-Item -ItemType Directory -Force -Path (Split-Path $Dest) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $Dest -Force
}

$bash = Find-GitBash
if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
$Bin   = Join-Path $Out 'bin'
$Share = Join-Path $Out 'share\cdj3k-emu'
New-Item -ItemType Directory -Force -Path $Bin, $Share | Out-Null

# -- The emulator -------------------------------------------------------------
if (-not $Binary) { $Binary = Join-Path $RepoRoot "dist\windows-bin-$Arch" }
Need-File (Join-Path $Binary 'cdj3k-emu.exe') 'run packaging/windows/cross-build.sh, or pass -Binary'
Get-ChildItem -Path $Binary -File | Where-Object { $_.Name -eq 'cdj3k-emu.exe' -or $_.Extension -eq '.dll' } |
    ForEach-Object { Copy-File $_.FullName (Join-Path $Bin $_.Name) }

# -- QEMU ---------------------------------------------------------------------
# QemuDir ships as is; its DLLs load from beside the executables.
foreach ($tool in 'qemu-system-aarch64.exe', 'qemu-img.exe') {
    Need-File (Join-Path $QemuDir $tool) 'point -QemuDir at the QEMU Windows build'
}
if (-not (Get-ChildItem -Path $QemuDir -Filter '*.dll' -File)) {
    Fail "$QemuDir has no DLLs; the QEMU build is not self-contained"
}
Need-File (Join-Path $QemuDir 'COPYING') 'the GPL text ships with QEMU'
Copy-Item -Path (Join-Path $QemuDir '*') -Destination $Bin -Recurse -Force
Remove-Item -LiteralPath (Join-Path $Bin 'COPYING')

# -- Payload ------------------------------------------------------------------
Write-Host '==> Assembling share\cdj3k-emu'
$steps = Get-ChildItem (Join-Path $RepoRoot 'initramfs-patch\patch-rootfs.d') -Filter '*.sh'
$dispatcher = Join-Path $RepoRoot 'scripts\make-patch-dispatcher.sh'
foreach ($f in @($dispatcher) + $steps.FullName) {
    if ([IO.File]::ReadAllText($f).Contains("`r")) {
        Fail "$f has CRLF line endings; check out with core.autocrlf=false (.gitattributes asks for LF)"
    }
}
New-Item -ItemType Directory -Force -Path (Join-Path $Share 'patch') | Out-Null
Push-Location $RepoRoot
try {
    $dispatched = (Join-Path $Share 'patch\patch-rootfs.sh') -replace '\\', '/'
    Invoke-Native $bash @('scripts/make-patch-dispatcher.sh', 'initramfs-patch/patch-rootfs.d', $dispatched)
} finally { Pop-Location }

foreach ($tool in 'cfgd_aarch64', 'pc_link_bridge_aarch64') {
    $src = Join-Path $RepoRoot "guest\out\$tool"
    Need-File $src 'run ./build.sh'
    Copy-File $src (Join-Path $Share "patch\$tool")
}

$mods = Get-ChildItem (Join-Path $RepoRoot 'build\docker-out\modules') -Filter '*.ko' -File -ErrorAction SilentlyContinue
if (-not $mods) { Fail 'build\docker-out\modules\*.ko not found - run ./build.sh' }
foreach ($m in $mods) { Copy-File $m.FullName (Join-Path $Share "patch\vanilla-modules\$($m.Name)") }
Write-Host "     modules: $($mods.Count)"

$dummy = Join-Path $RepoRoot 'build\docker-out\dummy_drv.so'
Need-File $dummy 'run ./build.sh'
Copy-File $dummy (Join-Path $Share 'patch\dummy_drv.so')

foreach ($tool in 'subucom_live', 'subucom_forwarder') {
    $src = Join-Path $RepoRoot "guest\out\${tool}_aarch64"
    Need-File $src 'run ./build.sh --modules-only'
    Copy-File $src (Join-Path $Share "tools\$tool")
}
$shim = Join-Path $RepoRoot 'guest\out\deck_shim.so'
Need-File $shim 'run: make -C guest'
Copy-File $shim (Join-Path $Share 'tools\deck_shim.so')

$image = Join-Path $RepoRoot 'build\Image'
Need-File $image 'run ./build.sh'
Copy-File $image (Join-Path $Share 'Image')

# -- TAP-Windows6 -------------------------------------------------------------
$cache = Join-Path $RepoRoot 'build\windows-cache'
New-Item -ItemType Directory -Force -Path $cache | Out-Null
$zip = Join-Path $cache "tap-windows6-$TapVersion-dist.win10.zip"
$have = if (Test-Path $zip) { (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower() } else { '' }
if ($have -ne $TapSha256) {
    Write-Host "==> Fetching TAP-Windows6 $TapVersion"
    Invoke-WebRequest -Uri $TapUrl -OutFile $zip -UseBasicParsing
    $got = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if ($got -ne $TapSha256) {
        Remove-Item -Force $zip
        Fail "dist.win10.zip sha256 $got, expected $TapSha256"
    }
}
$tapTmp = Join-Path $cache "tap-$TapVersion"
if (Test-Path $tapTmp) { Remove-Item -Recurse -Force $tapTmp }
Expand-Archive -Path $zip -DestinationPath $tapTmp
foreach ($name in 'OemVista.inf', 'tap0901.sys', 'tap0901.cat') {
    $src = Join-Path $tapTmp "dist.win10\$TapArch\$name"
    Need-File $src "missing from the TAP-Windows6 $TapVersion archive"
    Copy-File $src (Join-Path $Share "tap-windows6\$name")
}
Remove-Item -Recurse -Force $tapTmp
Write-Host "     tap-windows6 $TapVersion ($TapArch)"

# -- Installer support files --------------------------------------------------
Copy-File (Join-Path $PSScriptRoot 'setup-helper.ps1') (Join-Path $Out 'installer\setup-helper.ps1')

# -- Licenses -----------------------------------------------------------------
foreach ($name in 'LICENSE-APACHE', 'LICENSE-MIT', 'NOTICE') {
    Need-File (Join-Path $RepoRoot $name) 'the checkout is incomplete'
    Copy-File (Join-Path $RepoRoot $name) (Join-Path $Out "licenses\$name")
}
Copy-File (Join-Path $QemuDir 'COPYING') (Join-Path $Out 'licenses\QEMU-COPYING')
@"
TAP-Windows6 $TapVersion, OpenVPN Inc., GPL-2.0.
Unmodified, from $TapUrl
Source: https://github.com/OpenVPN/tap-windows6/tree/$TapVersion
"@ | Set-Content -Path (Join-Path $Out 'licenses\TAP-WINDOWS6.txt') -Encoding ASCII

# -- Icon ---------------------------------------------------------------------
# A multi-size .ico of PNG frames, from the app's 1024 px icon.
Write-Host '==> Icon'
Add-Type -AssemblyName System.Drawing
$iconSrc = Join-Path $RepoRoot 'app\cdj3k-emu\assets\icon_1024_padded.png'
Need-File $iconSrc 'the checkout is incomplete'
$sizes = 16, 24, 32, 48, 64, 128, 256
$source = [Drawing.Image]::FromFile($iconSrc)
$frames = foreach ($s in $sizes) {
    $bmp = New-Object Drawing.Bitmap $s, $s, ([Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.DrawImage($source, 0, 0, $s, $s)
    $g.Dispose()
    $ms = New-Object IO.MemoryStream
    $bmp.Save($ms, [Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    , $ms.ToArray()
}
$source.Dispose()
$fs = [IO.File]::Create((Join-Path $Out 'cdj3k-emu.ico'))
$w = New-Object IO.BinaryWriter $fs
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $dim = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([uint16]1); $w.Write([uint16]32)
    $w.Write([uint32]$frames[$i].Length); $w.Write([uint32]$offset)
    $offset += $frames[$i].Length
}
foreach ($f in $frames) { $w.Write([byte[]]$f) }
$w.Dispose(); $fs.Dispose()

# -- Completeness check -------------------------------------------------------
Write-Host '==> Checking the tree'
$required = @(
    'bin\cdj3k-emu.exe', 'bin\qemu-system-aarch64.exe', 'bin\qemu-img.exe',
    'share\cdj3k-emu\Image', 'share\cdj3k-emu\patch\patch-rootfs.sh',
    'share\cdj3k-emu\patch\cfgd_aarch64', 'share\cdj3k-emu\patch\pc_link_bridge_aarch64',
    'share\cdj3k-emu\patch\dummy_drv.so',
    'share\cdj3k-emu\tools\subucom_live', 'share\cdj3k-emu\tools\subucom_forwarder',
    'share\cdj3k-emu\tools\deck_shim.so',
    'share\cdj3k-emu\tap-windows6\OemVista.inf', 'share\cdj3k-emu\tap-windows6\tap0901.sys',
    'share\cdj3k-emu\tap-windows6\tap0901.cat',
    'installer\setup-helper.ps1', 'cdj3k-emu.ico',
    'licenses\LICENSE-APACHE', 'licenses\LICENSE-MIT', 'licenses\NOTICE',
    'licenses\QEMU-COPYING', 'licenses\TAP-WINDOWS6.txt'
)
$missing = $required | Where-Object { -not (Test-Path (Join-Path $Out $_)) }
if ($missing) { Fail "staged tree is missing: $($missing -join ', ')" }
if (-not (Get-ChildItem (Join-Path $Share 'patch\vanilla-modules') -Filter '*.ko')) { Fail 'no guest modules staged' }

$size = (Get-ChildItem $Out -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ('==> Staged {0:N0} MB at {1}' -f $size, $Out)
