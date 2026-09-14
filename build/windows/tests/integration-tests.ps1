[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")][string]$Configuration = "Release",
    [switch]$ConfirmDisposableTestMachine,
    [switch]$ManualProductionSmoke,
    [switch]$ConfirmDisposableCleanVm,
    [string]$TranscriptPath = ""
)

$ErrorActionPreference = "Stop"
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$projectPath = Join-Path $repositoryRoot "build\windows\miru-agent.wixproj"
$fixtureSource = Join-Path $PSScriptRoot "integration-test.wxs"
$binDir = Join-Path $repositoryRoot "target\x86_64-pc-windows-msvc\$($Configuration.ToLowerInvariant())"
$artifactsRoot = Join-Path ([IO.Path]::GetTempPath()) ("miru-integration-tests-" + [Guid]::NewGuid().ToString("N"))
$deterministicLogs = Join-Path $repositoryRoot "build\windows\artifacts\package-tests\logs"
$sessionLogs = Join-Path $deterministicLogs ([Guid]::NewGuid().ToString("N"))
$programDataRoot = Join-Path $env:ProgramData "Miru"
$logsRoot = Join-Path $programDataRoot "logs"
$protectedRoots = @($programDataRoot, $logsRoot, (Join-Path $programDataRoot "auth"), (Join-Path $programDataRoot "tmp"))
$representativeFiles = New-Object System.Collections.ArrayList
$markerPath = Join-Path $programDataRoot "rollback-payload.txt"
$sentinelPath = Join-Path $programDataRoot "integration-sentinel.txt"
$customerLogPath = Join-Path $logsRoot "customer-owned.log"
$customerLogContents = "customer-owned-log-retain"
$agentPath = Join-Path ([Environment]::GetEnvironmentVariable("ProgramW6432", "Process")) "Miru\Agent\miru-agent.exe"
$upgradeCode = "{B5ED0336-5F14-4308-A667-3CE8CDEF7D48}"
$fixtureProducts = @(
    "{B7AFDD4E-E6DB-4ED9-8C34-F318A04486B1}",
    "{3CE73709-ECE4-48A5-B7E7-1AC13C5EF30A}",
    "{4E72A894-00B5-433B-A445-C2CFD7FCF432}"
)
$testUser = "MiruMsiTestUser"
$testPassword = "M!ru-" + [Guid]::NewGuid().ToString("N") + "-9a"
$createdUser = $false
$failureEvidence = New-Object System.Collections.ArrayList
$cleanupFailures = New-Object System.Collections.ArrayList
$integrationFailure = $null
$transcriptStarted = $false

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

function Assert-Elevated64BitWindows {
    Assert-True ([Environment]::Is64BitOperatingSystem) "64-bit Windows is required"
    Assert-True ([Environment]::Is64BitProcess) "64-bit Windows PowerShell is required"
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    Assert-True ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) "an elevated Administrator session is required"
}

function Get-RelatedProducts {
    $installer = $null
    try {
        $installer = New-Object -ComObject WindowsInstaller.Installer
        $products = $installer.GetType().InvokeMember("RelatedProducts", "GetProperty", $null, $installer, @($upgradeCode))
        return @($products | ForEach-Object { [string]$_ })
    }
    finally {
        if ($null -ne $installer) { [Runtime.InteropServices.Marshal]::ReleaseComObject($installer) | Out-Null }
    }
}

function Assert-InstalledAllowlistSafe {
    param([switch]$RequireClean)
    $related = @(Get-RelatedProducts)
    if ($RequireClean -and $related.Count -ne 0) {
        throw "Manual production smoke requires a clean VM with no product matching $upgradeCode."
    }
    foreach ($product in $related) {
        if ($fixtureProducts -notcontains $product.ToUpperInvariant()) {
            throw "Refusing mutation: installed related ProductCode $product is outside the committed fixture allowlist."
        }
    }
    return $related
}

function Invoke-Msi {
    param([string[]]$Arguments, [string]$Name, [int[]]$AllowedExitCodes)
    New-Item -ItemType Directory -Path $sessionLogs -Force | Out-Null
    $logPath = Join-Path $sessionLogs "$Name.log"
    $fullArguments = @($Arguments) + @("/qn", "/norestart", "/l*v", ('"{0}"' -f $logPath))
    Write-Host "MSI log ($Name): $logPath"
    $process = Start-Process -FilePath "msiexec.exe" -ArgumentList $fullArguments -Wait -PassThru
    if ($AllowedExitCodes -notcontains $process.ExitCode) {
        [void]$failureEvidence.Add($logPath)
        throw "msiexec $Name returned $($process.ExitCode); log: $logPath"
    }
    return $process.ExitCode
}

