[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$ProjectPath,
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$BinDir,
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$ArtifactsDirectory
)

$ErrorActionPreference = "Stop"
$expectedUpgradeCode = "{B5ED0336-5F14-4308-A667-3CE8CDEF7D48}"
$expectedDirectories = @(
    @("MiruDataDir", "{D0542DF7-5B61-4F09-938B-57F05C1B5458}", "MIRUDATA", "CommonAppDataFolder", "Miru"),
    @("MiruLogsDir", "{C3AF8332-28E8-4707-8430-780C553D86EC}", "MIRULOGS", "MIRUDATA", "logs"),
    @("MiruAuthDir", "{A2AE361A-41E6-427A-AF4C-ACCEE7F451F9}", "MIRUAUTH", "MIRUDATA", "auth"),
    @("MiruTmpDir", "{D654A9BF-2860-44FA-8FFB-A8E36986197B}", "MIRUTMP", "MIRUDATA", "tmp")
)
$expectedSddl = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw "ASSERT: $Message" }
}

function Assert-Equal {
    param($Expected, $Actual, [string]$Message)
    if (-not [object]::Equals($Expected, $Actual)) {
        throw "ASSERT: $Message (expected '$Expected', actual '$Actual')"
    }
}

function Remove-AndCreateChild {
    param([string]$Parent, [string]$Child)
    $path = Join-Path $Parent $Child
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Recurse -Force
    }
    New-Item -ItemType Directory -Path $path | Out-Null
    return (Resolve-Path $path).Path
}

function Invoke-DotNetBuild {
    param(
        [string]$Version,
        [string]$OutputDirectory,
        [string]$ProductCode = "",
        [string]$TestWixSource = "",
        [string]$FixturePayloadPath = ""
    )

    $arguments = @(
        "build", $script:resolvedProject, "--no-restore", "--configuration", "Release",
        "-p:Platform=x64", "-p:Version=$Version", "-p:BinDir=$script:resolvedBinDir",
        "-p:OutputPath=$OutputDirectory\", "-p:IntermediateOutputPath=$OutputDirectory\obj\"
    )
    if ($ProductCode) { $arguments += "-p:ProductCode=$ProductCode" }
    if ($TestWixSource) { $arguments += "-p:TestWixSource=$TestWixSource" }
    if ($FixturePayloadPath) { $arguments += "-p:FixturePayloadPath=$FixturePayloadPath" }
    & dotnet @arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "dotnet build failed for version $Version" }
    $msi = Get-ChildItem -LiteralPath $OutputDirectory -Filter "*.msi" -File -Recurse |
        Where-Object { $_.FullName -notmatch '\\obj\\' } |
        Select-Object -First 1
    if ($null -eq $msi) { throw "No MSI was produced for version $Version" }
    return $msi.FullName
}

function Open-MsiDatabase {
    param([string]$Path)
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $database = $installer.GetType().InvokeMember("OpenDatabase", "InvokeMethod", $null, $installer, @($Path, 0))
    return [pscustomobject]@{ Installer = $installer; Database = $database }
}

function Close-MsiDatabase {
    param($Handle)
    if ($null -ne $Handle.Database) { [Runtime.InteropServices.Marshal]::ReleaseComObject($Handle.Database) | Out-Null }
    if ($null -ne $Handle.Installer) { [Runtime.InteropServices.Marshal]::ReleaseComObject($Handle.Installer) | Out-Null }
}

function Get-MsiRows {
    param($Database, [string]$Query, [int]$Columns)
    $view = $null
    $record = $null
    $rows = @()
    try {
        $view = $Database.GetType().InvokeMember("OpenView", "InvokeMethod", $null, $Database, @($Query))
        $view.GetType().InvokeMember("Execute", "InvokeMethod", $null, $view, $null) | Out-Null
        while ($true) {
            $record = $view.GetType().InvokeMember("Fetch", "InvokeMethod", $null, $view, $null)
            if ($null -eq $record) { break }
            $row = @()
            for ($column = 1; $column -le $Columns; $column++) {
                $row += $record.GetType().InvokeMember("StringData", "GetProperty", $null, $record, @($column))
            }
            $rows += ,$row
            [Runtime.InteropServices.Marshal]::ReleaseComObject($record) | Out-Null
            $record = $null
        }
        return @($rows)
    }
    finally {
        if ($null -ne $record) { [Runtime.InteropServices.Marshal]::ReleaseComObject($record) | Out-Null }
        if ($null -ne $view) {
            $view.GetType().InvokeMember("Close", "InvokeMethod", $null, $view, $null) | Out-Null
            [Runtime.InteropServices.Marshal]::ReleaseComObject($view) | Out-Null
        }
    }
}

