#!/usr/bin/env pwsh
# lwid installer for Windows
# Usage: irm https://raw.githubusercontent.com/Marlinski/lwid/main/install.ps1 | iex

$ErrorActionPreference = 'Stop'

$Repo       = 'Marlinski/lwid'
$BinaryName = 'lwid.exe'
$Asset      = 'lwid-windows-x86_64.exe'
$InstallDir = Join-Path $env:LOCALAPPDATA 'lwid'
$InstallPath = Join-Path $InstallDir $BinaryName

$Version = if ($env:LWID_VERSION -and $env:LWID_VERSION -ne '') { $env:LWID_VERSION } else { 'latest' }

# Build download URL
if ($Version -eq 'latest') {
    $Url = "https://github.com/$Repo/releases/latest/download/$Asset"
} else {
    $Url = "https://github.com/$Repo/releases/download/$Version/$Asset"
}

# Download
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Write-Host "Downloading $Asset..."
Invoke-WebRequest -Uri $Url -OutFile $InstallPath -UseBasicParsing

# Add to user PATH if not already present
$UserPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable('PATH', "$InstallDir;$UserPath", 'User')
    Write-Host "`nAdded $InstallDir to your PATH (restart your terminal to take effect)."
}

Write-Host "`nlwid installed to $InstallPath"

# Write default server config if DEFAULT_SERVER is set
if ($env:DEFAULT_SERVER -and $env:DEFAULT_SERVER -ne '') {
    $ConfigDir = Join-Path $env:APPDATA 'lwid'
    New-Item -ItemType Directory -Force -Path $ConfigDir | Out-Null
    $ConfigPath = Join-Path $ConfigDir 'config.toml'
    "[defaults]`nserver = `"$($env:DEFAULT_SERVER)`"" | Set-Content -Path $ConfigPath -Encoding UTF8
    Write-Host "Default server set to $($env:DEFAULT_SERVER)"
}

Write-Host "Run: lwid --help"
