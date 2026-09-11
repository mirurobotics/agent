<#
.SYNOPSIS
    Install a stable Miru Agent release from a trusted local MSI or GitHub.

.DESCRIPTION
    Requires elevated 64-bit Windows PowerShell. Before msiexec is started, the
    script verifies the package identity, UpgradeCode, x64 platform, and strict
    three-field MSI version. Downloaded files are also checked against the exact
    matching SHA-256 record. Checksums detect corruption; publisher authentication
    remains deferred until the release artifacts are Authenticode-signed.

.PARAMETER Version
    Stable version to install, such as "v0.10.3". A leading "v" is accepted.

.PARAMETER FromMsi
    Install a local MSI instead of downloading a release asset.
#>
[CmdletBinding()]
param(
    [string]$Version = "",
    [string]$FromMsi = ""
)

$ErrorActionPreference = "Stop"
$script:GitHubRepo = "mirurobotics/agent"
$script:MsiBaseName = "miru-agent"
$script:ExpectedProductName = "Miru Agent"
$script:ExpectedManufacturer = "Miru Robotics"
$script:ExpectedUpgradeCode = "{B5ED0336-5F14-4308-A667-3CE8CDEF7D48}"
$script:RequestTimeoutSeconds = 300

function Write-InstallLog {
    param([string]$Message)

    Write-Host "==> $Message" -ForegroundColor Green
}

function Assert-InstallAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Run this installer from an elevated Administrator PowerShell session."
    }
}

function Assert-InstallArchitecture {
    if (-not [Environment]::Is64BitOperatingSystem) {
        throw "The Miru Agent supports 64-bit Windows only."
    }
    if (-not [Environment]::Is64BitProcess) {
        throw "Run this installer from 64-bit PowerShell."
    }
}

function ConvertTo-MsiVersion {
    param(
        [Parameter(Mandatory = $true)][string]$Value,
        [switch]$AllowLeadingV
    )

    $candidate = $Value
    if ($AllowLeadingV -and $candidate.StartsWith("v", [StringComparison]::Ordinal)) {
        $candidate = $candidate.Substring(1)
    }
    if ($candidate -notmatch '^([0-9]+)\.([0-9]+)\.([0-9]+)$') {
        throw "Version must be a stable MAJOR.MINOR.PATCH value within MSI bounds."
    }

    try {
        $major = [uint32]::Parse($Matches[1], [Globalization.CultureInfo]::InvariantCulture)
        $minor = [uint32]::Parse($Matches[2], [Globalization.CultureInfo]::InvariantCulture)
        $patch = [uint32]::Parse($Matches[3], [Globalization.CultureInfo]::InvariantCulture)
    }
    catch {
        throw "Version must be a stable MAJOR.MINOR.PATCH value within MSI bounds."
    }

    if ($major -gt 255 -or $minor -gt 255 -or $patch -gt 65535) {
        throw "Version must be a stable MAJOR.MINOR.PATCH value within MSI bounds."
    }
    return $candidate
}

function Invoke-WithTls12 {
    param([Parameter(Mandatory = $true)][scriptblock]$Request)

    $originalProtocol = [Net.ServicePointManager]::SecurityProtocol
    try {
        [Net.ServicePointManager]::SecurityProtocol =
            $originalProtocol -bor [Net.SecurityProtocolType]::Tls12
        return & $Request
    }
    finally {
        [Net.ServicePointManager]::SecurityProtocol = $originalProtocol
    }
}

function Invoke-InstallWebRequest {
    param(
        [Parameter(Mandatory = $true)][string]$Uri,
        [string]$OutFile = ""
    )

    return Invoke-WithTls12 {
        if ($OutFile) {
            return Invoke-WebRequest -Uri $Uri -OutFile $OutFile -UseBasicParsing -TimeoutSec $script:RequestTimeoutSeconds
        }
        return Invoke-WebRequest -Uri $Uri -UseBasicParsing -TimeoutSec $script:RequestTimeoutSeconds
    }
}

function New-InstallTempDirectory {
    $randomBytes = New-Object byte[] 16
    $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
    try {
        $generator.GetBytes($randomBytes)
    }
    finally {
        $generator.Dispose()
    }

    $directoryName = "miru-install-{0}" -f ([BitConverter]::ToString($randomBytes) -replace '-', '')
    $directory = Join-Path ([IO.Path]::GetTempPath()) $directoryName
    New-Item -ItemType Directory -Path $directory -ErrorAction Stop | Out-Null
    return $directory
}

function Get-ExpectedChecksum {
    param(
        [Parameter(Mandatory = $true)][string]$ChecksumPath,
        [Parameter(Mandatory = $true)][string]$AssetName
    )

    $matchingDigests = @()
    foreach ($line in [IO.File]::ReadAllLines($ChecksumPath)) {
        if ($line -notmatch '^\s*([0-9A-Fa-f]{64})[ \t]+\*?(.+?)\s*$') {
            continue
        }
        if (-not [string]::Equals($Matches[2], $AssetName, [StringComparison]::Ordinal)) {
            continue
        }
        $matchingDigests += $Matches[1].ToUpperInvariant()
    }

    if ($matchingDigests.Count -ne 1) {
        throw "Checksums must contain exactly one valid SHA-256 record for $AssetName."
    }
    return $matchingDigests[0]
}