function Complete-IntegrationRun {
    param([Management.Automation.ErrorRecord]$PrimaryFailure)
    foreach ($cleanupFailure in $cleanupFailures) {
        Write-Host "Cleanup failure: $cleanupFailure" -ForegroundColor Red
    }
    if ($null -ne $PrimaryFailure) { throw $PrimaryFailure }
    if ($cleanupFailures.Count -ne 0) { throw "Integration cleanup failed; see cleanup diagnostics above." }
}

function Build-IntegrationPackage {
    param([string]$Version, [string]$ProductCode, [string]$Marker)
    $output = Join-Path $artifactsRoot $Version
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    $payload = Join-Path $output "rollback-payload.txt"
    [IO.File]::WriteAllText($payload, $Marker, [Text.Encoding]::ASCII)
    & dotnet build $projectPath --no-restore --configuration Release "-p:Platform=x64" "-p:Version=$Version" "-p:BinDir=$binDir" "-p:ProductCode=$ProductCode" "-p:TestWixSource=$fixtureSource" "-p:FixturePayloadPath=$payload" "-p:OutputPath=$output\" "-p:IntermediateOutputPath=$output\obj\" | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "integration package build failed for $Version" }
    $package = Get-ChildItem -LiteralPath $output -Filter "*.msi" -Recurse -File | Where-Object { $_.FullName -notmatch '\\obj\\' } | Select-Object -First 1
    if ($null -eq $package) { throw "integration package missing for $Version" }
    return $package.FullName
}

function Assert-NoService {
    $service = Get-Service -Name "MiruAgent" -ErrorAction SilentlyContinue
    Assert-True ($null -eq $service) "MiruAgent service must not exist"
}

function Assert-OneRegistration {
    param([string]$ExpectedProduct)
    $related = @(Get-RelatedProducts)
    Assert-Equal 1 $related.Count "exactly one related product registration"
    Assert-Equal $ExpectedProduct.ToUpperInvariant() $related[0].ToUpperInvariant() "registered ProductCode"
}

function Test-ArpProductCode {
    param([string]$ProductCode)
    $paths = @(
        "Registry::HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode",
        "Registry::HKEY_LOCAL_MACHINE\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode"
    )
    return (@($paths | Where-Object { Test-Path -LiteralPath $_ })).Count -ne 0
}

function Get-MiruArpProducts {
    $roots = @(
        "Registry::HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        "Registry::HKEY_LOCAL_MACHINE\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"
    )
    return @($roots | ForEach-Object {
        Get-ChildItem -LiteralPath $_ -ErrorAction SilentlyContinue |
            ForEach-Object { Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue }
    } | Where-Object { $_.DisplayName -eq "Miru Agent" })
}

function Get-MsiIdentity {
    param([string]$Path)
    $installer = $null
    $database = $null
    try {
        $installer = New-Object -ComObject WindowsInstaller.Installer
        $database = $installer.GetType().InvokeMember("OpenDatabase", "InvokeMethod", $null, $installer, @($Path, 0))
        $values = @{}
        foreach ($name in @("ProductName", "ProductVersion", "ProductCode", "UpgradeCode")) {
            $view = $null
            $record = $null
            try {
                $query = "SELECT ``Value`` FROM ``Property`` WHERE ``Property``='$name'"
                $view = $database.GetType().InvokeMember("OpenView", "InvokeMethod", $null, $database, @($query))
                $view.GetType().InvokeMember("Execute", "InvokeMethod", $null, $view, $null) | Out-Null
                $record = $view.GetType().InvokeMember("Fetch", "InvokeMethod", $null, $view, $null)
                $values[$name] = $record.GetType().InvokeMember("StringData", "GetProperty", $null, $record, @(1))
            }
            finally {
                if ($null -ne $record) { [Runtime.InteropServices.Marshal]::ReleaseComObject($record) | Out-Null }
                if ($null -ne $view) {
                    $view.GetType().InvokeMember("Close", "InvokeMethod", $null, $view, $null) | Out-Null
                    [Runtime.InteropServices.Marshal]::ReleaseComObject($view) | Out-Null
                }
            }
        }
        return [pscustomobject]$values
    }
    finally {
        if ($null -ne $database) { [Runtime.InteropServices.Marshal]::ReleaseComObject($database) | Out-Null }
        if ($null -ne $installer) { [Runtime.InteropServices.Marshal]::ReleaseComObject($installer) | Out-Null }
    }
}

