#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$ProjectPath,
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$BinDir,
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$ArtifactsDirectory
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
Import-Module -Force -Name (Join-Path $PSScriptRoot "MsiTest.psm1")

function Remove-AndCreateChild {
    param(
        [Parameter(Mandatory = $true)][string]$Parent,
        [Parameter(Mandatory = $true)][string]$Child
    )
    $path = Join-Path $Parent $Child
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Recurse -Force
    }
    New-Item -ItemType Directory -Path $path | Out-Null
    return (Resolve-Path $path).Path
}

function Get-SequenceNumber {
    param(
        [Parameter(Mandatory = $true)][object[]]$Sequence,
        [Parameter(Mandatory = $true)][string]$Action
    )
    return [int](@($Sequence | Where-Object { $_[0] -eq $Action })[0][2])
}

# ---------------------------------------------------------------------------
# Production table assertions, one concern each
# ---------------------------------------------------------------------------

function Assert-RetainedDirectoryComponent {
    param(
        [Parameter(Mandatory = $true)][object[]]$Expected,
        [Parameter(Mandatory = $true)][object[]]$Components,
        [Parameter(Mandatory = $true)][object[]]$Directories,
        [Parameter(Mandatory = $true)][object[]]$Folders
    )
    $name, $guid, $directory, $parent, $leaf = $Expected
    $retained = @($Components | Where-Object { $_[0] -eq $name })
    Assert-Equal 1 $retained.Count "retained component $name"
    Assert-Equal $guid $retained[0][1].ToUpperInvariant() "$name stable component identity"
    Assert-Equal $directory $retained[0][2] "$name component directory"
    Assert-True (([int]$retained[0][3] -band 256) -ne 0) "$name is 64-bit"
    Assert-True ([string]::IsNullOrEmpty($retained[0][4])) "$name has a directory key path"
    Assert-Equal 1 (@($Directories | Where-Object { $_[0] -eq $directory -and $_[1] -eq $parent -and $_[2] -eq $leaf })).Count "$name directory hierarchy"
    Assert-Equal 1 (@($Folders | Where-Object { $_[0] -eq $directory -and $_[1] -eq $name })).Count "$name CreateFolder mapping"
}

function Assert-DirectoryComponents {
    param([Parameter(Mandatory = $true)]$Database)
    $components = @(Get-MsiRows $Database "SELECT ``Component``, ``ComponentId``, ``Directory_``, ``Attributes``, ``KeyPath`` FROM ``Component``" 5)
    $directories = @(Get-MsiRows $Database "SELECT ``Directory``, ``Directory_Parent``, ``DefaultDir`` FROM ``Directory``" 3)
    $folders = @(Get-MsiRows $Database "SELECT ``Directory_``, ``Component_`` FROM ``CreateFolder``" 2)
    foreach ($expected in $MsiExpectedDirectories) {
        Assert-RetainedDirectoryComponent $expected $components $directories $folders
    }
    $binary = @($components | Where-Object { $_[0] -eq "MiruAgentExe" })
    Assert-Equal 1 $binary.Count "binary component"
    Assert-True (([int]$binary[0][3] -band 256) -ne 0) "binary component is 64-bit"
    Assert-Equal "AGENTFOLDER" $binary[0][2] "binary component directory"
    Assert-Equal "miru_agent.exe" $binary[0][4] "binary component file key path"
    Assert-Equal 1 (@($directories | Where-Object { $_[0] -eq "INSTALLFOLDER" -and $_[1] -eq "ProgramFiles64Folder" -and $_[2] -eq "Miru" })).Count "64-bit install directory"
}

function Assert-ProtectedPermissionRows {
    param([Parameter(Mandatory = $true)]$Database)
    $permissions = @(Get-MsiRows $Database "SELECT ``LockObject``, ``Table``, ``SDDLText`` FROM ``MsiLockPermissionsEx``" 3)
    $expected = @($MsiExpectedDirectories | ForEach-Object { "$($_[2])|CreateFolder|$MsiExpectedSddl" } | Sort-Object)
    $actual = @($permissions | ForEach-Object { "{0}|{1}|{2}" -f $_[0], $_[1], $_[2] } | Sort-Object)
    Assert-Equal ($expected -join "`n") ($actual -join "`n") "exact protected permission rows"
}

