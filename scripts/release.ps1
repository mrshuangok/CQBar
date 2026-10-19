[CmdletBinding()]
param(
    [switch]$NoOpen
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Invoke-CheckedCommand {
    param([string]$Executable, [string[]]$CommandArguments)
    & $Executable @CommandArguments
    if ($LASTEXITCODE -ne 0) {
        throw "Command failed with exit code ${LASTEXITCODE}: $Executable $($CommandArguments -join ' ')"
    }
}

$projectRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$tauriConfigPath = Join-Path $projectRoot "src-tauri\tauri.conf.json"
$packageJsonPath = Join-Path $projectRoot "package.json"
$releaseRoot = Join-Path $projectRoot "release"
$nsisRoot = Join-Path $projectRoot "src-tauri\target\release\bundle\nsis"
$nodeModulesPath = Join-Path $projectRoot "node_modules"

if (-not (Get-Command npm.cmd -ErrorAction SilentlyContinue)) {
    throw "npm.cmd was not found."
}
if (-not (Get-Command cargo.exe -ErrorAction SilentlyContinue)) {
    throw "cargo.exe was not found."
}

$packageJson = Get-Content -LiteralPath $packageJsonPath -Raw -Encoding UTF8 | ConvertFrom-Json
$tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json
$version = [string]$tauriConfig.version
$productName = [string]$tauriConfig.productName
if ($version -notmatch '^[0-9A-Za-z][0-9A-Za-z.-]*$') {
    throw "Invalid release version: $version"
}
if ([string]$packageJson.version -ne $version) {
    throw "Version mismatch: package.json=$($packageJson.version), tauri.conf.json=$version"
}

Push-Location $projectRoot
try {
    if (-not (Test-Path -LiteralPath $nodeModulesPath -PathType Container)) {
        Invoke-CheckedCommand -Executable "npm.cmd" -CommandArguments @("ci")
    }

    & (Join-Path $PSScriptRoot "build-native.ps1") -Configuration Release
    if ($LASTEXITCODE -ne 0) {
        throw "Native DeskBand build failed."
    }

    $env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc"
    Invoke-CheckedCommand -Executable "npm.cmd" -CommandArguments @("run", "tauri:build", "--", "--config", "src-tauri/tauri.release.conf.json")

    $setupPath = Join-Path $nsisRoot "${productName}_${version}_x64-setup.exe"
    if (-not (Test-Path -LiteralPath $setupPath -PathType Leaf)) {
        throw "NSIS installer was not generated at $setupPath"
    }

    New-Item -ItemType Directory -Path $releaseRoot -Force | Out-Null
    $releaseName = "${productName}-v${version}-windows-x64-setup.exe"
    $destination = Join-Path $releaseRoot $releaseName
    Copy-Item -LiteralPath $setupPath -Destination $destination -Force
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $stream = [System.IO.File]::OpenRead($destination)
        try {
            $hash = [System.BitConverter]::ToString($sha256.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
        }
        finally {
            $stream.Dispose()
        }
    }
    finally {
        $sha256.Dispose()
    }
    "$hash  $releaseName" | Set-Content -LiteralPath (Join-Path $releaseRoot "SHA256SUMS.txt") -Encoding ASCII

    Write-Host "Installer: $destination" -ForegroundColor Green
    Write-Host "SHA-256: $hash"
    if (-not $NoOpen) {
        Start-Process explorer.exe -ArgumentList ('/select,"' + $destination + '"')
    }
}
finally {
    Pop-Location
}
