# synapse install script for Windows (PowerShell)
#
# Usage:
#   irm https://raw.githubusercontent.com/Kodjaoglanian/synapse/main/install.ps1 | iex
#
# Or pin a specific version:
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/Kodjaoglanian/synapse/main/install.ps1))) -Version "v0.1.0"
#
# Flags:
#   -Version <tag>   Install a specific release tag (default: latest)
#   -Prefix <dir>    Install prefix (default: $env:LOCALAPPDATA\Programs\synapse)

param(
    [string]$Version = "latest",
    [string]$Prefix = ""
)

$ErrorActionPreference = "Stop"

$Repo = "Kodjaoglanian/synapse"

# --- Determine install prefix ---
if (-not $Prefix) {
    $Prefix = Join-Path $env:LOCALAPPDATA "Programs\synapse"
}

# --- Detect architecture ---
$Arch = $env:PROCESSOR_ARCHITECTURE
if ($Arch -eq "AMD64") {
    $Target = "x86_64-pc-windows-msvc"
} elseif ($Arch -eq "ARM64") {
    Write-Host "ARM64 Windows is not yet supported. Please open an issue." -ForegroundColor Red
    exit 1
} else {
    Write-Host "Unsupported architecture: $Arch" -ForegroundColor Red
    exit 1
}

# --- Resolve version ---
if ($Version -eq "latest") {
    Write-Host ">>> Fetching latest release tag..." -ForegroundColor Cyan
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest"
    $Version = $release.tag_name
}

Write-Host ">>> Installing synapse $Version for $Target" -ForegroundColor Cyan

# --- Determine download URL ---
$AssetName = "synapse-$Version-$Target.zip"
$DownloadUrl = "https://github.com/$Repo/releases/download/$Version/$AssetName"

Write-Host ">>> Downloading $DownloadUrl"

# --- Download to a temp directory ---
$TempDir = Join-Path $env:TEMP "synapse-install-$(Get-Random)"
New-Item -ItemType Directory -Path $TempDir -Force | Out-Null

try {
    $ZipPath = Join-Path $TempDir $AssetName
    Invoke-WebRequest -Uri $DownloadUrl -OutFile $ZipPath -UseBasicParsing
} catch {
    Write-Host "Download failed. The asset may not exist for $Target." -ForegroundColor Red
    Write-Host "Check available assets at: https://github.com/$Repo/releases/tag/$Version" -ForegroundColor Yellow
    Remove-Item -Recurse -Force $TempDir
    exit 1
}

Write-Host ">>> Extracting..."
Expand-Archive -Path $ZipPath -DestinationPath $TempDir -Force

# --- Find the binary ---
$Binary = Get-ChildItem -Path $TempDir -Filter "synapse.exe" -Recurse | Select-Object -First 1
if (-not $Binary) {
    Write-Host "Binary not found in archive." -ForegroundColor Red
    Remove-Item -Recurse -Force $TempDir
    exit 1
}

# --- Install ---
if (-not (Test-Path $Prefix)) {
    New-Item -ItemType Directory -Path $Prefix -Force | Out-Null
}

$DestPath = Join-Path $Prefix "synapse.exe"
Copy-Item $Binary.FullName $DestPath -Force

Write-Host ">>> Installed to $DestPath" -ForegroundColor Green

# --- Add to PATH (user-level, persistent) ---
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$Prefix*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$Prefix", "User")
    Write-Host ">>> Added $Prefix to your PATH (restart your terminal to apply)." -ForegroundColor Yellow
} else {
    Write-Host ">>> $Prefix is already in your PATH." -ForegroundColor Green
}

# --- Verify ---
Write-Host ""
Write-Host "✓ synapse installed successfully!" -ForegroundColor Green
Write-Host ""
& $DestPath --version 2>$null
if ($LASTEXITCODE -ne 0) {
    & $DestPath --help 2>&1 | Select-Object -First 1
}
Write-Host ""
Write-Host "Run 'synapse --help' to get started." -ForegroundColor Cyan

# --- Cleanup ---
Remove-Item -Recurse -Force $TempDir