function Assert-FixtureIsolation {
    param([Parameter(Mandatory = $true)]$Database)
    $actions = @()
    if (Test-MsiTable $Database "CustomAction") {
        $actions = @(Get-MsiRows $Database "SELECT ``Action``, ``Type``, ``Source``, ``Target`` FROM ``CustomAction``" 4)
    }
    Assert-Equal 0 (@($actions | Where-Object { ($_[0] + $_[2] + $_[3]) -match 'FailUpgradeForTest|FAIL_UPGRADE_FOR_TEST|rollback-payload' })).Count "production custom action isolation"
    $sequence = @(Get-MsiRows $Database "SELECT ``Action``, ``Condition``, ``Sequence`` FROM ``InstallExecuteSequence``" 3)
    Assert-Equal 0 (@($sequence | Where-Object { ($_[0] + $_[1]) -match 'FailUpgradeForTest|FAIL_UPGRADE_FOR_TEST' })).Count "production sequence isolation"
    $files = @(Get-MsiRows $Database "SELECT ``FileName`` FROM ``File``" 1)
    Assert-Equal 0 (@($files | Where-Object { $_[0] -match 'rollback|fixture|test' })).Count "no test fixture payload"
}

function Assert-TransactionalMajorUpgrade {
    param([Parameter(Mandatory = $true)]$Database)
    $sequence = @(Get-MsiRows $Database "SELECT ``Action``, ``Condition``, ``Sequence`` FROM ``InstallExecuteSequence``" 3)
    Assert-Equal 1 (@($sequence | Where-Object { $_[0] -eq "RemoveExistingProducts" })).Count "major upgrade removes existing product"
    $remove = Get-SequenceNumber $sequence "RemoveExistingProducts"
    $initialize = Get-SequenceNumber $sequence "InstallInitialize"
    $finalize = Get-SequenceNumber $sequence "InstallFinalize"
    Assert-True ($remove -gt $initialize -and $remove -lt $finalize) "RemoveExistingProducts is transactional"
    $upgradeRows = @(Get-MsiRows $Database "SELECT ``UpgradeCode``, ``VersionMin``, ``VersionMax``, ``Attributes``, ``ActionProperty`` FROM ``Upgrade``" 5)
    Assert-True ($upgradeRows.Count -ge 2) "upgrade and downgrade rows exist"
}

function Assert-DowngradeLaunchCondition {
    param([Parameter(Mandatory = $true)]$Database)
    $conditions = @(Get-MsiRows $Database "SELECT ``Condition``, ``Description`` FROM ``LaunchCondition``" 2)
    $downgrade = @($conditions | Where-Object { $_[0] -match 'WIX_DOWNGRADE_DETECTED' })
    Assert-Equal 1 $downgrade.Count "downgrade launch condition"
    Assert-Equal "A newer version of Miru Agent is already installed." $downgrade[0][1] "downgrade launch condition description"
}

function Assert-NoServiceTables {
    param([Parameter(Mandatory = $true)]$Database)
    Assert-True (-not (Test-MsiTable $Database "ServiceInstall")) "no service installation table"
    Assert-True (-not (Test-MsiTable $Database "ServiceControl")) "no service control table"
}

function Assert-ProductionTables {
    param([Parameter(Mandatory = $true)][string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    try {
        Assert-DirectoryComponents $handle.Database
        Assert-ProtectedPermissionRows $handle.Database
        Assert-FixtureIsolation $handle.Database
        Assert-TransactionalMajorUpgrade $handle.Database
        Assert-DowngradeLaunchCondition $handle.Database
        Assert-NoServiceTables $handle.Database
    }
    finally { Close-MsiDatabase $handle }
}

function Assert-Package {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Version
    )
    $metadata = Get-MsiContract -Path $Path
    Assert-Equal $MsiProductName $metadata.ProductName "ProductName"
    Assert-Equal $MsiManufacturer $metadata.Manufacturer "Manufacturer"
    Assert-Equal $Version $metadata.ProductVersion "ProductVersion"
    Assert-Equal $MsiUpgradeCode $metadata.UpgradeCode "UpgradeCode"
    Assert-True ($metadata.ProductCode -match '^\{[0-9A-Fa-f-]{36}\}$') "ProductCode"
    Assert-True ($metadata.Template -match '(^|;)x64($|;)') "x64 summary template"
    Assert-Equal 500 ([int]$metadata.InstallerVersion) "InstallerVersion"
    Assert-Equal "1" $metadata.ALLUSERS "per-machine package"
    return $metadata
}

# ---------------------------------------------------------------------------
# Builds
# ---------------------------------------------------------------------------

function Build-ProductionPackage {
    param(
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)][string]$Directory
    )
    $built = Invoke-DotNetBuild -ProjectPath $resolvedProject -BinDir $resolvedBinDir -Version $Version -OutputDirectory $Directory
    $path = Join-Path $Directory "miru-agent-$Version.msi"
    if ($built -ne $path) { Copy-Item -LiteralPath $built -Destination $path -Force }
    $metadata = Assert-Package $path $Version
    Assert-ProductionTables $path
    Write-Host "PASS package $Version"
    return $metadata
}

