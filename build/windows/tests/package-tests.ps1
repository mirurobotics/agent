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

function Reset-ChildDirectory {
    param(
        [Parameter(Mandatory = $true)][string]$Parent,
        [Parameter(Mandatory = $true)][string]$Child
    )
    $path = Join-Path $Parent $Child
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Recurse -Force
    }
    return Initialize-Directory $path
}

function Build-ProductionPackage {
    param(
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)][string]$Directory
    )
    $built = Invoke-DotNetBuild -ProjectPath $resolvedProject `
        -BinDir $resolvedBinDir -Version $Version -OutputDirectory $Directory
    $path = Join-Path $Directory "miru-agent-$Version.msi"
    if ($built -ne $path) { Copy-Item -LiteralPath $built -Destination $path -Force }
    $metadata = Assert-Package $path $Version
    Assert-ProductionTables $path
    Write-Host "PASS package $Version"
    return $metadata
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

function Assert-ProductionTables {
    param([Parameter(Mandatory = $true)][string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    try {
        Assert-DirectoryComponents $handle.Database
        Assert-ProtectedPermissionRows $handle.Database
        Assert-FixtureIsolation $handle.Database
        Assert-TransactionalMajorUpgrade $handle.Database
        Assert-DowngradeLaunchCondition $handle.Database
        Assert-ServiceTables $handle.Database
    }
    finally { Close-MsiDatabase $handle }
}

function Assert-DirectoryComponents {
    param([Parameter(Mandatory = $true)]$Database)
    $layout = Get-DirectoryLayout $Database
    foreach ($expected in $MsiExpectedDirectories) {
        Assert-RetainedDirectoryComponent $expected $layout
    }
    Assert-AgentBinaryComponent $layout.Components
    Assert-InstallFolder $layout.Directories
}

function Get-DirectoryLayout {
    param([Parameter(Mandatory = $true)]$Database)
    $componentQuery = "SELECT ``Component``, ``ComponentId``, ``Directory_``, " + `
        "``Attributes``, ``KeyPath`` FROM ``Component``"
    $directoryQuery = "SELECT ``Directory``, ``Directory_Parent``, " + `
        "``DefaultDir`` FROM ``Directory``"
    $folderQuery = "SELECT ``Directory_``, ``Component_`` FROM ``CreateFolder``"
    return [pscustomobject]@{
        Components = @(Get-MsiRows $Database $componentQuery 5)
        Directories = @(Get-MsiRows $Database $directoryQuery 3)
        Folders = @(Get-MsiRows $Database $folderQuery 2)
    }
}

function Assert-RetainedDirectoryComponent {
    param(
        [Parameter(Mandatory = $true)][object[]]$Expected,
        [Parameter(Mandatory = $true)]$Layout
    )
    $name, $guid, $directory, $parent, $leaf = $Expected
    $retained = @($Layout.Components | Where-Object { $_[0] -eq $name })
    Assert-Equal 1 $retained.Count "retained component $name"
    Assert-Equal $guid $retained[0][1].ToUpperInvariant() `
        "$name stable component identity"
    Assert-Equal $directory $retained[0][2] "$name component directory"
    Assert-True (([int]$retained[0][3] -band 256) -ne 0) "$name is 64-bit"
    Assert-True ([string]::IsNullOrEmpty($retained[0][4])) `
        "$name has a directory key path"
    $hierarchy = @($Layout.Directories | Where-Object {
        $_[0] -eq $directory -and $_[1] -eq $parent -and $_[2] -eq $leaf
    })
    Assert-Equal 1 $hierarchy.Count "$name directory hierarchy"
    $created = @($Layout.Folders | Where-Object {
        $_[0] -eq $directory -and $_[1] -eq $name
    })
    Assert-Equal 1 $created.Count "$name CreateFolder mapping"
}

function Assert-AgentBinaryComponent {
    param([Parameter(Mandatory = $true)][object[]]$Components)
    $binary = @($Components | Where-Object { $_[0] -eq "MiruAgentExe" })
    Assert-Equal 1 $binary.Count "binary component"
    Assert-True (([int]$binary[0][3] -band 256) -ne 0) "binary component is 64-bit"
    Assert-Equal "AGENTFOLDER" $binary[0][2] "binary component directory"
    Assert-Equal "miru_agent.exe" $binary[0][4] "binary component file key path"
}

function Assert-InstallFolder {
    param([Parameter(Mandatory = $true)][object[]]$Directories)
    $installFolder = @($Directories | Where-Object {
        $_[0] -eq "INSTALLFOLDER" -and
        $_[1] -eq "ProgramFiles64Folder" -and
        $_[2] -eq "Miru"
    })
    Assert-Equal 1 $installFolder.Count "64-bit install directory"
}

function Assert-ProtectedPermissionRows {
    param([Parameter(Mandatory = $true)]$Database)
    $permissionQuery = "SELECT ``LockObject``, ``Table``, ``SDDLText`` " + `
        "FROM ``MsiLockPermissionsEx``"
    $permissions = @(Get-MsiRows $Database $permissionQuery 3)
    $expected = @($MsiExpectedDirectories | ForEach-Object {
        "$($_[2])|CreateFolder|$MsiExpectedSddl"
    } | Sort-Object)
    $actual = @($permissions | ForEach-Object {
        "{0}|{1}|{2}" -f $_[0], $_[1], $_[2]
    } | Sort-Object)
    Assert-Equal ($expected -join "`n") ($actual -join "`n") `
        "exact protected permission rows"
}

function Assert-FixtureIsolation {
    param([Parameter(Mandatory = $true)]$Database)
    $actions = @()
    if (Test-MsiTable $Database "CustomAction") {
        $actionQuery = "SELECT ``Action``, ``Type``, ``Source``, ``Target`` " + `
            "FROM ``CustomAction``"
        $actions = @(Get-MsiRows $Database $actionQuery 4)
    }
    $fixturePattern = 'FailUpgradeForTest|FAIL_UPGRADE_FOR_TEST|rollback-payload'
    $actionHits = @($actions | Where-Object {
        ($_[0] + $_[2] + $_[3]) -match $fixturePattern
    })
    Assert-Equal 0 $actionHits.Count "production custom action isolation"
    $sequenceQuery = "SELECT ``Action``, ``Condition``, ``Sequence`` " + `
        "FROM ``InstallExecuteSequence``"
    $sequence = @(Get-MsiRows $Database $sequenceQuery 3)
    $sequenceHits = @($sequence | Where-Object {
        ($_[0] + $_[1]) -match 'FailUpgradeForTest|FAIL_UPGRADE_FOR_TEST'
    })
    Assert-Equal 0 $sequenceHits.Count "production sequence isolation"
    $files = @(Get-MsiRows $Database "SELECT ``FileName`` FROM ``File``" 1)
    $fileHits = @($files | Where-Object { $_[0] -match 'rollback|fixture|test' })
    Assert-Equal 0 $fileHits.Count "no test fixture payload"
}

function Assert-TransactionalMajorUpgrade {
    param([Parameter(Mandatory = $true)]$Database)
    $sequenceQuery = "SELECT ``Action``, ``Condition``, ``Sequence`` " + `
        "FROM ``InstallExecuteSequence``"
    $sequence = @(Get-MsiRows $Database $sequenceQuery 3)
    $removeExisting = @($sequence | Where-Object {
        $_[0] -eq "RemoveExistingProducts"
    })
    Assert-Equal 1 $removeExisting.Count `
        "major upgrade removes existing product"
    $remove = Get-SequenceNumber $sequence "RemoveExistingProducts"
    $initialize = Get-SequenceNumber $sequence "InstallInitialize"
    $finalize = Get-SequenceNumber $sequence "InstallFinalize"
    Assert-True ($remove -gt $initialize -and $remove -lt $finalize) `
        "RemoveExistingProducts is transactional"
    $upgradeQuery = "SELECT ``UpgradeCode``, ``VersionMin``, ``VersionMax``, " + `
        "``Attributes``, ``ActionProperty`` FROM ``Upgrade``"
    $upgradeRows = @(Get-MsiRows $Database $upgradeQuery 5)
    Assert-True ($upgradeRows.Count -ge 2) "upgrade and downgrade rows exist"
}

function Get-SequenceNumber {
    param(
        [Parameter(Mandatory = $true)][object[]]$Sequence,
        [Parameter(Mandatory = $true)][string]$Action
    )
    return [int](@($Sequence | Where-Object { $_[0] -eq $Action })[0][2])
}

function Assert-DowngradeLaunchCondition {
    param([Parameter(Mandatory = $true)]$Database)
    $conditionQuery = "SELECT ``Condition``, ``Description`` " + `
        "FROM ``LaunchCondition``"
    $conditions = @(Get-MsiRows $Database $conditionQuery 2)
    $downgrade = @($conditions | Where-Object {
        $_[0] -match 'WIX_DOWNGRADE_DETECTED'
    })
    Assert-Equal 1 $downgrade.Count "downgrade launch condition"
    Assert-Equal "A newer version of Miru Agent is already installed." `
        $downgrade[0][1] "downgrade launch condition description"
}

function Assert-ServiceTables {
    param([Parameter(Mandatory = $true)]$Database)
    Assert-ServiceInstallRow $Database
    Assert-MsiServiceConfig $Database
    Assert-ServiceControlRow $Database
    Assert-ServiceRecoveryTable $Database
    Assert-RollbackServiceConfig $Database
}

# The MSI registers miru-agent as an own-process, auto-start, vital service that
# runs as its virtual account NT SERVICE\miru-agent, with no arguments and the
# authored display name and description.
function Assert-ServiceInstallRow {
    param([Parameter(Mandatory = $true)]$Database)
    Assert-True (Test-MsiTable $Database "ServiceInstall") "service install table present"
    $query = "SELECT ``ServiceInstall``, ``Name``, ``DisplayName``, " + `
        "``ServiceType``, ``StartType``, ``ErrorControl``, ``StartName``, " + `
        "``Arguments``, ``Component_``, ``Description`` FROM ``ServiceInstall``"
    $rows = @(Get-MsiRows $Database $query 10)
    Assert-Equal 1 $rows.Count "one service install row"
    $row = $rows[0]
    Assert-Equal "miru-agent" $row[1] "service name"
    Assert-Equal "Miru Agent" $row[2] "service display name"
    Assert-Equal "Miru Config Agent" $row[9] "service description"
    Assert-Equal "MiruAgentExe" $row[8] "service owning component"
    Assert-Equal $MsiServiceAccount $row[6] "service runs as NT SERVICE\miru-agent"
    Assert-True ([string]::IsNullOrEmpty($row[7])) "service takes no arguments"
    Assert-True (([int]$row[3] -band 16) -ne 0) "service is own-process"
    Assert-Equal 2 ([int]$row[4]) "service start type is automatic"
    Assert-True (([int]$row[5] -band 1) -ne 0) "service error control is normal"
    Assert-True (([int]$row[5] -band 0x8000) -ne 0) "service is vital"
}

# The core ServiceConfig element compiles to two MsiServiceConfig rows, applied
# on install and repair: the unrestricted SID type and a required-privileges
# list. The rows are inert unless MsiConfigureServices runs between
# InstallServices and StartServices.
function Assert-MsiServiceConfig {
    param([Parameter(Mandatory = $true)]$Database)
    Assert-True (Test-MsiTable $Database "MsiServiceConfig") "MsiServiceConfig table present"
    $query = "SELECT ``MsiServiceConfig``, ``Name``, ``Event``, " + `
        "``ConfigType``, ``Argument``, ``Component_`` FROM ``MsiServiceConfig``"
    $rows = @(Get-MsiRows $Database $query 6)
    Assert-Equal 2 $rows.Count "two MsiServiceConfig rows"
    foreach ($row in $rows) {
        Assert-Equal $MsiServiceName $row[1] "service config $($row[0]) name"
        Assert-Equal "MiruAgentExe" $row[5] "service config $($row[0]) owning component"
        $configEvent = [int]$row[2]
        Assert-True (($configEvent -band 0x1) -ne 0) "service config $($row[0]) applied on install"
        Assert-True (($configEvent -band 0x4) -ne 0) "service config $($row[0]) applied on repair"
    }
    $sidTypeArguments = @{ none = "0"; unrestricted = "1"; restricted = "3" }
    $sidRows = @($rows | Where-Object { [int]$_[3] -eq 5 })
    Assert-Equal 1 $sidRows.Count "one SERVICE_CONFIG_SERVICE_SID_INFO row"
    Assert-Equal $sidTypeArguments[$MsiServiceSettings.SidType] $sidRows[0][4] `
        "service SID type is $($MsiServiceSettings.SidType)"
    $privilegeRows = @($rows | Where-Object { [int]$_[3] -eq 6 })
    Assert-Equal 1 $privilegeRows.Count "one SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO row"
    Assert-Equal ($MsiServiceSettings.RequiredPrivileges -join "[~]") $privilegeRows[0][4] `
        "service required privileges"
    $sequenceQuery = "SELECT ``Action``, ``Condition``, ``Sequence`` " + `
        "FROM ``InstallExecuteSequence``"
    $sequence = @(Get-MsiRows $Database $sequenceQuery 3)
    $configureRows = @($sequence | Where-Object {
        $_[0] -eq "MsiConfigureServices"
    })
    Assert-Equal 1 $configureRows.Count `
        "MsiConfigureServices is scheduled once"
    $configure = Get-SequenceNumber $sequence "MsiConfigureServices"
    $install = Get-SequenceNumber $sequence "InstallServices"
    $start = Get-SequenceNumber $sequence "StartServices"
    Assert-True ($configure -gt $install -and $configure -lt $start) `
        "MsiConfigureServices runs after InstallServices and before StartServices"
}

# ServiceControl starts the service on install, stops it on install and
# uninstall, deletes it on uninstall, and waits for each transition.
function Assert-ServiceControlRow {
    param([Parameter(Mandatory = $true)]$Database)
    Assert-True (Test-MsiTable $Database "ServiceControl") "service control table present"
    $query = "SELECT ``Name``, ``Event``, ``Wait``, ``Component_`` " + `
        "FROM ``ServiceControl``"
    $rows = @(Get-MsiRows $Database $query 4)
    Assert-Equal 1 $rows.Count "one service control row"
    $row = $rows[0]
    Assert-Equal "miru-agent" $row[0] "service control name"
    Assert-Equal "MiruAgentExe" $row[3] "service control owning component"
    Assert-Equal 1 ([int]$row[2]) "service control waits for transitions"
    $serviceEvent = [int]$row[1]
    Assert-True (($serviceEvent -band 0x1) -ne 0) "service starts on install"
    Assert-True (($serviceEvent -band 0x2) -ne 0) "service stops on install"
    Assert-True (($serviceEvent -band 0x20) -ne 0) "service stops on uninstall"
    Assert-True (($serviceEvent -band 0x80) -ne 0) "service deletes on uninstall"
}