function Get-MsiQueryRows {
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

function Assert-FailingFixtureContract {
    param([string]$Path)
    $installer = $null
    $database = $null
    try {
        $installer = New-Object -ComObject WindowsInstaller.Installer
        $database = $installer.GetType().InvokeMember("OpenDatabase", "InvokeMethod", $null, $installer, @($Path, 0))
        $tables = @(Get-MsiQueryRows $database "SELECT ``Name`` FROM ``_Tables``" 1)
        Assert-Equal 1 (@($tables | Where-Object { $_[0] -eq "CustomAction" })).Count "fixture custom action table"
        $actions = @(Get-MsiQueryRows $database "SELECT ``Action``, ``Type``, ``Source``, ``Target`` FROM ``CustomAction``" 4)
        $action = @($actions | Where-Object { $_[0] -eq "FailUpgradeForTest" })
        Assert-Equal 1 $action.Count "one failing fixture custom action"
        Assert-Equal 3106 ([int]$action[0][1]) "deferred no-impersonate checked Type 34 action"
        Assert-Equal "SystemFolder" $action[0][2] "failing action SystemFolder source"
        Assert-Equal "[SystemFolder]cmd.exe /d /c exit /b 1" $action[0][3] "isolated cmd failure command"
        $sequence = @(Get-MsiQueryRows $database "SELECT ``Action``, ``Condition``, ``Sequence`` FROM ``InstallExecuteSequence``" 3)
        $fixtureRow = @($sequence | Where-Object { $_[0] -eq "FailUpgradeForTest" })
        Assert-Equal 1 $fixtureRow.Count "one failing action sequence row"
        Assert-Equal "FAIL_UPGRADE_FOR_TEST=1" $fixtureRow[0][1] "failing action condition"
        $installFiles = [int](@($sequence | Where-Object { $_[0] -eq "InstallFiles" })[0][2])
        $installFinalize = [int](@($sequence | Where-Object { $_[0] -eq "InstallFinalize" })[0][2])
        $fixtureSequence = [int]$fixtureRow[0][2]
        Assert-True ($fixtureSequence -gt $installFiles -and $fixtureSequence -lt $installFinalize) "failing action runs after files and before finalize"
    }
    finally {
        if ($null -ne $database) { [Runtime.InteropServices.Marshal]::ReleaseComObject($database) | Out-Null }
        if ($null -ne $installer) { [Runtime.InteropServices.Marshal]::ReleaseComObject($installer) | Out-Null }
    }
}

function Assert-ProtectedAcl {
    param([string]$LiteralPath, [string]$Label)
    $acl = Get-Acl -LiteralPath $LiteralPath
    Assert-Equal "S-1-5-18" $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value "$Label owner is SYSTEM"
    Assert-True $acl.AreAccessRulesProtected "$Label DACL inheritance is disabled"
    Assert-Equal 2 @($acl.Access).Count "only two total $Label ACEs"
    $explicit = @($acl.Access | Where-Object { -not $_.IsInherited })
    Assert-Equal 2 $explicit.Count "only two explicit $Label ACEs"
    $expectedSids = @("S-1-5-18", "S-1-5-32-544")
    $actualSids = @()
    foreach ($rule in $explicit) {
        $sid = $rule.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
        $actualSids += $sid
        Assert-Equal "Allow" $rule.AccessControlType.ToString() "ACE type"
        Assert-Equal ([int][Security.AccessControl.FileSystemRights]::FullControl) ([int]$rule.FileSystemRights) "ACE grants exactly full control"
        $expectedInheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit
        Assert-Equal ([int]$expectedInheritance) ([int]$rule.InheritanceFlags) "ACE inherits to containers and files"
        Assert-Equal ([int][Security.AccessControl.PropagationFlags]::None) ([int]$rule.PropagationFlags) "ACE has no propagation restriction"
    }
    Assert-Equal (($expectedSids | Sort-Object) -join "`n") (($actualSids | Sort-Object) -join "`n") "$Label ACE identities"
    $icacls = @(& icacls.exe $LiteralPath 2>&1)
    Assert-Equal 0 $LASTEXITCODE "icacls can inspect $Label"
    return ($icacls | Out-String)
}

function Assert-ProtectedAcls {
    foreach ($path in $protectedRoots) {
        Assert-ProtectedAcl -LiteralPath $path -Label $path | Out-Null
    }
}

function Add-PermissiveAces {
    param([string]$OwnerSid = "")
    foreach ($path in $protectedRoots) {
        New-Item -ItemType Directory -Path $path -Force | Out-Null
        $permissive = New-Object Security.AccessControl.DirectorySecurity
        $permissive.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;WD)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
        Set-Acl -LiteralPath $path -AclObject $permissive
        if ($OwnerSid) {
            & icacls.exe $path /setowner "*$OwnerSid" | Out-Null
            Assert-Equal 0 $LASTEXITCODE "hostile owner assigned to $path"
        }
        $acl = Get-Acl -LiteralPath $path
        Assert-True $acl.AreAccessRulesProtected "$path hostile DACL is protected"
        $everyone = @($acl.Access | Where-Object { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -eq "S-1-1-0" })
        Assert-Equal 1 $everyone.Count "$path has one permissive Everyone ACE"
        Assert-Equal "Allow" $everyone[0].AccessControlType.ToString() "$path permits Everyone"
        Assert-Equal ([int][Security.AccessControl.FileSystemRights]::FullControl) ([int]$everyone[0].FileSystemRights) "$path grants full control"
        Assert-Equal 3 ([int]$everyone[0].InheritanceFlags) "$path permissive ACE inherits to files and directories"
        Assert-Equal 0 ([int]$everyone[0].PropagationFlags) "$path permissive ACE has no propagation restriction"
        Assert-True (-not $everyone[0].IsInherited) "$path permissive ACE is explicit"
        if ($OwnerSid) { Assert-Equal $OwnerSid $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value "$path hostile owner verified" }
    }
}