function Get-MsiPropertyValue {
    param($Database, [string]$Name)
    $escaped = $Name.Replace("'", "''")
    $rows = @(Get-MsiRows -Database $Database -Query "SELECT ``Value`` FROM ``Property`` WHERE ``Property``='$escaped'" -Columns 1)
    if ($rows.Count -eq 0) { return $null }
    return $rows[0][0]
}

function Test-MsiTable {
    param($Database, [string]$Name)
    $escaped = $Name.Replace("'", "''")
    return (@(Get-MsiRows -Database $Database -Query "SELECT ``Name`` FROM ``_Tables`` WHERE ``Name``='$escaped'" -Columns 1)).Count -eq 1
}

function Get-MsiContract {
    param([string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    $summary = $null
    try {
        $summary = $handle.Database.GetType().InvokeMember("SummaryInformation", "GetProperty", $null, $handle.Database, @(0))
        $template = $summary.GetType().InvokeMember("Property", "GetProperty", $null, $summary, @(7))
        return [pscustomobject]@{
            ProductName = Get-MsiPropertyValue $handle.Database "ProductName"
            Manufacturer = Get-MsiPropertyValue $handle.Database "Manufacturer"
            ProductVersion = Get-MsiPropertyValue $handle.Database "ProductVersion"
            ProductCode = Get-MsiPropertyValue $handle.Database "ProductCode"
            UpgradeCode = Get-MsiPropertyValue $handle.Database "UpgradeCode"
            ALLUSERS = Get-MsiPropertyValue $handle.Database "ALLUSERS"
            Template = $template
            InstallerVersion = $summary.GetType().InvokeMember("Property", "GetProperty", $null, $summary, @(14))
        }
    }
    finally {
        if ($null -ne $summary) { [Runtime.InteropServices.Marshal]::ReleaseComObject($summary) | Out-Null }
        Close-MsiDatabase $handle
    }
}

function Assert-ProductionTables {
    param([string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    try {
        $components = @(Get-MsiRows $handle.Database "SELECT ``Component``, ``ComponentId``, ``Directory_``, ``Attributes``, ``KeyPath`` FROM ``Component``" 5)
        $directories = @(Get-MsiRows $handle.Database "SELECT ``Directory``, ``Directory_Parent``, ``DefaultDir`` FROM ``Directory``" 3)
        $folders = @(Get-MsiRows $handle.Database "SELECT ``Directory_``, ``Component_`` FROM ``CreateFolder``" 2)
        foreach ($expected in $expectedDirectories) {
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
        $expectedPermissions = @($expectedDirectories | ForEach-Object { "$($_[2])|CreateFolder|$expectedSddl" } | Sort-Object)
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
    param([string]$Path, [string]$Version)
    $metadata = Get-MsiContract -Path $Path
    Assert-Equal "Miru Agent" $metadata.ProductName "ProductName"
    Assert-Equal "Miru" $metadata.Manufacturer "Manufacturer"
    Assert-Equal $Version $metadata.ProductVersion "ProductVersion"
    Assert-Equal $expectedUpgradeCode $metadata.UpgradeCode "UpgradeCode"
    Assert-True ($metadata.ProductCode -match '^\{[0-9A-Fa-f-]{36}\}$') "ProductCode"
    Assert-True ($metadata.Template -match '(^|;)x64($|;)') "x64 summary template"
    Assert-Equal 500 ([int]$metadata.InstallerVersion) "InstallerVersion"
    Assert-Equal "1" $metadata.ALLUSERS "per-machine package"
    return $metadata
}

function Invoke-ExpectedBuildFailure {
    param([string]$Name, [string[]]$Properties, [string]$ExpectedErrorCode)
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

$v1Built = Invoke-DotNetBuild -Version "1.0.0" -OutputDirectory $v1Directory
$v1Path = Join-Path $v1Directory "miru-agent-1.0.0.msi"
if ($v1Built -ne $v1Path) { Copy-Item -LiteralPath $v1Built -Destination $v1Path -Force }
$v1 = Assert-Package $v1Path "1.0.0"
Assert-ProductionTables $v1Path
Write-Host "PASS package 1.0.0"

$v2Built = Invoke-DotNetBuild -Version "1.1.0" -OutputDirectory $v2Directory
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
    $built = Invoke-DotNetBuild -Version $version -OutputDirectory $directory
    $destination = Join-Path $boundariesDirectory "miru-agent-$version.msi"
    Copy-Item -LiteralPath $built -Destination $destination -Force
    Assert-Package $destination $version | Out-Null
    Assert-ProductionTables $destination
}
Write-Host "PASS version boundaries (0.0.0, 255.255.65535)"

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