# The Util extension emits its own failure-actions table, Wix4ServiceConfig,
# rather than the standard MSI ServiceConfig table. Its units are the spec's:
# seconds for the restart delay and days for the reset period.
function Assert-ServiceRecoveryTable {
    param([Parameter(Mandatory = $true)]$Database)
    Assert-True (Test-MsiTable $Database "Wix4ServiceConfig") `
        "WiX Util service recovery table present"
    $query = "SELECT ``ServiceName``, ``Component_``, ``NewService``, " + `
        "``FirstFailureActionType``, ``SecondFailureActionType``, " + `
        "``ThirdFailureActionType``, ``ResetPeriodInDays``, " + `
        "``RestartServiceDelayInSeconds``, ``ProgramCommandLine``, " + `
        "``RebootMessage`` FROM ``Wix4ServiceConfig``"
    $rows = @(Get-MsiRows $Database $query 10)
    Assert-Equal 1 $rows.Count "one service recovery row"
    $row = $rows[0]
    Assert-Equal $MsiServiceName $row[0] "service recovery service name"
    Assert-Equal "MiruAgentExe" $row[1] "service recovery owning component"
    Assert-Equal "1" $row[2] "service recovery configures the installed service"
    Assert-Equal ($MsiServiceSettings.FailureActions -join ",") `
        (@($row[3], $row[4], $row[5]) -join ",") "service failure actions"
    Assert-Equal ([string]$MsiServiceSettings.ResetPeriodDays) $row[6] `
        "service failure count reset period in days"
    Assert-Equal ([string]$MsiServiceSettings.RestartDelaySeconds) $row[7] `
        "service restart delay in seconds"
    Assert-True ([string]::IsNullOrEmpty($row[8]) -and [string]::IsNullOrEmpty($row[9])) `
        "service recovery runs no command and shows no reboot message"
}

# If a failed major upgrade rolls back, Windows Installer recreates the old
# service without its failure actions, SID type, or required privileges, so
# miru-agent.wxs reapplies them with one rollback-only, no-impersonation
# WixQuietExec64 action that ignores failure; a type-51 setter passes it the
# sc.exe command line, which must encode the spec above. Both run only in the
# uninstall a major upgrade starts, between StopServices and DeleteServices, so
# rollback reapplies the settings after recreating the service and before
# restarting it.
function Assert-RollbackServiceConfig {
    param([Parameter(Mandatory = $true)]$Database)
    $name = "MiruRollbackServiceConfig"
    $actionQuery = "SELECT ``Action``, ``Type``, ``Source``, ``Target`` " + `
        "FROM ``CustomAction``"
    $actions = @(Get-MsiRows $Database $actionQuery 4)
    $action = @($actions | Where-Object { $_[0] -eq $name })
    Assert-Equal 1 $action.Count "one $name custom action"
    # msidbCustomActionTypeDll + Continue + Rollback + InScript + NoImpersonate
    Assert-Equal 3393 ([int]$action[0][1]) `
        "$name is a rollback-only, no-impersonation, ignore-failure DLL action"
    Assert-Equal "Wix4UtilCA_X64" $action[0][2] "$name runs the x64 Util CA DLL"
    $binaries = @(Get-MsiRows $Database "SELECT ``Name`` FROM ``Binary``" 1 |
        ForEach-Object { $_[0] })
    Assert-True ($binaries -contains $action[0][2]) "$name DLL is in the Binary table"
    Assert-Equal "WixQuietExec64" $action[0][3] "$name runs WixQuietExec64"
    $setter = @($actions | Where-Object { $_[0] -eq "Set$name" })
    Assert-Equal 1 $setter.Count "one Set$name custom action"
    # msidbCustomActionTypeTextData + SourceProperty
    Assert-Equal 51 ([int]$setter[0][1]) "Set$name sets a property"
    Assert-Equal $name $setter[0][2] "Set$name sets the $name CustomActionData"
    Assert-Equal (Get-RollbackServiceCommand) $setter[0][3] `
        "$name command line matches the service spec"
    $sequenceQuery = "SELECT ``Action``, ``Condition``, ``Sequence`` " + `
        "FROM ``InstallExecuteSequence``"
    $sequence = @(Get-MsiRows $Database $sequenceQuery 3)
    foreach ($scheduledName in "Set$name", $name) {
        $scheduled = @($sequence | Where-Object { $_[0] -eq $scheduledName })
        Assert-Equal 1 $scheduled.Count "$scheduledName is scheduled once"
        Assert-Equal 'UPGRADINGPRODUCTCODE AND REMOVE~="ALL"' $scheduled[0][1] `
            "$scheduledName is scheduled only when a major upgrade removes this version"
    }
    $order = @("StopServices", "Set$name", $name, "DeleteServices")
    for ($i = 1; $i -lt $order.Count; $i++) {
        $previous = Get-SequenceNumber $sequence $order[$i - 1]
        $current = Get-SequenceNumber $sequence $order[$i]
        Assert-True ($previous -lt $current) `
            "$($order[$i - 1]) is sequenced before $($order[$i])"
    }
}

# The rollback command line rendered from the service spec: sc.exe takes the
# reset period in seconds and the restart delay in milliseconds.
function Get-RollbackServiceCommand {
    $settings = $MsiServiceSettings
    $delayMilliseconds = $settings.RestartDelaySeconds * 1000
    $actions = @($settings.FailureActions | ForEach-Object { "$_/$delayMilliseconds" }) -join "/"
    $sc = '"[System64Folder]sc.exe"'
    $commands = @(
        "$sc failure $MsiServiceName reset= $($settings.ResetPeriodDays * 86400) actions= $actions",
        "$sc sidtype $MsiServiceName $($settings.SidType)",
        "$sc privs $MsiServiceName $($settings.RequiredPrivileges -join '/')"
    )
    return '"[System64Folder]cmd.exe" /d /s /c "{0}"' -f ($commands -join " & ")
}

function Assert-ServiceSidDerivation {
    Assert-Equal "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464" `
        (Get-ServiceSid "TrustedInstaller") "TrustedInstaller service SID derivation"
    Assert-Equal $MsiServiceSid (Get-ServiceSid $MsiServiceName) `
        "miru-agent service SID derivation"
    Write-Host "PASS service SID derivation"
}

function Build-FixturePackage {
    param([Parameter(Mandatory = $true)][string]$Directory)
    $payload = Join-Path $Directory "rollback-payload.txt"
    [IO.File]::WriteAllText($payload, "fixture-contract", [Text.Encoding]::ASCII)
    return Invoke-DotNetBuild -ProjectPath $resolvedProject `
        -BinDir $resolvedBinDir -Version "1.2.0" `
        -OutputDirectory $Directory `
        -ProductCode $MsiFixtureProductCodes[2] `
        -TestWixSource (Join-Path $PSScriptRoot "integration-test.wxs") `
        -FixturePayloadPath $payload
}

