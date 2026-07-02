# EAiCoding MCP service automated build and packaging script (Windows PowerShell)
# Purpose: Build the Rust MCP service and package it into a green portable distribution with eagent-tools

$ErrorActionPreference = "Stop"

Write-Host "=============================================" -ForegroundColor Cyan
Write-Host "  Starting Build & Package for EAiCoding MCP" -ForegroundColor Cyan
Write-Host "=============================================" -ForegroundColor Cyan

# 1. Check Rust environment
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error "Cargo command not found. Please ensure Rust is installed."
}

# 2. Execute Cargo Release Build
Write-Host "`n[1/4] Running cargo build --release..." -ForegroundColor Yellow
cargo build --release

$exePath = "target\release\eaicoding-mcp.exe"
if (-not (Test-Path $exePath)) {
    Write-Error "Build failed, target file not found: $exePath"
}
Write-Host "Cargo release build finished successfully!" -ForegroundColor Green

# 3. Create release output directory
Write-Host "`n[2/4] Creating release directory and gathering dependencies..." -ForegroundColor Yellow
$releaseDir = "release\eaicoding-mcp-portable"
if (Test-Path $releaseDir) {
    Remove-Item -Path $releaseDir -Recurse -Force
}
New-Item -ItemType Directory -Path $releaseDir | Out-Null

# Copy executable
Copy-Item -Path $exePath -Destination $releaseDir

# Copy bundled tools
$toolsSrc = "resources\eagent-tools"
if (Test-Path $toolsSrc) {
    $toolsDest = Join-Path $releaseDir "eagent-tools"
    Copy-Item -Path $toolsSrc -Destination $toolsDest -Recurse -Force
    Write-Host "Successfully copied local eagent-tools." -ForegroundColor Green
} else {
    Write-Host "Warning: resources\eagent-tools folder not found." -ForegroundColor Yellow
    Write-Host "New users will need to run setup_env tool to download compiler dependencies." -ForegroundColor Magenta
}

# 4. Generate Zip Archive
Write-Host "`n[3/4] Packaging output directory to ZIP archive..." -ForegroundColor Yellow
$zipOutput = "release\eaicoding-mcp-portable.zip"
if (Test-Path $zipOutput) {
    Remove-Item -Path $zipOutput -Force
}

# Compress directory
Compress-Archive -Path "$releaseDir\*" -DestinationPath $zipOutput -Force
Write-Host "Distribution ZIP generated: $zipOutput" -ForegroundColor Green

# 5. Output Checksum & Verification
Write-Host "`n[4/4] Calculating file SHA256 checksums..." -ForegroundColor Yellow
$exeHash = Get-FileHash -Path "$releaseDir\eaicoding-mcp.exe" -Algorithm SHA256
$zipHash = Get-FileHash -Path $zipOutput -Algorithm SHA256

$exeSize = (Get-Item "$releaseDir\eaicoding-mcp.exe").Length / 1MB
$zipSize = (Get-Item $zipOutput).Length / 1MB

Write-Host "`n================ Package Summary ================" -ForegroundColor Cyan
Write-Host ("Main Executable (eaicoding-mcp.exe):")
Write-Host ("  Size: {0:N2} MB" -f $exeSize)
Write-Host ("  SHA256: " + $exeHash.Hash)
Write-Host ("Distribution Archive (eaicoding-mcp-portable.zip):")
Write-Host ("  Size: {0:N2} MB" -f $zipSize)
Write-Host ("  SHA256: " + $zipHash.Hash)
Write-Host "=================================================" -ForegroundColor Cyan
Write-Host "`nInfo: You can now distribute the eaicoding-mcp-portable folder or ZIP to users." -ForegroundColor Green