function Assert-CustomerStateRetained {
    param([string]$Stage)
    Assert-Equal "retain-me" ([IO.File]::ReadAllText($sentinelPath)) "$Stage keeps sentinel"
    Assert-True (Test-Path -LiteralPath $logsRoot -PathType Container) "$Stage keeps logs directory"
    Assert-Equal $customerLogContents ([IO.File]::ReadAllText($customerLogPath)) "$Stage keeps customer log"
    foreach ($file in $representativeFiles) {
        Assert-Equal $file.Contents ([IO.File]::ReadAllText($file.Path)) "$Stage keeps $($file.Path)"
    }
}

function New-RepresentativeSecrets {
    param([string]$Stage)
    foreach ($path in $protectedRoots) {
        $file = [pscustomobject]@{
            Parent = $path
            Path = Join-Path $path ("representative-$Stage-" + [Guid]::NewGuid().ToString("N") + ".txt")
            Contents = "representative-$Stage-" + [Guid]::NewGuid().ToString("N")
            CreatePath = Join-Path $path ("non-admin-" + [Guid]::NewGuid().ToString("N") + ".txt")
        }
        [IO.File]::WriteAllText($file.Path, $file.Contents)
        Assert-True (Test-Path -LiteralPath $file.Path -PathType Leaf) "representative read target exists"
        Assert-True (-not (Test-Path -LiteralPath $file.CreatePath)) "representative create target is absent"
        $acl = Get-Acl -LiteralPath $file.Path
        Assert-True (-not $acl.AreAccessRulesProtected) "$($file.Path) inherits its DACL"
        Assert-Equal 2 @($acl.Access).Count "$($file.Path) has only trusted inherited ACEs"
        $sids = @($acl.Access | ForEach-Object {
            Assert-True $_.IsInherited "representative file ACE is inherited"
            Assert-Equal "Allow" $_.AccessControlType.ToString() "representative file allow ACE"
            Assert-Equal ([int][Security.AccessControl.FileSystemRights]::FullControl) ([int]$_.FileSystemRights) "representative file full control"
            $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
        })
        Assert-Equal "S-1-5-18,S-1-5-32-544" (($sids | Sort-Object) -join ",") "representative file trusted identities"
        [void]$representativeFiles.Add($file)
        $file
    }
}

function Invoke-NonAdminProbe {
    param([string]$Stage)
    $files = @(New-RepresentativeSecrets -Stage $Stage)
    $probeRoot = Join-Path $artifactsRoot ("probe-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $probeRoot -Force | Out-Null
    & icacls.exe $probeRoot /grant ("$testUser`:(OI)(CI)F") | Out-Null
    Assert-Equal 0 $LASTEXITCODE "non-admin probe directory permissions"
    $scriptPath = Join-Path $probeRoot "probe.ps1"
    $manifestPath = Join-Path $probeRoot "manifest.json"
    $resultPath = Join-Path $probeRoot "result.json"
    $manifest = @{ Files = $files; ProbeRoot = $probeRoot; ControlPath = (Join-Path $probeRoot "control.txt") }
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4))
    $probe = @'
