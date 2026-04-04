$ErrorActionPreference = "Stop"

$repo = "mfolk77/forge"
$installDir = if ($env:FORGE_INSTALL_DIR) { $env:FORGE_INSTALL_DIR } else { "$env:USERPROFILE\.local\bin" }
$asset = "forge-windows-x86_64.zip"

# Get latest release
$latestUrl = "https://api.github.com/repos/$repo/releases/latest"
try {
    $release = Invoke-RestMethod -Uri $latestUrl -Headers @{ "User-Agent" = "forge-installer" }
    $tag = $release.tag_name
} catch {
    Write-Error "Could not fetch latest release. Check your internet connection."
    exit 1
}

$downloadUrl = "https://github.com/$repo/releases/download/$tag/$asset"

Write-Host "Installing Forge $tag for Windows x86_64..."
Write-Host "  From: $downloadUrl"
Write-Host "  To:   $installDir\forge.exe"

# Create install directory
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

# Download
$tmpDir = New-TemporaryFile | ForEach-Object { Remove-Item $_; New-Item -ItemType Directory -Path $_ }
$zipPath = Join-Path $tmpDir $asset

Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath

# Extract
Expand-Archive -Path $zipPath -DestinationPath $tmpDir -Force

# Move binary
$binary = Get-ChildItem -Path $tmpDir -Filter "forge-windows*" -Recurse | Select-Object -First 1
if ($binary) {
    Copy-Item $binary.FullName "$installDir\forge.exe" -Force
} else {
    Write-Error "Binary not found in archive"
    exit 1
}

# Cleanup
Remove-Item -Recurse -Force $tmpDir

Write-Host ""
Write-Host "Forge installed to $installDir\forge.exe"

# Check PATH
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    Write-Host ""
    Write-Host "Adding $installDir to your PATH..."
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    Write-Host "Restart your terminal for PATH changes to take effect."
}

Write-Host ""
Write-Host "Run 'forge --version' to verify."
