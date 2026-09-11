<#
.SYNOPSIS
    Install the Miru Agent on Windows from a released MSI.

.DESCRIPTION
    Windows parity for scripts/install/install.sh. Downloads the Miru Agent MSI
    for a given version (or the latest release), verifies its SHA-256 checksum
    against the published checksums file, and installs it silently via msiexec.
    MSI upgrade semantics (MajorUpgrade in miru-agent.wxs) handle replacing an
    existing install.

    SCAFFOLDING — not yet exercised end-to-end. Depends on the MSI and checksums
    being published as release assets, which requires the deferred Windows build
    lane (see build/windows/README.md).

.PARAMETER Version
    Semantic version to install, e.g. "v0.10.3". Defaults to the latest release.

.PARAMETER Prerelease
    Install the latest prerelease instead of the latest stable release.

.PARAMETER FromMsi
    Install from a local .msi path instead of downloading (parity with --from-pkg).

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File install.ps1 -Version v0.10.3
#>
[CmdletBinding()]
param(
    [string]$Version = "",
    [switch]$Prerelease,
    [string]$FromMsi = ""
)

$ErrorActionPreference = "Stop"
$GitHubRepo = "mirurobotics/agent"
$MsiName = "miru-agent"

function Write-Log { param($m) Write-Host "==> $m" -ForegroundColor Green }
function Die { param($m) Write-Host "Error: $m" -ForegroundColor Red; exit 1 }

# Windows x64 only for now (matches the msvc build target).
if (-not [Environment]::Is64BitOperatingSystem) {
    Die "The Miru Agent Windows build supports 64-bit Windows only."
}

# Resolve the MSI: either a caller-provided local file or a downloaded release.
if ($FromMsi) {
    if (-not (Test-Path $FromMsi)) { Die "Provided MSI does not exist: $FromMsi" }
    $msiPath = (Resolve-Path $FromMsi).Path
    Write-Log "Installing from local MSI: $msiPath"
} else {
    # Determine the version to install.
    if (-not $Version) {
        Write-Log "Fetching latest $(if ($Prerelease) {'prerelease'} else {'release'}) version..."
        $releases = Invoke-RestMethod "https://api.github.com/repos/$GitHubRepo/releases"
        $Version = if ($Prerelease) {
            ($releases | Where-Object { $_.prerelease } | Select-Object -First 1).tag_name
        } else {
            (Invoke-RestMethod "https://api.github.com/repos/$GitHubRepo/releases/latest").tag_name
        }
    }
    if (-not $Version) { Die "Could not determine the version to install." }
    $ver = $Version.TrimStart("v")
    Write-Log "Version to install: $ver"

    $downloadDir = Join-Path $env:TEMP "miru-install"
    New-Item -ItemType Directory -Force -Path $downloadDir | Out-Null
    $msiFile = "${MsiName}_${ver}_amd64.msi"
    $msiPath = Join-Path $downloadDir $msiFile
    $base = "https://github.com/$GitHubRepo/releases/download/v$ver"

    Write-Log "Downloading $msiFile"
    Invoke-WebRequest "$base/$msiFile" -OutFile $msiPath

    # Verify SHA-256 against the published checksums file (parity with install.sh).
    $checksumsPath = Join-Path $downloadDir "checksums.txt"
    Invoke-WebRequest "$base/agent_${ver}_checksums.txt" -OutFile $checksumsPath
    $expected = (Select-String -Path $checksumsPath -Pattern ([regex]::Escape($msiFile)) |
        Select-Object -First 1).Line -split '\s+' | Select-Object -First 1
    if (-not $expected) { Die "No checksum found for $msiFile" }
    $actual = (Get-FileHash -Algorithm SHA256 -Path $msiPath).Hash
    if ($actual -ne $expected.ToUpper()) {
        Die "Checksum verification failed for $msiFile (expected $expected, got $actual)"
    }
    Write-Log "Checksum verified"
}

# Silent install. msiexec exit 3010 = success, reboot required.
Write-Log "Installing the Miru Agent (msiexec)"
$logFile = Join-Path $env:TEMP "miru-agent-install.log"
$p = Start-Process msiexec.exe -Wait -PassThru -ArgumentList @(
    "/i", "`"$msiPath`"", "/qn", "/norestart", "/l*v", "`"$logFile`""
)
if ($p.ExitCode -ne 0 -and $p.ExitCode -ne 3010) {
    Die "msiexec failed with exit code $($p.ExitCode). See $logFile"
}
Write-Log "Miru Agent installed. Provision it with provision.ps1."