function Assert-FileChecksum {
    param(
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string]$ChecksumPath,
        [Parameter(Mandatory = $true)][string]$AssetName
    )

    $expected = Get-ExpectedChecksum -ChecksumPath $ChecksumPath -AssetName $AssetName
    $actual = (Get-FileHash -Algorithm SHA256 -Path $FilePath).Hash.ToUpperInvariant()
    if (-not [string]::Equals($actual, $expected, [StringComparison]::Ordinal)) {
        throw "Checksum verification failed for $AssetName (expected $expected, got $actual)."
    }
}

function Get-MsiProperty {
    param(
        [Parameter(Mandatory = $true)]$Database,
        [Parameter(Mandatory = $true)][string]$Name
    )

    $query = "SELECT ``Value`` FROM ``Property`` WHERE ``Property``='$Name'"
    $view = $Database.GetType().InvokeMember("OpenView", "InvokeMethod", $null, $Database, @($query))
    $record = $null
    try {
        $view.GetType().InvokeMember("Execute", "InvokeMethod", $null, $view, $null) | Out-Null
        $record = $view.GetType().InvokeMember("Fetch", "InvokeMethod", $null, $view, $null)
        if ($null -eq $record) {
            return $null
        }
        return $record.GetType().InvokeMember("StringData", "GetProperty", $null, $record, @(1))
    }
    finally {
        if ($null -ne $record) {
            [Runtime.InteropServices.Marshal]::ReleaseComObject($record) | Out-Null
        }
        if ($null -ne $view) {
            $view.GetType().InvokeMember("Close", "InvokeMethod", $null, $view, $null) | Out-Null
            [Runtime.InteropServices.Marshal]::ReleaseComObject($view) | Out-Null
        }
    }
}

function Get-MsiMetadata {
    param([Parameter(Mandatory = $true)][string]$Path)

    $installer = $null
    $database = $null
    $summary = $null
    try {
        $installer = New-Object -ComObject WindowsInstaller.Installer
        $database = $installer.GetType().InvokeMember("OpenDatabase", "InvokeMethod", $null, $installer, @($Path, 0))
        $summary = $database.GetType().InvokeMember("SummaryInformation", "GetProperty", $null, $database, @(0))
        $template = $summary.GetType().InvokeMember("Property", "GetProperty", $null, $summary, @(7))

        return [pscustomobject]@{
            ProductName = Get-MsiProperty -Database $database -Name "ProductName"
            Manufacturer = Get-MsiProperty -Database $database -Name "Manufacturer"
            ProductVersion = Get-MsiProperty -Database $database -Name "ProductVersion"
            ProductCode = Get-MsiProperty -Database $database -Name "ProductCode"
            UpgradeCode = Get-MsiProperty -Database $database -Name "UpgradeCode"
            Template = $template
        }
    }
    finally {
        if ($null -ne $summary) {
            [Runtime.InteropServices.Marshal]::ReleaseComObject($summary) | Out-Null
        }
        if ($null -ne $database) {
            [Runtime.InteropServices.Marshal]::ReleaseComObject($database) | Out-Null
        }
        if ($null -ne $installer) {
            [Runtime.InteropServices.Marshal]::ReleaseComObject($installer) | Out-Null
        }
    }
}

function Assert-MiruMsiMetadata {
    param(
        [Parameter(Mandatory = $true)]$Metadata,
        [string]$ExpectedVersion = ""
    )

    if (-not [string]::Equals($Metadata.ProductName, $script:ExpectedProductName, [StringComparison]::Ordinal)) {
        throw "MSI ProductName is not '$script:ExpectedProductName'."
    }
    if (-not [string]::Equals($Metadata.Manufacturer, $script:ExpectedManufacturer, [StringComparison]::Ordinal)) {
        throw "MSI Manufacturer is not '$script:ExpectedManufacturer'."
    }
    if (-not [string]::Equals($Metadata.UpgradeCode, $script:ExpectedUpgradeCode, [StringComparison]::OrdinalIgnoreCase)) {
        throw "MSI UpgradeCode does not identify the Miru Agent."
    }
    if ($Metadata.ProductCode -notmatch '^\{[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\}$') {
        throw "MSI ProductCode is not a valid product GUID."
    }
    if ($Metadata.Template -notmatch '(^|;)x64($|;)') {
        throw "MSI platform is not x64."
    }

    $packageVersion = ConvertTo-MsiVersion -Value $Metadata.ProductVersion
    if ($ExpectedVersion -and -not [string]::Equals($packageVersion, $ExpectedVersion, [StringComparison]::Ordinal)) {
        throw "MSI version $packageVersion does not match requested version $ExpectedVersion."
    }
    return $packageVersion
}