param([string]$ManifestPath, [string]$ResultPath)
$ErrorActionPreference = "Stop"
function Get-AccessOutcome {
    param([scriptblock]$Action)
    try { & $Action | Out-Null; return "Allowed" }
    catch {
        $cause = $_.Exception.GetBaseException()
        if ($cause -is [UnauthorizedAccessException] -or ($cause -is [ComponentModel.Win32Exception] -and $cause.NativeErrorCode -eq 5)) {
            return "AccessDenied"
        }
        throw
    }
}
function Set-PermissiveDacl {
    param([string]$Path)
    # Default All also requests an audit update requiring a separate privilege.
    # Persist clears modification flags, so each attempt needs a fresh descriptor.
    $replacement = New-Object Security.AccessControl.DirectorySecurity
    $replacement.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;WD)", [Security.AccessControl.AccessControlSections]::Access)
    [IO.Directory]::SetAccessControl($Path, $replacement)
}
$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
$control = [pscustomobject]@{
    Create = Get-AccessOutcome { [IO.File]::WriteAllText($manifest.ControlPath, "probe-control") }
    Read = Get-AccessOutcome {
        if ([IO.File]::ReadAllText($manifest.ControlPath) -ne "probe-control") { throw "Probe control mismatch" }
    }
    Regrant = Get-AccessOutcome { Set-PermissiveDacl $manifest.ProbeRoot }
}
$results = @($manifest.Files | ForEach-Object {
    $file = $_
    [pscustomobject]@{
        Path = $file.Path
        Read = Get-AccessOutcome { [IO.File]::ReadAllText($file.Path) }
        Create = Get-AccessOutcome { [IO.File]::WriteAllText($file.CreatePath, "unexpected") }
        Regrant = Get-AccessOutcome { Set-PermissiveDacl $file.Parent }
    }
})
@{
    Sid = $identity.User.Value
    Administrator = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    Control = $control
    Results = $results
} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $ResultPath
'@
    [IO.File]::WriteAllText($scriptPath, $probe)
    $securePassword = ConvertTo-SecureString $testPassword -AsPlainText -Force
    $credential = New-Object Management.Automation.PSCredential("$env:COMPUTERNAME\$testUser", $securePassword)
    $process = Start-Process -FilePath "powershell.exe" -Credential $credential -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", ('"{0}"' -f $scriptPath), ('"{0}"' -f $manifestPath), ('"{0}"' -f $resultPath)) -Wait -PassThru
    Assert-Equal 0 $process.ExitCode "non-admin probe process"
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    Assert-Equal $testUserSid $result.Sid "probe runs as the temporary account"
    Assert-Equal $false $result.Administrator "probe account is not an administrator"
    foreach ($operation in @("Read", "Create", "Regrant")) {
        Assert-Equal "Allowed" $result.Control.$operation "probe control $operation succeeds"
    }
    Assert-Equal $files.Count @($result.Results).Count "one result per protected directory"
    foreach ($file in $files) {
        $entry = @($result.Results | Where-Object { $_.Path -eq $file.Path })
        Assert-Equal 1 $entry.Count "one result for $($file.Path)"
        foreach ($operation in @("Read", "Create", "Regrant")) {
            Assert-Equal "AccessDenied" $entry[0].$operation "$Stage $operation denied for $($file.Parent)"
        }
        Assert-True (-not (Test-Path -LiteralPath $file.CreatePath)) "non-admin child was not created"
    }
    Assert-ProtectedAcls
    Assert-CustomerStateRetained "$Stage after non-admin probes"
}

function Invoke-DirectProvisionCheck {
    $stdout = Join-Path $artifactsRoot "provision-check.stdout.txt"
    $stderr = Join-Path $artifactsRoot "provision-check.stderr.txt"
    $process = Start-Process -FilePath $agentPath -ArgumentList @("provision", "--check") -RedirectStandardOutput $stdout -RedirectStandardError $stderr -Wait -PassThru
    Assert-Equal 3 $process.ExitCode "fresh install provision check"
    $output = ([IO.File]::ReadAllText($stdout) + [IO.File]::ReadAllText($stderr))
    Assert-True (-not [string]::IsNullOrWhiteSpace($output)) "provision check preserves useful output"
}

