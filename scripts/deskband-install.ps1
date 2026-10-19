[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Install', 'Uninstall')]
    [string]$Action,
    [Parameter(Mandatory = $true)]
    [string]$InstallDirectory
)

$ErrorActionPreference = 'Stop'
$bandId = '{4A7B6D93-4829-4CE3-977E-D25194348A6A}'
$classKey = "SOFTWARE\Classes\CLSID\$bandId"
$approvedKey = 'SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved'
$managedRoot = [System.IO.Path]::GetFullPath((Join-Path $env:ProgramData 'CodexQuotaFloat\DeskBand'))
$resources = $InstallDirectory
$controller = Join-Path $resources 'deskbandctl.exe'
$sourceDll = Join-Path $resources 'codex-deskband.dll'
$regsvr32Directory = if ([Environment]::Is64BitProcess) { 'System32' } else { 'Sysnative' }
$regsvr32 = Join-Path $env:WINDIR "$regsvr32Directory\regsvr32.exe"

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class DelayedFileRemoval {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool MoveFileEx(string source, IntPtr destination, int flags);
}
'@

function Get-RegisteredDll {
    $root = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::LocalMachine,
        [Microsoft.Win32.RegistryView]::Registry64
    )
    try {
        $key = $root.OpenSubKey("$classKey\InprocServer32")
        if (-not $key) { return $null }
        try { return [string]$key.GetValue('') }
        finally { $key.Dispose() }
    }
    finally { $root.Dispose() }
}

function Remove-Registration {
    $root = [Microsoft.Win32.RegistryKey]::OpenBaseKey(
        [Microsoft.Win32.RegistryHive]::LocalMachine,
        [Microsoft.Win32.RegistryView]::Registry64
    )
    try {
        $root.DeleteSubKeyTree($classKey, $false)
        $approved = $root.OpenSubKey($approvedKey, $true)
        if ($approved) {
            try { $approved.DeleteValue($bandId, $false) }
            finally { $approved.Dispose() }
        }
    }
    finally { $root.Dispose() }
}

function Invoke-Registration {
    param([string]$Dll, [switch]$Unregister)
    $arguments = if ($Unregister) { '/u /s "' + $Dll + '"' } else { '/s "' + $Dll + '"' }
    $process = Start-Process -FilePath $regsvr32 -ArgumentList $arguments -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) {
        throw "regsvr32 failed with exit code $($process.ExitCode): $Dll"
    }
}

function Test-ManagedDll {
    param([string]$Path)
    if (-not $Path) { return $false }
    $full = [System.IO.Path]::GetFullPath($Path)
    $prefix = $managedRoot.TrimEnd('\') + '\'
    return $full.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)
}

function Remove-ManagedDll {
    param([string]$Path)
    if (-not (Test-ManagedDll $Path)) { return }
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return }
    try {
        Remove-Item -LiteralPath $Path -Force
    }
    catch {
        if (-not [DelayedFileRemoval]::MoveFileEx($Path, [IntPtr]::Zero, 4)) {
            throw "Could not remove or schedule removal of $Path"
        }
    }
}

function Remove-StaleDlls {
    param([string]$Keep)
    if (-not (Test-Path -LiteralPath $managedRoot -PathType Container)) { return }
    Get-ChildItem -LiteralPath $managedRoot -Filter 'codex-deskband-*.dll' -File |
        Where-Object { -not $Keep -or $_.FullName -ne $Keep } |
        ForEach-Object { Remove-ManagedDll -Path $_.FullName }
}

function Restart-ExplorerWithOldBand {
    param([string]$CurrentDll)
    $hosts = @(Get-Process explorer -ErrorAction SilentlyContinue | Where-Object {
        $process = $_
        try {
            @($process.Modules | Where-Object {
                (Test-ManagedDll $_.FileName) -and $_.FileName -ne $CurrentDll
            }).Count -gt 0
        }
        catch { $false }
    })
    if ($hosts.Count -eq 0) { return }

    $oldIds = @($hosts | ForEach-Object { $_.Id })
    $hosts | Stop-Process -Force -ErrorAction Stop
    $deadline = (Get-Date).AddSeconds(10)
    do {
        Start-Sleep -Milliseconds 500
        $replacement = @(Get-Process explorer -ErrorAction SilentlyContinue |
            Where-Object { $oldIds -notcontains $_.Id })
    } while ($replacement.Count -eq 0 -and (Get-Date) -lt $deadline)
    if ($replacement.Count -eq 0) {
        Start-Process -FilePath (Join-Path $env:WINDIR 'explorer.exe')
    }
    if (Test-Path -LiteralPath $controller -PathType Leaf) {
        for ($attempt = 0; $attempt -lt 20; $attempt++) {
            & $controller is-shown | Out-Null
            if ($LASTEXITCODE -eq 0 -or $LASTEXITCODE -eq 1) { break }
            Start-Sleep -Milliseconds 500
        }
    }
}

function Hide-Toolbar {
    if (Test-Path -LiteralPath $controller -PathType Leaf) {
        & $controller hide | Out-Null
    }
}

if ($Action -eq 'Install') {
    if (-not (Test-Path -LiteralPath $sourceDll -PathType Leaf)) {
        throw "DeskBand DLL is missing: $sourceDll"
    }
    New-Item -ItemType Directory -Path $managedRoot -Force | Out-Null
    $previous = Get-RegisteredDll
    $destination = Join-Path $managedRoot ("codex-deskband-" + [guid]::NewGuid().ToString('N') + '.dll')
    Copy-Item -LiteralPath $sourceDll -Destination $destination
    try {
        if ($previous) {
            Hide-Toolbar
            if (Test-Path -LiteralPath $previous -PathType Leaf) {
                try { Invoke-Registration -Dll $previous -Unregister }
                catch { Write-Warning "DLL unregister failed; clearing its registry entries directly: $_" }
            }
        }
        Remove-Registration
        Invoke-Registration -Dll $destination
        if ((Get-RegisteredDll) -ne $destination) {
            throw 'DeskBand registration did not point to the new DLL.'
        }
        if (Test-Path -LiteralPath $controller -PathType Leaf) {
            & $controller refresh | Out-Null
        }
        Restart-ExplorerWithOldBand -CurrentDll $destination
        Remove-StaleDlls -Keep $destination
    }
    catch {
        Remove-Registration
        Remove-ManagedDll -Path $destination
        if ($previous -and (Test-Path -LiteralPath $previous -PathType Leaf)) {
            Invoke-Registration -Dll $previous
        }
        throw
    }
}
else {
    $registered = Get-RegisteredDll
    if ($registered) {
        Hide-Toolbar
        if (Test-Path -LiteralPath $registered -PathType Leaf) {
            try { Invoke-Registration -Dll $registered -Unregister }
            catch { Write-Warning "DLL unregister failed; clearing its registry entries directly: $_" }
        }
    }
    Remove-Registration
    if (Get-RegisteredDll) { throw 'DeskBand registration still exists after uninstall.' }
    Remove-StaleDlls
}