function Test-MsiProductInstalled {
    param([Parameter(Mandatory = $true)][string]$ProductCode)

    $installer = $null
    try {
        $installer = New-Object -ComObject WindowsInstaller.Installer
        $state = $installer.GetType().InvokeMember("ProductState", "GetProperty", $null, $installer, @($ProductCode))
        return [int]$state -eq 5
    }
    finally {
        if ($null -ne $installer) {
            [Runtime.InteropServices.Marshal]::ReleaseComObject($installer) | Out-Null
        }
    }
}

function Invoke-MsiInstall {
    param(
        [Parameter(Mandatory = $true)][string]$MsiPath,
        [Parameter(Mandatory = $true)][string]$LogPath,
        [Parameter(Mandatory = $true)][string]$ProductCode
    )

    $arguments = @(
        "/i",
        ('"{0}"' -f $MsiPath),
        "/qn",
        "/norestart",
        "/l*v",
        ('"{0}"' -f $LogPath)
    )
    if (Test-MsiProductInstalled -ProductCode $ProductCode) {
        $arguments += @("REINSTALL=ALL", "REINSTALLMODE=vomus")
    }
    $process = Start-Process -FilePath "msiexec.exe" -Wait -PassThru -ArgumentList $arguments
    return $process.ExitCode
}

function Get-LatestStableVersion {
    $response = Invoke-InstallWebRequest -Uri "https://api.github.com/repos/$script:GitHubRepo/releases/latest"
    $release = $response.Content | ConvertFrom-Json
    return ConvertTo-MsiVersion -Value $release.tag_name -AllowLeadingV
}

function Invoke-InstallMain {
    param(
        [string]$RequestedVersion = "",
        [string]$LocalMsi = ""
    )

    Assert-InstallAdministrator
    Assert-InstallArchitecture

    $normalizedVersion = ""
    if ($RequestedVersion) {
        $normalizedVersion = ConvertTo-MsiVersion -Value $RequestedVersion -AllowLeadingV
    }

    $downloadDirectory = $null
    try {
        if ($LocalMsi) {
            if (-not (Test-Path -LiteralPath $LocalMsi -PathType Leaf)) {
                throw "Provided MSI does not exist: $LocalMsi"
            }
            $msiPath = (Resolve-Path -LiteralPath $LocalMsi).Path
        }
        else {
            if (-not $normalizedVersion) {
                Write-InstallLog "Fetching the latest stable release version"
                $normalizedVersion = Get-LatestStableVersion
            }

            $downloadDirectory = New-InstallTempDirectory
            $assetName = "${script:MsiBaseName}_${normalizedVersion}_amd64.msi"
            $msiPath = Join-Path $downloadDirectory $assetName
            $checksumsPath = Join-Path $downloadDirectory "checksums.txt"
            $releaseBase = "https://github.com/$script:GitHubRepo/releases/download/v$normalizedVersion"

            Write-InstallLog "Downloading $assetName"
            Invoke-InstallWebRequest -Uri "$releaseBase/$assetName" -OutFile $msiPath | Out-Null
            Invoke-InstallWebRequest -Uri "$releaseBase/agent_${normalizedVersion}_checksums.txt" -OutFile $checksumsPath | Out-Null
            Assert-FileChecksum -FilePath $msiPath -ChecksumPath $checksumsPath -AssetName $assetName
            Write-InstallLog "Checksum verified"
        }

        $metadata = Get-MsiMetadata -Path $msiPath
        $packageVersion = Assert-MiruMsiMetadata -Metadata $metadata -ExpectedVersion $normalizedVersion
        Write-InstallLog "Installing Miru Agent $packageVersion"

        $logPath = Join-Path ([IO.Path]::GetTempPath()) "miru-agent-install-$([Guid]::NewGuid().ToString('N')).log"
        $exitCode = Invoke-MsiInstall -MsiPath $msiPath -LogPath $logPath -ProductCode $metadata.ProductCode
        if ($exitCode -eq 0) {
            Remove-Item -LiteralPath $logPath -Force -ErrorAction SilentlyContinue
            Write-InstallLog "Miru Agent installed. Provision it with provision.ps1."
            return 0
        }
        if ($exitCode -eq 3010) {
            Remove-Item -LiteralPath $logPath -Force -ErrorAction SilentlyContinue
            Write-InstallLog "Miru Agent installed; Windows must be restarted to complete the installation."
            return 3010
        }
        throw "msiexec failed with exit code $exitCode. The verbose log was retained at $logPath"
    }
    finally {
        if ($downloadDirectory -and (Test-Path -LiteralPath $downloadDirectory)) {
            Remove-Item -LiteralPath $downloadDirectory -Recurse -Force
        }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        exit (Invoke-InstallMain -RequestedVersion $Version -LocalMsi $FromMsi)
    }
    catch {
        Write-Host "Error: $($_.Exception.Message)" -ForegroundColor Red
        exit 1
    }
}
