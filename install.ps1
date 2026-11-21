<#
.SYNOPSIS
    Install the pu-get release binary on Windows.

.DESCRIPTION
    Downloads a release archive, verifies it against the release's
    SHA256SUMS.txt, and installs pu-get.exe for the current user.

    HTTPS is required for GitHub downloads; plain HTTP is accepted only for a
    localhost mirror.

.EXAMPLE
    irm https://raw.githubusercontent.com/dumbramkrishna/pu-get/main/install.ps1 | iex

.EXAMPLE
    .\install.ps1 -Version v0.0.1 -InstallDir "$HOME\.local\bin" -AddToPath
#>
[CmdletBinding()]
param(
    [string]$Version = $env:PU_GET_VERSION,
    [string]$InstallDir = $env:PU_GET_INSTALL_DIR,
    [switch]$Force,
    [switch]$DryRun,
    [switch]$AddToPath
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repo = if ($env:PU_GET_REPO) { $env:PU_GET_REPO } else { "dumbramkrishna/pu-get" }
$ApiBase = if ($env:PU_GET_API_BASE) { $env:PU_GET_API_BASE } else { "https://api.github.com" }
$DownloadBase = if ($env:PU_GET_DOWNLOAD_BASE) { $env:PU_GET_DOWNLOAD_BASE } else { "https://github.com" }
$UserAgent = @{ "User-Agent" = "pu-get-installer" }

function Assert-AllowedUrl {
    param([Parameter(Mandatory = $true)][string]$Url)

    if ($Url -match '^https://') {
        return
    }
    if ($Url -match '^http://(localhost|127\.0\.0\.1)(:\d+)?/') {
        return
    }
    throw "refusing non-HTTPS URL: $Url"
}

function Get-RemoteText {
    param([Parameter(Mandatory = $true)][string]$Url)

    Assert-AllowedUrl $Url
    (Invoke-WebRequest -Uri $Url -UseBasicParsing -Headers $UserAgent).Content
}

function Save-RemoteFile {
    param(
        [Parameter(Mandatory = $true)][string]$Url,
        [Parameter(Mandatory = $true)][string]$Path
    )

    Assert-AllowedUrl $Url
    try {
        Invoke-WebRequest -Uri $Url -OutFile $Path -UseBasicParsing -Headers $UserAgent
    }
    catch {
        throw "download failed: $Url`n$($_.Exception.Message)"
    }
}

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
}
catch {
    # PowerShell 7 already negotiates a modern protocol.
}

if (-not [Environment]::Is64BitOperatingSystem) {
    throw "pu-get requires 64-bit Windows"
}

$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
if ($architecture -ne [System.Runtime.InteropServices.Architecture]::X64) {
    throw "unsupported Windows architecture: $architecture. Only x86_64 builds are published."
}
$Target = "x86_64-pc-windows-msvc"

if ([string]::IsNullOrWhiteSpace($Version)) {
    Write-Host "Looking up the newest pu-get release..."
    $releaseApi = "$ApiBase/repos/$Repo/releases?per_page=1"
    Assert-AllowedUrl $releaseApi
    try {
        $releases = Invoke-RestMethod -Uri $releaseApi -Headers $UserAgent
    }
    catch {
        throw "could not query GitHub releases; pass -Version v0.0.1 to skip the lookup`n$($_.Exception.Message)"
    }

    $releases = @($releases)
    if ($releases.Count -eq 0 -or [string]::IsNullOrWhiteSpace($releases[0].tag_name)) {
        throw "no published release found; pass -Version v0.0.1"
    }
    $Version = $releases[0].tag_name
}

if (-not $Version.StartsWith("v")) {
    $Version = "v$Version"
}
if ($Version -notmatch '^[A-Za-z0-9._-]+$') {
    throw "invalid version: $Version"
}

if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    if ($env:LOCALAPPDATA) {
        $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\pu-get\bin"
    }
    elseif ($HOME) {
        $InstallDir = Join-Path $HOME ".local\bin"
    }
    else {
        throw "could not determine an install directory; pass -InstallDir DIR"
    }
}

$asset = "pu-get-$Version-$Target.zip"
$releaseUrl = "$DownloadBase/$Repo/releases/download/$Version/$asset"
$checksumUrl = "$DownloadBase/$Repo/releases/download/$Version/SHA256SUMS.txt"
$destination = Join-Path $InstallDir "pu-get.exe"
$displayVersion = $Version -replace '^v', ''

if ($DryRun) {
    Write-Host "Would install pu-get $displayVersion"
    Write-Host "  platform:  $Target"
    Write-Host "  archive:   $releaseUrl"
    Write-Host "  checksum:  $checksumUrl"
    Write-Host "  install:   $destination"
    return
}

if ((Test-Path -LiteralPath $destination -PathType Leaf) -and -not $Force) {
    try {
        $installed = & $destination --version 2>$null
    }
    catch {
        $installed = $null
    }
    if ($installed -eq "pu-get $displayVersion") {
        Write-Host "pu-get $displayVersion is already installed at $destination"
        Write-Host "Use -Force to reinstall."
        return
    }
}

$temp = Join-Path ([IO.Path]::GetTempPath()) ("pu-get-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $temp -Force | Out-Null

try {
    $archive = Join-Path $temp $asset
    Write-Host "Downloading pu-get $displayVersion for $Target..."
    Save-RemoteFile -Url $releaseUrl -Path $archive

    Write-Host "Verifying SHA-256 checksum..."
    $sums = (Get-RemoteText -Url $checksumUrl) -split "`r?`n"
    $pattern = '\s+\*?' + [regex]::Escape($asset) + '$'
    $line = $sums | Where-Object { $_ -match $pattern } | Select-Object -First 1
    if (-not $line) {
        throw "SHA256SUMS.txt does not contain $asset"
    }

    $expected = (($line -split '\s+')[0]).ToLowerInvariant()
    $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) {
        throw "checksum mismatch for $asset (expected $expected, got $actual)"
    }

    Expand-Archive -LiteralPath $archive -DestinationPath $temp -Force
    $unpacked = Join-Path (Join-Path $temp ([IO.Path]::GetFileNameWithoutExtension($asset))) "pu-get.exe"
    if (-not (Test-Path -LiteralPath $unpacked -PathType Leaf)) {
        throw "archive did not contain pu-get.exe"
    }

    $reported = & $unpacked --version
    if ($LASTEXITCODE -ne 0 -or $reported -ne "pu-get $displayVersion") {
        throw "the downloaded binary failed to run on this system"
    }

    if (Test-Path -LiteralPath $InstallDir -PathType Leaf) {
        throw "$InstallDir exists and is not a directory"
    }
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Copy-Item -LiteralPath $unpacked -Destination $destination -Force

    Write-Host ""
    Write-Host "Installed pu-get $displayVersion to $destination"

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $pathParts = @()
    if ($userPath) {
        $pathParts = $userPath -split ';'
    }
    if ($pathParts -notcontains $InstallDir) {
        if ($AddToPath) {
            $newPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
            [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
            Write-Host "Added $InstallDir to the user PATH. Restart the terminal, then run: pu-get --version"
        }
        else {
            Write-Host ""
            Write-Host "$InstallDir is not in your PATH. Add it once, then restart the terminal:"
            Write-Host "  [Environment]::SetEnvironmentVariable('Path', [Environment]::GetEnvironmentVariable('Path', 'User') + ';$InstallDir', 'User')"
            Write-Host "Or re-run this installer with -AddToPath."
        }
    }
}
finally {
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
}
