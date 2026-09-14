#Requires -Version 5.1
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# Keep these in sync with build/windows/miru-agent.wxs.
$MsiProductName = "Miru Agent"
$MsiManufacturer = "Miru"
$MsiUpgradeCode = "{B5ED0336-5F14-4308-A667-3CE8CDEF7D48}"
$MsiExpectedSddl = "O:SYD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
$MsiExpectedDirectories = @(
    @("MiruDataDir", "{D0542DF7-5B61-4F09-938B-57F05C1B5458}", "MIRUDATA", "CommonAppDataFolder", "Miru"),
    @("MiruLogsDir", "{C3AF8332-28E8-4707-8430-780C553D86EC}", "MIRULOGS", "MIRUDATA", "logs"),
    @("MiruAuthDir", "{A2AE361A-41E6-427A-AF4C-ACCEE7F451F9}", "MIRUAUTH", "MIRUDATA", "auth"),
    @("MiruTmpDir", "{D654A9BF-2860-44FA-8FFB-A8E36986197B}", "MIRUTMP", "MIRUDATA", "tmp")
)
$MsiFixtureProductCodes = @(
    "{B7AFDD4E-E6DB-4ED9-8C34-F318A04486B1}",
    "{3CE73709-ECE4-48A5-B7E7-1AC13C5EF30A}",
    "{4E72A894-00B5-433B-A445-C2CFD7FCF432}"
)

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

function New-MsiSessionLogDirectory {
    param([Parameter(Mandatory = $true)][string]$DeterministicLogs)
    Join-Path $DeterministicLogs ([Guid]::NewGuid().ToString("N"))
}

function Open-MsiDatabase {
    param([Parameter(Mandatory = $true)][string]$Path)
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $database = $installer.GetType().InvokeMember("OpenDatabase", "InvokeMethod", $null, $installer, @($Path, 0))
    return [pscustomobject]@{ Installer = $installer; Database = $database }
}

function Close-MsiDatabase {
    param($Handle)
    if ($null -ne $Handle -and $null -ne $Handle.Database) {
        [Runtime.InteropServices.Marshal]::ReleaseComObject($Handle.Database) | Out-Null
    }
    if ($null -ne $Handle -and $null -ne $Handle.Installer) {
        [Runtime.InteropServices.Marshal]::ReleaseComObject($Handle.Installer) | Out-Null
    }
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
    param([Parameter(Mandatory = $true)][string]$Path)
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

function Get-MsiIdentity {
    param([Parameter(Mandatory = $true)][string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    try {
        $values = @{}
        foreach ($name in @("ProductName", "ProductVersion", "ProductCode", "UpgradeCode")) {
            $value = Get-MsiPropertyValue $handle.Database $name
            Assert-True ($null -ne $value) "MSI identity property $name present"
            $values[$name] = $value
        }
        return [pscustomobject]$values
    }
    finally { Close-MsiDatabase $handle }
}

function Invoke-DotNetBuild {
    param(
        [Parameter(Mandatory = $true)][string]$ProjectPath,
        [Parameter(Mandatory = $true)][string]$BinDir,
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)][string]$OutputDirectory,
        [string]$ProductCode = "",
        [string]$TestWixSource = "",
        [string]$FixturePayloadPath = ""
    )

    $arguments = @(
        "build", $ProjectPath, "--no-restore", "--configuration", "Release",
        "-p:Platform=x64", "-p:Version=$Version", "-p:BinDir=$BinDir",
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

function Assert-FailingFixtureContract {
    param([Parameter(Mandatory = $true)][string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    try {
        $tables = @(Get-MsiRows $handle.Database "SELECT ``Name`` FROM ``_Tables``" 1)
        Assert-Equal 1 (@($tables | Where-Object { $_[0] -eq "CustomAction" })).Count "fixture custom action table"
        $actions = @(Get-MsiRows $handle.Database "SELECT ``Action``, ``Type``, ``Source``, ``Target`` FROM ``CustomAction``" 4)
        $action = @($actions | Where-Object { $_[0] -eq "FailUpgradeForTest" })
        Assert-Equal 1 $action.Count "one failing fixture custom action"
        Assert-Equal 3106 ([int]$action[0][1]) "deferred no-impersonate checked Type 34 action"
        Assert-Equal "SystemFolder" $action[0][2] "failing action SystemFolder source"
        Assert-Equal "[SystemFolder]cmd.exe /d /c exit /b 1" $action[0][3] "isolated cmd failure command"
        $sequence = @(Get-MsiRows $handle.Database "SELECT ``Action``, ``Condition``, ``Sequence`` FROM ``InstallExecuteSequence``" 3)
        $fixtureRow = @($sequence | Where-Object { $_[0] -eq "FailUpgradeForTest" })
        Assert-Equal 1 $fixtureRow.Count "one failing action sequence row"
        Assert-Equal "FAIL_UPGRADE_FOR_TEST=1" $fixtureRow[0][1] "failing action condition"
        $installFiles = [int](@($sequence | Where-Object { $_[0] -eq "InstallFiles" })[0][2])
        $installFinalize = [int](@($sequence | Where-Object { $_[0] -eq "InstallFinalize" })[0][2])
        $fixtureSequence = [int]$fixtureRow[0][2]
        Assert-True ($fixtureSequence -gt $installFiles -and $fixtureSequence -lt $installFinalize) "failing action runs after files and before finalize"
    }
    finally { Close-MsiDatabase $handle }
}

Export-ModuleMember -Function @(
    "Assert-True",
    "Assert-Equal",
    "New-MsiSessionLogDirectory",
    "Open-MsiDatabase",
    "Close-MsiDatabase",
    "Get-MsiRows",
    "Get-MsiPropertyValue",
    "Test-MsiTable",
    "Get-MsiContract",
    "Get-MsiIdentity",
    "Invoke-DotNetBuild",
    "Assert-FailingFixtureContract"
) -Variable @(
    "MsiProductName",
    "MsiManufacturer",
    "MsiUpgradeCode",
    "MsiExpectedSddl",
    "MsiExpectedDirectories",
    "MsiFixtureProductCodes"
)