function Build-FixturePackage {
    param([Parameter(Mandatory = $true)][string]$Directory)
    $payload = Join-Path $Directory "rollback-payload.txt"
    [IO.File]::WriteAllText($payload, "fixture-contract", [Text.Encoding]::ASCII)
    return Invoke-DotNetBuild -ProjectPath $resolvedProject -BinDir $resolvedBinDir -Version "1.2.0" `
        -OutputDirectory $Directory -ProductCode $MsiFixtureProductCodes[2] `
        -TestWixSource (Join-Path $PSScriptRoot "integration-test.wxs") -FixturePayloadPath $payload
}

function Invoke-ExpectedBuildFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$Properties,
        [Parameter(Mandatory = $true)][string]$ExpectedErrorCode
    )
    $outputPath = Join-Path $invalidDirectory $Name
    New-Item -ItemType Directory -Path $outputPath | Out-Null
    $arguments = @("build", $resolvedProject, "--no-restore", "--configuration", "Release", "-p:OutputPath=$outputPath\") + $Properties
    $output = @(& dotnet @arguments 2>&1)
    $exitCode = $LASTEXITCODE
    $outputText = $output | Out-String
    Assert-True ($exitCode -ne 0) "$Name build fails`n$outputText"
    Assert-True ($outputText -match "\b$([regex]::Escape($ExpectedErrorCode))\b") "$Name expected error code $ExpectedErrorCode`n$outputText"
    Assert-Equal 0 (@(Get-ChildItem -LiteralPath $outputPath -Filter "*.msi" -File -Recurse)).Count "$Name produces no MSI`n$outputText"
}

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

$resolvedProject = (Resolve-Path -LiteralPath $ProjectPath).Path
$resolvedBinDir = (Resolve-Path -LiteralPath $BinDir).Path
$resolvedArtifacts = [IO.Path]::GetFullPath($ArtifactsDirectory)
New-Item -ItemType Directory -Path $resolvedArtifacts -Force | Out-Null
$invalidDirectory = Remove-AndCreateChild $resolvedArtifacts "invalid"

$v1 = Build-ProductionPackage "1.0.0" (Remove-AndCreateChild $resolvedArtifacts "v1")
$v2 = Build-ProductionPackage "1.1.0" (Remove-AndCreateChild $resolvedArtifacts "v2")
Assert-True (-not [string]::Equals($v1.ProductCode, $v2.ProductCode, [StringComparison]::OrdinalIgnoreCase)) "normal ProductCodes differ"
Assert-Equal $v1.UpgradeCode $v2.UpgradeCode "normal UpgradeCode remains stable"
Write-Host "PASS ProductCodes differ, UpgradeCode stable"

Assert-FailingFixtureContract (Build-FixturePackage (Remove-AndCreateChild $resolvedArtifacts "fixture"))
Write-Host "PASS fixture custom-action contract"

# One representative case per validation family proves the wixproj error
# mechanism works; the remaining MIRUMSI codes are three-line MSBuild checks.
$common = @("-p:BinDir=$resolvedBinDir", "-p:Platform=x64")
Invoke-ExpectedBuildFailure "prerelease-version" ($common + "-p:Version=1.2.3-beta.1") "MIRUMSI1002"
Invoke-ExpectedBuildFailure "omitted-bindir" @("-p:Version=1.2.3", "-p:Platform=x64") "MIRUMSI1003"
Invoke-ExpectedBuildFailure "missing-fixture-payload" ($common + @("-p:Version=1.2.3", "-p:TestWixSource=integration-test.wxs")) "MIRUMSI1009"
Write-Host "PASS invalid inputs rejected (3 cases)"