function Invoke-ExpectedBuildFailure {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$Properties,
        [Parameter(Mandatory = $true)][string]$ExpectedErrorCode
    )
    $outputPath = Initialize-Directory (Join-Path $invalidDirectory $Name)
    $arguments = @(
        "build", $resolvedProject, "--no-restore", "--configuration", "Release",
        "-p:OutputPath=$outputPath\"
    ) + $Properties
    $output = @(& dotnet @arguments 2>&1)
    $exitCode = $LASTEXITCODE
    $outputText = $output | Out-String
    Assert-True ($exitCode -ne 0) "$Name build fails`n$outputText"
    Assert-True ($outputText -match `
        "\b$([regex]::Escape($ExpectedErrorCode))\b") `
        "$Name expected error code $ExpectedErrorCode`n$outputText"
    $msiCount = @(Get-ChildItem -LiteralPath $outputPath `
        -Filter "*.msi" -File -Recurse).Count
    Assert-Equal 0 $msiCount "$Name produces no MSI`n$outputText"
}

$resolvedProject = (Resolve-Path -LiteralPath $ProjectPath).Path
$resolvedBinDir = (Resolve-Path -LiteralPath $BinDir).Path
$resolvedArtifacts = Initialize-Directory ([IO.Path]::GetFullPath($ArtifactsDirectory))
$invalidDirectory = Reset-ChildDirectory $resolvedArtifacts "invalid"

Assert-ServiceSidDerivation
$v1 = Build-ProductionPackage "1.0.0" (Reset-ChildDirectory $resolvedArtifacts "v1")
$v2 = Build-ProductionPackage "1.1.0" (Reset-ChildDirectory $resolvedArtifacts "v2")
Assert-True (-not [string]::Equals($v1.ProductCode, $v2.ProductCode, `
    [StringComparison]::OrdinalIgnoreCase)) "normal ProductCodes differ"
Assert-Equal $v1.UpgradeCode $v2.UpgradeCode "normal UpgradeCode remains stable"
Write-Host "PASS ProductCodes differ, UpgradeCode stable"

Assert-FailingFixtureContract (Build-FixturePackage `
    (Reset-ChildDirectory $resolvedArtifacts "fixture"))
Write-Host "PASS fixture custom-action contract"

# One representative case per validation family proves the wixproj error
# mechanism works; the remaining MIRUMSI codes are three-line MSBuild checks.
$common = @("-p:BinDir=$resolvedBinDir", "-p:Platform=x64")
Invoke-ExpectedBuildFailure "prerelease-version" `
    ($common + "-p:Version=1.2.3-beta.1") "MIRUMSI1002"
Invoke-ExpectedBuildFailure "omitted-bindir" `
    @("-p:Version=1.2.3", "-p:Platform=x64") "MIRUMSI1003"
Invoke-ExpectedBuildFailure "missing-fixture-payload" `
    ($common + @("-p:Version=1.2.3", `
        "-p:TestWixSource=integration-test.wxs")) "MIRUMSI1009"
Write-Host "PASS invalid inputs rejected (3 cases)"
