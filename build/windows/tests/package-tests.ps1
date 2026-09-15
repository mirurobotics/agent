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

function Assert-ProductionTables {
    param([Parameter(Mandatory = $true)][string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    try {
        $components = @(Get-MsiRows $handle.Database "SELECT ``Component``, ``ComponentId``, ``Directory_``, ``Attributes``, ``KeyPath`` FROM ``Component``" 5)
        $directories = @(Get-MsiRows $handle.Database "SELECT ``Directory``, ``Directory_Parent``, ``DefaultDir`` FROM ``Directory``" 3)
        $folders = @(Get-MsiRows $handle.Database "SELECT ``Directory_``, ``Component_`` FROM ``CreateFolder``" 2)
        foreach ($expected in $MsiExpectedDirectories) {
            $name, $guid, $directory, $parent, $leaf = $expected
            $retained = @($components | Where-Object { $_[0] -eq $name })
            Assert-Equal 1 $retained.Count "retained component $name"
            Assert-Equal $guid $retained[0][1].ToUpperInvariant() "$name stable component identity"
            Assert-Equal $directory $retained[0][2] "$name component directory"
            Assert-True (([int]$retained[0][3] -band 256) -ne 0) "$name is 64-bit"
            Assert-True ([string]::IsNullOrEmpty($retained[0][4])) "$name has a directory key path"
            Assert-Equal 1 (@($directories | Where-Object { $_[0] -eq $directory -and $_[1] -eq $parent -and $_[2] -eq $leaf })).Count "$name directory hierarchy"
            Assert-Equal 1 (@($folders | Where-Object { $_[0] -eq $directory -and $_[1] -eq $name })).Count "$name CreateFolder mapping"
        }
        $binary = @($components | Where-Object { $_[0] -eq "MiruAgentExe" })
        Assert-Equal 1 $binary.Count "binary component"
        Assert-True (([int]$binary[0][3] -band 256) -ne 0) "binary component is 64-bit"
        Assert-Equal "AGENTFOLDER" $binary[0][2] "binary component directory"
        Assert-Equal "miru_agent.exe" $binary[0][4] "binary component file key path"
        Assert-Equal 1 (@($directories | Where-Object { $_[0] -eq "INSTALLFOLDER" -and $_[1] -eq "ProgramFiles64Folder" -and $_[2] -eq "Miru" })).Count "64-bit install directory"

        $permissions = @(Get-MsiRows $handle.Database "SELECT ``LockObject``, ``Table``, ``SDDLText`` FROM ``MsiLockPermissionsEx``" 3)
        $expectedPermissions = @($MsiExpectedDirectories | ForEach-Object { "$($_[2])|CreateFolder|$MsiExpectedSddl" } | Sort-Object)
        $actualPermissions = @($permissions | ForEach-Object { "{0}|{1}|{2}" -f $_[0], $_[1], $_[2] } | Sort-Object)
        Assert-Equal ($expectedPermissions -join "`n") ($actualPermissions -join "`n") "exact protected permission rows"

        $actions = @()
        if (Test-MsiTable $handle.Database "CustomAction") {
            $actions = @(Get-MsiRows $handle.Database "SELECT ``Action``, ``Type``, ``Source``, ``Target`` FROM ``CustomAction``" 4)
        }
        Assert-Equal 0 (@($actions | Where-Object { ($_[0] + $_[2] + $_[3]) -match 'FailUpgradeForTest|FAIL_UPGRADE_FOR_TEST|rollback-payload' })).Count "production custom action isolation"
        $sequence = @(Get-MsiRows $handle.Database "SELECT ``Action``, ``Condition``, ``Sequence`` FROM ``InstallExecuteSequence``" 3)
        Assert-Equal 0 (@($sequence | Where-Object { ($_[0] + $_[1]) -match 'FailUpgradeForTest|FAIL_UPGRADE_FOR_TEST' })).Count "production sequence isolation"
        $removeExisting = @($sequence | Where-Object { $_[0] -eq "RemoveExistingProducts" })
        Assert-Equal 1 $removeExisting.Count "major upgrade removes existing product"
        $initialize = [int](@($sequence | Where-Object { $_[0] -eq "InstallInitialize" })[0][2])
        $finalize = [int](@($sequence | Where-Object { $_[0] -eq "InstallFinalize" })[0][2])
        $removeSequence = [int]$removeExisting[0][2]
        Assert-True ($removeSequence -gt $initialize -and $removeSequence -lt $finalize) "RemoveExistingProducts is transactional"

        $upgradeRows = @(Get-MsiRows $handle.Database "SELECT ``UpgradeCode``, ``VersionMin``, ``VersionMax``, ``Attributes``, ``ActionProperty`` FROM ``Upgrade``" 5)
        Assert-True ($upgradeRows.Count -ge 2) "upgrade and downgrade rows exist"
        $launchConditions = @(Get-MsiRows $handle.Database "SELECT ``Condition``, ``Description`` FROM ``LaunchCondition``" 2)
        $downgradeConditions = @($launchConditions | Where-Object { $_[0] -match 'WIX_DOWNGRADE_DETECTED' })
        Assert-Equal 1 $downgradeConditions.Count "downgrade launch condition"
        Assert-Equal "A newer version of Miru Agent is already installed." $downgradeConditions[0][1] "downgrade launch condition description"
        Assert-True (-not (Test-MsiTable $handle.Database "ServiceInstall")) "no service installation table"
        Assert-True (-not (Test-MsiTable $handle.Database "ServiceControl")) "no service control table"
        $files = @(Get-MsiRows $handle.Database "SELECT ``FileName`` FROM ``File``" 1)
        Assert-Equal 0 (@($files | Where-Object { $_[0] -match 'rollback|fixture|test' })).Count "no test fixture payload"
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

function Invoke-ExpectedBuildFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$Properties,
        [Parameter(Mandatory = $true)][string]$ExpectedErrorCode
    )
    $outputPath = Join-Path $script:invalidDirectory $Name
    New-Item -ItemType Directory -Path $outputPath | Out-Null
    $arguments = @("build", $script:resolvedProject, "--no-restore", "--configuration", "Release", "-p:OutputPath=$outputPath\") + $Properties
    $output = @(& dotnet @arguments 2>&1)
    $exitCode = $LASTEXITCODE
    $outputText = $output | Out-String
    Assert-True ($exitCode -ne 0) "$Name build fails`n$outputText"
    Assert-True ($outputText -match "\b$([regex]::Escape($ExpectedErrorCode))\b") "$Name expected error code $ExpectedErrorCode`n$outputText"
    Assert-Equal 0 (@(Get-ChildItem -LiteralPath $outputPath -Filter "*.msi" -File -Recurse)).Count "$Name produces no MSI`n$outputText"
}

$resolvedProject = (Resolve-Path -LiteralPath $ProjectPath).Path
$resolvedBinDir = (Resolve-Path -LiteralPath $BinDir).Path
$resolvedArtifacts = [IO.Path]::GetFullPath($ArtifactsDirectory)
New-Item -ItemType Directory -Path $resolvedArtifacts -Force | Out-Null
$v1Directory = Remove-AndCreateChild $resolvedArtifacts "v1"
$v2Directory = Remove-AndCreateChild $resolvedArtifacts "v2"
$boundariesDirectory = Remove-AndCreateChild $resolvedArtifacts "boundaries"
$invalidDirectory = Remove-AndCreateChild $resolvedArtifacts "invalid"
$fixtureDirectory = Remove-AndCreateChild $resolvedArtifacts "fixture"

$v1Built = Invoke-DotNetBuild -ProjectPath $resolvedProject -BinDir $resolvedBinDir -Version "1.0.0" -OutputDirectory $v1Directory
$v1Path = Join-Path $v1Directory "miru-agent-1.0.0.msi"
if ($v1Built -ne $v1Path) { Copy-Item -LiteralPath $v1Built -Destination $v1Path -Force }
$v1 = Assert-Package $v1Path "1.0.0"
Assert-ProductionTables $v1Path
Write-Host "PASS package 1.0.0"

$v2Built = Invoke-DotNetBuild -ProjectPath $resolvedProject -BinDir $resolvedBinDir -Version "1.1.0" -OutputDirectory $v2Directory
$v2Path = Join-Path $v2Directory "miru-agent-1.1.0.msi"
if ($v2Built -ne $v2Path) { Copy-Item -LiteralPath $v2Built -Destination $v2Path -Force }
$v2 = Assert-Package $v2Path "1.1.0"
Assert-ProductionTables $v2Path
Write-Host "PASS package 1.1.0"

Assert-True (-not [string]::Equals($v1.ProductCode, $v2.ProductCode, [StringComparison]::OrdinalIgnoreCase)) "normal ProductCodes differ"
Write-Host "PASS ProductCodes differ"
Assert-Equal $v1.UpgradeCode $v2.UpgradeCode "normal UpgradeCode remains stable"
Write-Host "PASS UpgradeCode stable"

foreach ($version in @("0.0.0", "255.255.65535")) {
    $directory = Join-Path $boundariesDirectory $version
    New-Item -ItemType Directory -Path $directory | Out-Null
    $built = Invoke-DotNetBuild -ProjectPath $resolvedProject -BinDir $resolvedBinDir -Version $version -OutputDirectory $directory
    $destination = Join-Path $boundariesDirectory "miru-agent-$version.msi"
    Copy-Item -LiteralPath $built -Destination $destination -Force
    Assert-Package $destination $version | Out-Null
    Assert-ProductionTables $destination
}
Write-Host "PASS version boundaries (0.0.0, 255.255.65535)"

$fixturePayload = Join-Path $fixtureDirectory "rollback-payload.txt"
[IO.File]::WriteAllText($fixturePayload, "fixture-contract", [Text.Encoding]::ASCII)
$fixtureBuilt = Invoke-DotNetBuild -ProjectPath $resolvedProject -BinDir $resolvedBinDir -Version "1.2.0" `
    -OutputDirectory $fixtureDirectory -ProductCode $MsiFixtureProductCodes[2] `
    -TestWixSource (Join-Path $PSScriptRoot "integration-test.wxs") -FixturePayloadPath $fixturePayload
Assert-FailingFixtureContract $fixtureBuilt
Write-Host "PASS fixture custom-action contract"

$common = @("-p:BinDir=$resolvedBinDir", "-p:Platform=x64")
Invoke-ExpectedBuildFailure "omitted-version" $common "MIRUMSI1001"
Invoke-ExpectedBuildFailure "empty-version" ($common + "-p:Version=") "MIRUMSI1001"
Invoke-ExpectedBuildFailure "omitted-bindir" @("-p:Version=1.2.3", "-p:Platform=x64") "MIRUMSI1003"
Invoke-ExpectedBuildFailure "empty-bindir" @("-p:Version=1.2.3", "-p:BinDir=", "-p:Platform=x64") "MIRUMSI1003"
foreach ($case in @(
    @("1.2.3-beta.1", "MIRUMSI1002"),
    @("1.2.3.4", "MIRUMSI1002"),
    @("256.0.0", "MIRUMSI1004"),
    @("1.256.0", "MIRUMSI1005"),
    @("1.2.65536", "MIRUMSI1006")
)) {
    $invalidVersion, $expectedErrorCode = $case
    Invoke-ExpectedBuildFailure ("version-" + $invalidVersion.Replace(".", "-").Replace("+", "-") ) ($common + "-p:Version=$invalidVersion") $expectedErrorCode
}
Write-Host "PASS invalid inputs rejected (9 cases)"

$missingDirectory = Join-Path $invalidDirectory "does-not-exist"
Invoke-ExpectedBuildFailure "nonexistent-bindir" @("-p:Version=1.2.3", "-p:BinDir=$missingDirectory", "-p:Platform=x64") "MIRUMSI1007"
$emptyBin = Join-Path $invalidDirectory "missing-executable"
New-Item -ItemType Directory -Path $emptyBin | Out-Null
Invoke-ExpectedBuildFailure "missing-executable-bindir" @("-p:Version=1.2.3", "-p:BinDir=$emptyBin", "-p:Platform=x64") "MIRUMSI1008"
Invoke-ExpectedBuildFailure "missing-fixture-payload" ($common + @("-p:Version=1.2.3", "-p:TestWixSource=integration-test.wxs")) "MIRUMSI1009"
Write-Host "PASS missing BinDir and FixturePayloadPath rejected"
