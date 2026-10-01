# SPDX-License-Identifier: MIT OR Apache-2.0
# setup-helper.ps1 - the privileged steps the installer and uninstaller run.
#
#   setup-helper.ps1 -Action InstallTap -DriverDir DIR
#   setup-helper.ps1 -Action RemoveTap
#   setup-helper.ps1 -Action EnableHypervisorPlatform
#
# Runs elevated, from <install dir>\installer. Exit code 0 is done, 3010 is
# done and a restart is pending, anything else is a failure.
#
# The TAP-Windows6 package is shared with OpenVPN. installer\tap-owned marks
# that InstallTap added it to a store that held no TAP-Windows package;
# RemoveTap deletes the package only under that marker.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('InstallTap', 'RemoveTap', 'EnableHypervisorPlatform')][string]$Action,
    [string]$DriverDir
)
$ErrorActionPreference = 'Stop'

$Marker = Join-Path $PSScriptRoot 'tap-owned'
$AdapterPrefix = 'cdj3k-emu-'

# The published name (oemNN.inf) of each TAP-Windows package in the driver
# store. pnputil's text output is localised.
function Get-TapPackages {
    Get-WindowsDriver -Online -All |
        Where-Object { $_.ProviderName -like 'TAP-Windows*' -and $_.OriginalFileName -match 'oemvista\.inf$' } |
        ForEach-Object { $_.Driver }
}

# The version in an INF's DriverVer line ("DriverVer = 02/27/2024,9.27.0.0").
function Get-InfVersion([string]$Inf) {
    $m = Select-String -LiteralPath $Inf -Pattern '^\s*DriverVer\s*=\s*[^,]*,\s*([0-9.]+)' | Select-Object -First 1
    if ($m) { [version]$m.Matches[0].Groups[1].Value }
}

# The version of every TAP-Windows6 package in the driver store, from its
# FileRepository\oemvista.inf_<arch>_<hash> copies.
function Get-StoredTapVersions {
    $repo = Join-Path $env:SystemRoot 'System32\DriverStore\FileRepository'
    Get-ChildItem -LiteralPath $repo -Directory -Filter 'oemvista.inf_*' -ErrorAction SilentlyContinue |
        Where-Object { Test-Path (Join-Path $_.FullName 'tap0901.sys') } |
        ForEach-Object { Get-InfVersion (Join-Path $_.FullName 'OemVista.inf') } |
        Where-Object { $_ }
}

# Devices bound to tap0901, present or not.
function Get-TapDevices {
    Get-PnpDevice -Class Net -ErrorAction SilentlyContinue |
        Where-Object { $_.HardwareID -contains 'tap0901' }
}

function Invoke-Pnputil([string[]]$Arguments) {
    & "$env:SystemRoot\System32\pnputil.exe" @Arguments | ForEach-Object { Write-Host $_ }
    return $LASTEXITCODE
}

switch ($Action) {
    'InstallTap' {
        $inf = Join-Path $DriverDir 'OemVista.inf'
        if (-not (Test-Path $inf)) { throw "$inf not found" }
        $ours = Get-InfVersion $inf
        $stored = @(Get-StoredTapVersions)
        $current = $stored | Where-Object { $ours -and $_ -ge $ours } | Select-Object -First 1
        if ($current) {
            Write-Host "TAP-Windows6 $current is already in the driver store"
            exit 0
        }
        $preexisting = $stored.Count -gt 0
        $code = Invoke-Pnputil @('/add-driver', $inf, '/install')
        # 259: added to the store, no matching device (none before the first adapter).
        if ($code -eq 259) { $code = 0 }
        if ($code -ne 0 -and $code -ne 3010) { exit $code }
        if (-not $preexisting) { Set-Content -Path $Marker -Value 'cdj3k-emu installed the TAP-Windows6 package' }
        exit $code
    }

    'RemoveTap' {
        # The Network Bridge the app made (net/windows_helper.rs records its GUID).
        $owned = Join-Path $env:ProgramData 'cdj3k-emu\bridge-owned'
        if (Test-Path $owned) {
            $guid = (Get-Content -Raw $owned).Trim()
            if ((netsh bridge list | Out-String) -match [regex]::Escape($guid)) {
                Write-Host "destroying bridge $guid"
                netsh bridge destroy $guid | Out-Null
            }
            Remove-Item -Recurse -Force (Split-Path $owned) -ErrorAction SilentlyContinue
        }
        $ours = Get-NetAdapter -IncludeHidden -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like "$AdapterPrefix*" -and $_.InterfaceDescription -like 'TAP-Windows*' }
        foreach ($adapter in $ours) {
            Write-Host "removing adapter $($adapter.Name)"
            [void](Invoke-Pnputil @('/remove-device', $adapter.PnPDeviceID))
        }
        if (-not (Test-Path $Marker)) { exit 0 }
        if (Test-Path 'HKLM:\SOFTWARE\OpenVPN') { Write-Host 'OpenVPN is installed; keeping the driver'; exit 0 }
        if (@(Get-TapDevices).Count -gt 0) { Write-Host 'other TAP adapters exist; keeping the driver'; exit 0 }
        foreach ($published in @(Get-TapPackages)) {
            Write-Host "deleting driver package $published"
            [void](Invoke-Pnputil @('/delete-driver', $published))
        }
        exit 0
    }

    'EnableHypervisorPlatform' {
        $feature = Get-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform
        if ($feature.State -eq 'Enabled') { exit 0 }
        if ($feature.State -eq 'EnablePending') { exit 3010 }
        & "$env:SystemRoot\System32\dism.exe" /Online /Enable-Feature /FeatureName:HypervisorPlatform /All /NoRestart
        exit $LASTEXITCODE
    }
}
