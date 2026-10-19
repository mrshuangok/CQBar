[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Debug',
    [switch]$OnlyController,
    [ValidatePattern('^[A-Za-z0-9-]+$')]
    [string]$OutputSubdirectory = ''
)

$ErrorActionPreference = 'Stop'
$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$sourceRoot = Join-Path $projectRoot 'native\deskband'
$outputRoot = Join-Path $projectRoot ('src-tauri\target\' + $Configuration.ToLowerInvariant())
if ($OutputSubdirectory) { $outputRoot = Join-Path $outputRoot $OutputSubdirectory }
$vswhere = Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'Microsoft Visual Studio\Installer\vswhere.exe'
$vsPath = (& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1)
if (-not $vsPath) { throw 'Visual Studio C++ x64 build tools not found.' }
$msvcVersion = Get-ChildItem (Join-Path $vsPath 'VC\Tools\MSVC') -Directory | Sort-Object Name -Descending | Select-Object -First 1
$msvc = $msvcVersion.FullName
$sdkRoot = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots').KitsRoot10.TrimEnd('\')
$sdkVersion = Get-ChildItem (Join-Path $sdkRoot 'Include') -Directory | Sort-Object Name -Descending | Select-Object -First 1
$sdk = $sdkVersion.Name
$cl = Join-Path $msvc 'bin\Hostx64\x64\cl.exe'
$env:PATH = (Split-Path $cl) + ';' + $env:PATH
$env:INCLUDE = @(
    (Join-Path $msvc 'include'),
    (Join-Path $sdkRoot "Include\$sdk\ucrt"),
    (Join-Path $sdkRoot "Include\$sdk\shared"),
    (Join-Path $sdkRoot "Include\$sdk\um"),
    (Join-Path $sdkRoot "Include\$sdk\winrt")
) -join ';'
$env:LIB = @(
    (Join-Path $msvc 'lib\x64'),
    (Join-Path $sdkRoot "Lib\$sdk\ucrt\x64"),
    (Join-Path $sdkRoot "Lib\$sdk\um\x64")
) -join ';'
New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
$optimize = if ($Configuration -eq 'Release') { '/O2' } else { '/Od' }
Push-Location $outputRoot
try {
    if (-not $OnlyController) {
        $dllArgs = @('/nologo', '/utf-8', '/std:c++17', '/EHsc', '/MT', $optimize, '/DUNICODE', '/D_UNICODE', '/LD', (Join-Path $sourceRoot 'deskband.cpp'), '/Fe:codex-deskband.dll', '/link', '/NOLOGO', ('/DEF:' + (Join-Path $sourceRoot 'deskband.def')), '/OUT:codex-deskband.dll', 'ole32.lib', 'uuid.lib', 'shell32.lib', 'user32.lib', 'gdi32.lib', 'gdiplus.lib', 'advapi32.lib')
        & $cl @dllArgs
        if ($LASTEXITCODE -ne 0) { throw 'DeskBand DLL build failed.' }
    }
    $ctlArgs = @('/nologo', '/utf-8', '/std:c++17', '/EHsc', '/MT', $optimize, '/DUNICODE', '/D_UNICODE', (Join-Path $sourceRoot 'deskbandctl.cpp'), '/Fe:deskbandctl.exe', '/link', '/NOLOGO', '/OUT:deskbandctl.exe', 'ole32.lib', 'uuid.lib', 'user32.lib')
    & $cl @ctlArgs
    if ($LASTEXITCODE -ne 0) { throw 'DeskBand controller build failed.' }
}
finally {
    Pop-Location
}