function Invoke-ManualSmoke {
    if (-not $ConfirmDisposableCleanVm -or [string]::IsNullOrWhiteSpace($TranscriptPath)) {
        throw "ManualProductionSmoke requires -ConfirmDisposableCleanVm and -TranscriptPath."
    }
    Assert-InstalledAllowlistSafe -RequireClean | Out-Null
    New-Item -ItemType Directory -Path $artifactsRoot -Force | Out-Null
    $arpProducts = @(Get-MiruArpProducts)
    Assert-Equal 0 $arpProducts.Count "clean VM has no Miru Agent registration"
    Assert-True (-not (Test-Path -LiteralPath $programDataRoot)) "clean VM has no pre-existing Miru ProgramData"
    Start-Transcript -LiteralPath $TranscriptPath -Force | Out-Null
    $script:transcriptStarted = $true
    Write-Host "Smoke started: $(Get-Date -Format o)"
    Get-ComputerInfo | Select-Object WindowsProductName, WindowsVersion, OsBuildNumber
    $v1 = Join-Path $repositoryRoot "build\windows\artifacts\package-tests\v1\miru-agent-1.0.0.msi"
    $v2 = Join-Path $repositoryRoot "build\windows\artifacts\package-tests\v2\miru-agent-1.1.0.msi"
    Assert-True (Test-Path -LiteralPath $v1 -PathType Leaf) "production v1 package exists"
    Assert-True (Test-Path -LiteralPath $v2 -PathType Leaf) "production v2 package exists"
    Get-FileHash -Algorithm SHA256 -LiteralPath $v1
    Get-FileHash -Algorithm SHA256 -LiteralPath $v2
    Get-MsiIdentity $v1 | Format-List
    Get-MsiIdentity $v2 | Format-List
    New-Item -ItemType Directory -Path $logsRoot -Force | Out-Null
    [IO.File]::WriteAllText($sentinelPath, "retain-me")
    [IO.File]::WriteAllText($customerLogPath, $customerLogContents)
    Add-PermissiveAces
    $installResult = Invoke-Msi @("/i", ('"{0}"' -f $v1)) "manual-install" @(0, 3010)
    Write-Host "Manual install reboot result: $installResult"
    Assert-ProtectedAcls
    New-RepresentativeSecrets -Stage "manual-install" | Out-Null
    Assert-CustomerStateRetained "manual install"
    Assert-NoService
    Write-Host "PASS manual v1 install repairs root/logs/auth/tmp ACLs and retains customer state"
    Add-PermissiveAces
    $maintenanceResult = Invoke-Msi @("/i", ('"{0}"' -f $v1), "REINSTALL=ALL", "REINSTALLMODE=vomus") "manual-maintenance" @(0, 3010)
    Write-Host "Manual maintenance reboot result: $maintenanceResult"
    Assert-CustomerStateRetained "manual maintenance"
    Assert-ProtectedAcls
    Write-Host "PASS manual maintenance repairs root/logs/auth/tmp ACLs and retains customer state"
    Add-PermissiveAces
    $upgradeResult = Invoke-Msi @("/i", ('"{0}"' -f $v2)) "manual-upgrade" @(0, 3010)
    Write-Host "Manual upgrade reboot result: $upgradeResult"
    Assert-CustomerStateRetained "manual upgrade"
    Assert-ProtectedAcls
    Assert-NoService
    Write-Host "PASS manual upgrade repairs root/logs/auth/tmp ACLs and retains customer state"
    $installed = @(Get-RelatedProducts)
    Assert-Equal 1 $installed.Count "one production product before uninstall"
    $uninstallResult = Invoke-Msi @("/x", $installed[0]) "manual-uninstall" @(0, 3010)
    Write-Host "Manual uninstall reboot result: $uninstallResult"
    Assert-True (-not (Test-Path -LiteralPath $agentPath)) "production executable removed"
    Assert-CustomerStateRetained "manual uninstall"
    Assert-ProtectedAcls
    Assert-NoService
    Write-Host "PASS manual uninstall retains protected root/logs/auth/tmp customer state"
    Write-Host "Smoke completed: $(Get-Date -Format o)"
    Write-Host "PASS manual production smoke; sentinel intentionally retained at $sentinelPath"
}

if (-not $ManualProductionSmoke -and -not $ConfirmDisposableTestMachine) {
    if (Test-Path -LiteralPath $programDataRoot) {
        throw "Refusing mutation of pre-existing $programDataRoot; normal integration requires -ConfirmDisposableTestMachine."
    }
    throw "Normal integration is destructive and requires -ConfirmDisposableTestMachine on a disposable test machine."
}

Assert-Elevated64BitWindows
if ($ManualProductionSmoke) {
    try { Invoke-ManualSmoke }
    catch { $script:integrationFailure = $_ }
    finally {
        try {
            if ($transcriptStarted) { Stop-Transcript | Out-Null }
        }
        catch {
            [void]$cleanupFailures.Add("transcript stop failed: $($_.Exception.Message)")
        }
        try {
            if (Test-Path -LiteralPath $artifactsRoot) { Remove-Item -LiteralPath $artifactsRoot -Recurse -Force -ErrorAction Stop }
        }
        catch {
            [void]$cleanupFailures.Add("temporary artifact deletion failed: $($_.Exception.Message)")
        }
    }
    Complete-IntegrationRun $integrationFailure
    exit 0
}

