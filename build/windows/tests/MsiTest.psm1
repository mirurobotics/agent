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
    param(
        [Parameter(Mandatory = $true)][bool]$Condition,
        [Parameter(Mandatory = $true)][string]$Message
    )
    if (-not $Condition) { throw "ASSERT: $Message" }
}

function Assert-Equal {
    param(
        [Parameter(Mandatory = $true)]$Expected,
        [Parameter(Mandatory = $true)]$Actual,
        [Parameter(Mandatory = $true)][string]$Message
    )
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
    $database = Invoke-ComMethod $installer "OpenDatabase" @($Path, 0)
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

function Invoke-ComMethod {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [object[]]$Arguments = $null
    )
    return $Object.GetType().InvokeMember($Name, "InvokeMethod", $null, $Object, $Arguments)
}

function Get-ComProperty {
    param(
        [Parameter(Mandatory = $true)]$Object,
        [Parameter(Mandatory = $true)][string]$Name,
        [object[]]$Arguments = $null
    )
    return $Object.GetType().InvokeMember($Name, "GetProperty", $null, $Object, $Arguments)
}

function Read-MsiRecord {
    param(
        [Parameter(Mandatory = $true)]$Record,
        [Parameter(Mandatory = $true)][int]$Columns
    )
    $row = @()
    for ($column = 1; $column -le $Columns; $column++) {
        $row += Get-ComProperty $Record "StringData" @($column)
    }
    return ,$row
}

function Get-MsiRows {
    param(
        [Parameter(Mandatory = $true)]$Database,
        [Parameter(Mandatory = $true)][string]$Query,
        [Parameter(Mandatory = $true)][int]$Columns
    )
    $view = $null
    $rows = @()
    try {
        $view = Invoke-ComMethod $Database "OpenView" @($Query)
        Invoke-ComMethod $view "Execute" | Out-Null
        while ($true) {
            $record = Invoke-ComMethod $view "Fetch"
            if ($null -eq $record) { break }
            $rows += ,(Read-MsiRecord $record $Columns)   # comma keeps each row as one element
            [Runtime.InteropServices.Marshal]::ReleaseComObject($record) | Out-Null
        }
        return @($rows)
    }
    finally {
        if ($null -ne $view) {
            Invoke-ComMethod $view "Close" | Out-Null
            [Runtime.InteropServices.Marshal]::ReleaseComObject($view) | Out-Null
        }
    }
}

function Get-MsiPropertyValue {
    param(
        [Parameter(Mandatory = $true)]$Database,
        [Parameter(Mandatory = $true)][string]$Name
    )
    $escaped = $Name.Replace("'", "''")
    $rows = @(Get-MsiRows -Database $Database -Query "SELECT ``Value`` FROM ``Property`` WHERE ``Property``='$escaped'" -Columns 1)
    if ($rows.Count -eq 0) { return $null }
    return $rows[0][0]
}

function Test-MsiTable {
    param(
        [Parameter(Mandatory = $true)]$Database,
        [Parameter(Mandatory = $true)][string]$Name
    )
    $escaped = $Name.Replace("'", "''")
    return (@(Get-MsiRows -Database $Database -Query "SELECT ``Name`` FROM ``_Tables`` WHERE ``Name``='$escaped'" -Columns 1)).Count -eq 1
}

function Get-MsiContract {
    param([Parameter(Mandatory = $true)][string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    $summary = $null
    try {
        $summary = Get-ComProperty $handle.Database "SummaryInformation" @(0)
        return [pscustomobject]@{
            ProductName = Get-MsiPropertyValue $handle.Database "ProductName"
            Manufacturer = Get-MsiPropertyValue $handle.Database "Manufacturer"
            ProductVersion = Get-MsiPropertyValue $handle.Database "ProductVersion"
            ProductCode = Get-MsiPropertyValue $handle.Database "ProductCode"
            UpgradeCode = Get-MsiPropertyValue $handle.Database "UpgradeCode"
            ALLUSERS = Get-MsiPropertyValue $handle.Database "ALLUSERS"
            Template = Get-ComProperty $summary "Property" @(7)
            InstallerVersion = Get-ComProperty $summary "Property" @(14)
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

function Assert-FixtureCustomAction {
    param([Parameter(Mandatory = $true)]$Database)
    Assert-True (Test-MsiTable $Database "CustomAction") "fixture custom action table"
    $actions = @(Get-MsiRows $Database "SELECT ``Action``, ``Type``, ``Source``, ``Target`` FROM ``CustomAction``" 4)
    $action = @($actions | Where-Object { $_[0] -eq "FailUpgradeForTest" })
    Assert-Equal 1 $action.Count "one failing fixture custom action"
    Assert-Equal 3106 ([int]$action[0][1]) "deferred no-impersonate checked Type 34 action"
    Assert-Equal "SystemFolder" $action[0][2] "failing action SystemFolder source"
    Assert-Equal "[SystemFolder]cmd.exe /d /c exit /b 1" $action[0][3] "isolated cmd failure command"
}

function Assert-FixtureSequence {
    param([Parameter(Mandatory = $true)]$Database)
    $sequence = @(Get-MsiRows $Database "SELECT ``Action``, ``Condition``, ``Sequence`` FROM ``InstallExecuteSequence``" 3)
    $fixtureRow = @($sequence | Where-Object { $_[0] -eq "FailUpgradeForTest" })
    Assert-Equal 1 $fixtureRow.Count "one failing action sequence row"
    Assert-Equal "FAIL_UPGRADE_FOR_TEST=1" $fixtureRow[0][1] "failing action condition"
    $installFiles = [int](@($sequence | Where-Object { $_[0] -eq "InstallFiles" })[0][2])
    $installFinalize = [int](@($sequence | Where-Object { $_[0] -eq "InstallFinalize" })[0][2])
    $fixtureSequence = [int]$fixtureRow[0][2]
    Assert-True ($fixtureSequence -gt $installFiles -and $fixtureSequence -lt $installFinalize) "failing action runs after files and before finalize"
}

function Assert-FailingFixtureContract {
    param([Parameter(Mandatory = $true)][string]$Path)
    $handle = Open-MsiDatabase -Path $Path
    try {
        Assert-FixtureCustomAction $handle.Database
        Assert-FixtureSequence $handle.Database
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