$initialRelated = @(Assert-InstalledAllowlistSafe)
$existingUser = Get-LocalUser -Name $testUser -ErrorAction SilentlyContinue
if ($null -ne $existingUser) {
    throw "Refusing mutation: the named integration account $testUser already exists."
}
New-Item -ItemType Directory -Path $artifactsRoot -Force | Out-Null
try {
    foreach ($product in $initialRelated) {
        Invoke-Msi @("/x", $product) "preclean-$($product.Trim('{}'))" @(0, 3010, 1605) | Out-Null
    }

    Assert-True (Test-Path -LiteralPath (Join-Path $binDir "miru-agent.exe") -PathType Leaf) "real x64 miru-agent.exe exists"
    $v1 = Build-IntegrationPackage "1.0.0" $fixtureProducts[0] "fixture-v1"
    $v2 = Build-IntegrationPackage "1.1.0" $fixtureProducts[1] "fixture-v2"
    $v3 = Build-IntegrationPackage "1.2.0" $fixtureProducts[2] "fixture-v3"
    Assert-FailingFixtureContract $v3

    $secureTestPassword = ConvertTo-SecureString $testPassword -AsPlainText -Force
    New-LocalUser -Name $testUser -Password $secureTestPassword | Out-Null
    $createdUser = $true
    $testUserSid = (Get-LocalUser -Name $testUser).SID.Value
    New-Item -ItemType Directory -Path $logsRoot -Force | Out-Null
    [IO.File]::WriteAllText($sentinelPath, "retain-me")
    [IO.File]::WriteAllText($customerLogPath, $customerLogContents)
    Add-PermissiveAces -OwnerSid $testUserSid

    Invoke-Msi @("/i", ('"{0}"' -f $v1)) "fixture-v1" @(0, 3010) | Out-Null
    Assert-True (Test-Path -LiteralPath $agentPath -PathType Leaf) "v1 executable installed"
    Assert-Equal "fixture-v1" ([IO.File]::ReadAllText($markerPath)) "v1 rollback marker"
    Assert-OneRegistration $fixtureProducts[0]
    Assert-True (Test-ArpProductCode $fixtureProducts[0]) "v1 installer metadata registered"
    Assert-CustomerStateRetained "initial install"
    Assert-ProtectedAcls
    # Fresh representative files exercise inheritance without creating real
    # provisioning state or relying on files secured by an earlier operation.
    Invoke-NonAdminProbe -Stage "install"
    Assert-NoService
    Invoke-DirectProvisionCheck
    Write-Host "PASS initial install, ACL correction, denial, direct provision check exit 3, and no service"

    $v1Hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $agentPath).Hash
    Add-PermissiveAces -OwnerSid $testUserSid
    Invoke-Msi @("/i", ('"{0}"' -f $v1), "REINSTALL=ALL", "REINSTALLMODE=vomus") "fixture-v1-maintenance" @(0, 3010) | Out-Null
    Assert-Equal "fixture-v1" ([IO.File]::ReadAllText($markerPath)) "maintenance keeps v1 marker"
    Assert-Equal $v1Hash (Get-FileHash -Algorithm SHA256 -LiteralPath $agentPath).Hash "maintenance keeps v1 hash"
    Assert-CustomerStateRetained "maintenance"
    Assert-OneRegistration $fixtureProducts[0]
    Assert-ProtectedAcls
    Invoke-NonAdminProbe -Stage "maintenance"
    Assert-NoService
    Write-Host "PASS same-MSI maintenance repairs ACL and retains v1 state"

    Add-PermissiveAces -OwnerSid $testUserSid
    Invoke-Msi @("/i", ('"{0}"' -f $v2)) "fixture-v2-upgrade" @(0, 3010) | Out-Null
    Assert-OneRegistration $fixtureProducts[1]
    Assert-True (Test-ArpProductCode $fixtureProducts[1]) "v2 installer metadata registered"
    Assert-Equal "fixture-v2" ([IO.File]::ReadAllText($markerPath)) "v2 marker"
    Assert-CustomerStateRetained "upgrade"
    $v2Hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $agentPath).Hash
    Assert-ProtectedAcls
    Invoke-NonAdminProbe -Stage "upgrade"
    Assert-NoService
    Write-Host "PASS v1-to-v2 upgrade repairs ACL and registers one product"

    Invoke-Msi @("/i", ('"{0}"' -f $v1)) "fixture-downgrade" @(1603) | Out-Null
    Assert-OneRegistration $fixtureProducts[1]
    Assert-Equal "fixture-v2" ([IO.File]::ReadAllText($markerPath)) "downgrade leaves v2 marker"
    Assert-Equal $v2Hash (Get-FileHash -Algorithm SHA256 -LiteralPath $agentPath).Hash "downgrade leaves v2 executable"
    Assert-CustomerStateRetained "downgrade rejection"
    Assert-ProtectedAcls
    Write-Host "PASS downgrade rejected with v2 intact"

    Invoke-Msi @("/i", ('"{0}"' -f $v3), "FAIL_UPGRADE_FOR_TEST=1") "fixture-v3-rollback" @(1603) | Out-Null
    Assert-OneRegistration $fixtureProducts[1]
    Assert-Equal "fixture-v2" ([IO.File]::ReadAllText($markerPath)) "rollback restores v2 marker"
    Assert-Equal $v2Hash (Get-FileHash -Algorithm SHA256 -LiteralPath $agentPath).Hash "rollback restores v2 executable"
    Assert-CustomerStateRetained "rollback"
    Assert-ProtectedAcls
    Write-Host "PASS failed v3 upgrade rolls back registration, hash, marker, sentinel, and DACL"

    Invoke-Msi @("/x", $fixtureProducts[1]) "fixture-v2-uninstall" @(0, 3010) | Out-Null
    Assert-Equal 0 (@(Get-RelatedProducts)).Count "product registration removed"
    Assert-True (-not (Test-ArpProductCode $fixtureProducts[1])) "installer-owned registry metadata removed"
    Assert-True (-not (Test-Path -LiteralPath $agentPath)) "executable removed"
    Assert-True (-not (Test-Path -LiteralPath $markerPath)) "test marker removed"
    Assert-True (Test-Path -LiteralPath $programDataRoot -PathType Container) "ProgramData retained"
    Assert-CustomerStateRetained "uninstall"
    Assert-ProtectedAcls
    Assert-NoService
    Write-Host "PASS uninstall removes package state and retains protected customer state"
}
catch {
    $script:integrationFailure = $_
    Write-Host "Integration failure: $($integrationFailure.Exception.Message)" -ForegroundColor Red
    try {
        Write-Host "Related products: $(@(Get-RelatedProducts) -join ', ')"
        if (Test-Path -LiteralPath $programDataRoot) { & icacls.exe $programDataRoot }
        if (Test-Path -LiteralPath $agentPath) { Get-FileHash -Algorithm SHA256 -LiteralPath $agentPath }
        if (Test-Path -LiteralPath $markerPath) { Write-Host "Marker: $([IO.File]::ReadAllText($markerPath))" }
    }
    catch {
        Write-Host "Failure evidence collection also failed: $($_.Exception.Message)" -ForegroundColor Red
    }
}
finally {
    try {
        foreach ($product in @(Get-RelatedProducts)) {
            if ($fixtureProducts -contains $product.ToUpperInvariant()) {
                Invoke-Msi @("/x", $product) "cleanup-$($product.Trim('{}'))" @(0, 3010, 1605) | Out-Null
            }
        }
        foreach ($product in @(Get-RelatedProducts)) {
            if ($fixtureProducts -contains $product.ToUpperInvariant()) {
                [void]$cleanupFailures.Add("fixture ProductCode $($product.ToUpperInvariant()) remains installed after cleanup")
            }
        }
    }
    catch {
        [void]$cleanupFailures.Add("product cleanup failed: $($_.Exception.Message)")
    }
    if ($createdUser) {
        try {
            Remove-LocalUser -Name $testUser -ErrorAction Stop
        }
        catch {
            [void]$cleanupFailures.Add("temporary user deletion failed: $($_.Exception.Message)")
        }
        try {
            $remainingUser = Get-LocalUser -Name $testUser -ErrorAction SilentlyContinue
            if ($null -ne $remainingUser) {
                [void]$cleanupFailures.Add("temporary user $testUser still exists after deletion")
            }
        }
        catch {
            [void]$cleanupFailures.Add("temporary user deletion verification failed: $($_.Exception.Message)")
        }
    }
    try {
        if (Test-Path -LiteralPath $markerPath) { Remove-Item -LiteralPath $markerPath -Force -ErrorAction Stop }
    }
    catch {
        [void]$cleanupFailures.Add("fixture marker deletion failed: $($_.Exception.Message)")
    }
    try {
        if (Test-Path -LiteralPath $artifactsRoot) { Remove-Item -LiteralPath $artifactsRoot -Recurse -Force -ErrorAction Stop }
    }
    catch {
        [void]$cleanupFailures.Add("temporary artifact deletion failed: $($_.Exception.Message)")
    }
}

Complete-IntegrationRun $integrationFailure
