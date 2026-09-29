#Requires -Version 5.1
Set-StrictMode -Version Latest

$script:WindowsTestRoot = $PSScriptRoot

function Assert-Elevated64BitWindows {
    Assert-True ([Environment]::Is64BitOperatingSystem) "64-bit Windows is required"
    Assert-True ([Environment]::Is64BitProcess) "64-bit Windows PowerShell is required"
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    Assert-True ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) "an elevated Administrator session is required"
}

function Assert-InstalledAllowlistSafe {
    $related = @(Get-RelatedProducts)
    foreach ($product in $related) {
        if (-not (Test-FixtureProduct $product)) {
            throw "Refusing mutation: installed related ProductCode $product is outside the committed fixture allowlist."
        }
    }
    return $related
}

function Get-RelatedProducts {
    $installer = $null
    try {
        $installer = New-Object -ComObject WindowsInstaller.Installer
        $products = $installer.GetType().InvokeMember("RelatedProducts", "GetProperty", $null, $installer, @($MsiUpgradeCode))
        return @($products | ForEach-Object { [string]$_ })
    }
    finally {
        if ($null -ne $installer) { [Runtime.InteropServices.Marshal]::ReleaseComObject($installer) | Out-Null }
    }
}

function Test-FixtureProduct {
    param([Parameter(Mandatory = $true)][string]$ProductCode)
    return $fixtureProducts -contains $ProductCode.ToUpperInvariant()
}

function Invoke-IntegrationRun {
    try { Invoke-IntegrationLifecycle }
    catch {
        $script:integrationFailure = $_
        Write-FailureEvidence $integrationFailure
    }
    finally {
        Remove-FixtureProducts
        Remove-TestUser
        Remove-TestFiles
    }
    Complete-IntegrationRun $integrationFailure
}

function Invoke-IntegrationLifecycle {
    Remove-LeftoverFixtures
    $packages = Build-LifecyclePackages
    New-TestUser
    Initialize-CustomerState
    Invoke-InstallStage $packages
    Invoke-MaintenanceStage $packages
    Invoke-UpgradeStage $packages
    Invoke-DowngradeStage $packages
    Invoke-RollbackStage $packages
    Invoke-UninstallStage
}

function Remove-LeftoverFixtures {
    foreach ($product in $initialRelated) {
        Uninstall-Msi $product "preclean-$($product.Trim('{}'))" @(0, 3010, 1605)
    }
}

function Uninstall-Msi {
    param(
        [Parameter(Mandatory = $true)][string]$ProductCode,
        [Parameter(Mandatory = $true)][string]$Name,
        [int[]]$AllowedExitCodes = @(0, 3010)
    )
    Invoke-Msi @("/x", $ProductCode) $Name $AllowedExitCodes | Out-Null
}

function Invoke-Msi {
    param(
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][int[]]$AllowedExitCodes
    )
    Initialize-Directory $sessionLogs | Out-Null
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

function Build-LifecyclePackages {
    Assert-True (Test-Path -LiteralPath (Join-Path $binDir "miru-agent.exe") -PathType Leaf) "real x64 miru-agent.exe exists"
    $packages = @{
        V1 = Build-IntegrationPackage "1.0.0" $fixtureProducts[0] "fixture-v1"
        V2 = Build-IntegrationPackage "1.1.0" $fixtureProducts[1] "fixture-v2"
        V3 = Build-IntegrationPackage "1.2.0" $fixtureProducts[2] "fixture-v3"
    }
    Assert-FailingFixtureContract $packages.V3
    return $packages
}

function Build-IntegrationPackage {
    param(
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)][string]$ProductCode,
        [Parameter(Mandatory = $true)][string]$Marker
    )
    $output = Initialize-Directory (Join-Path $artifactsRoot $Version)
    $payload = Join-Path $output "rollback-payload.txt"
    [IO.File]::WriteAllText($payload, $Marker, [Text.Encoding]::ASCII)
    Invoke-DotNetBuild -ProjectPath $projectPath -BinDir $binDir -Version $Version `
        -OutputDirectory $output -ProductCode $ProductCode `
        -TestWixSource $fixtureSource -FixturePayloadPath $payload
}

function New-TestUser {
    $securePassword = ConvertTo-SecureString $testPassword -AsPlainText -Force
    New-LocalUser -Name $testUser -Password $securePassword | Out-Null
    $script:createdUser = $true
    $script:testUserSid = (Get-LocalUser -Name $testUser).SID.Value
}

function Initialize-CustomerState {
    foreach ($path in $protectedRoots) {
        Initialize-Directory $path | Out-Null
    }
    foreach ($log in @(Get-AgentLogFiles)) { Remove-Item -LiteralPath $log.FullName -Force }
    foreach ($file in $customerOwnedFiles) {
        [IO.File]::WriteAllText($file.Path, $file.Contents)
    }
}

function Invoke-InstallStage {
    param([Parameter(Mandatory = $true)]$Packages)
    Add-PermissiveAces -OwnerSid $testUserSid
    # The service must create the first log file through its own ACE.
    Assert-Equal 0 (@(Get-AgentLogFiles)).Count "no agent log file exists before install"
    Install-Msi $Packages.V1 "fixture-v1"
    Assert-True (Test-Path -LiteralPath $agentPath -PathType Leaf) "v1 executable installed"
    Assert-InstalledVersion $fixtureProducts[0] "fixture-v1" "v1"
    Assert-ProtectedState "initial install"
    Invoke-NonAdminProbe -Stage "install"
    Assert-ServiceInstalled "install"
    Assert-ServiceRuntimeIdentity "install" 0
    foreach ($log in @(Get-AgentLogFiles)) { Assert-InheritedProtection $log.FullName }
    Write-Host "PASS initial install, ACL correction, denial, and service installed"
}

# Loosen every protected directory so the next installer operation must repair it.
function Add-PermissiveAces {
    param([string]$OwnerSid = "")
    foreach ($path in $protectedRoots) {
        Set-PermissiveAcl $path $OwnerSid
        Assert-PermissiveAcl $path $OwnerSid
    }
}

function Set-PermissiveAcl {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [string]$OwnerSid = ""
    )
    Initialize-Directory $Path | Out-Null
    $permissive = New-Object Security.AccessControl.DirectorySecurity
    $permissive.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;WD)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")
    Set-Acl -LiteralPath $Path -AclObject $permissive
    if ($OwnerSid) {
        & icacls.exe $Path /setowner "*$OwnerSid" | Out-Null
        Assert-Equal 0 $LASTEXITCODE "hostile owner assigned to $Path"
    }
}

function Assert-PermissiveAcl {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [string]$OwnerSid = ""
    )
    $acl = Get-Acl -LiteralPath $Path
    Assert-True $acl.AreAccessRulesProtected "$Path hostile DACL is protected"
    $everyone = @($acl.Access | Where-Object { $_.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value -eq "S-1-1-0" })
    Assert-Equal 1 $everyone.Count "$Path has one permissive Everyone ACE"
    Assert-True (-not $everyone[0].IsInherited) "$Path permissive ACE is explicit"
    Assert-FullControlAce $everyone[0] "$Path permissive" -Inheritable | Out-Null
    if ($OwnerSid) { Assert-Equal $OwnerSid $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value "$Path hostile owner verified" }
}

# Returns the ACE's SID. Directory ACEs must also propagate to children;
# file ACEs carry no inheritance flags, so -Inheritable is only for directories.
function Assert-FullControlAce {
    param(
        [Parameter(Mandatory = $true)]$Rule,
        [Parameter(Mandatory = $true)][string]$Label,
        [switch]$Inheritable
    )
    Assert-Equal "Allow" $Rule.AccessControlType.ToString() "$Label ACE type"
    Assert-Equal ([int][Security.AccessControl.FileSystemRights]::FullControl) ([int]$Rule.FileSystemRights) "$Label ACE grants exactly full control"
    if ($Inheritable) {
        $inherit = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit
        Assert-Equal ([int]$inherit) ([int]$Rule.InheritanceFlags) "$Label ACE inherits to containers and files"
        Assert-Equal ([int][Security.AccessControl.PropagationFlags]::None) ([int]$Rule.PropagationFlags) "$Label ACE has no propagation restriction"
    }
    return $Rule.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
}

function Install-Msi {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Name,
        [int[]]$AllowedExitCodes = @(0, 3010),
        [string[]]$Properties = @()
    )
    Invoke-Msi (@("/i", ('"{0}"' -f $Path)) + $Properties) $Name $AllowedExitCodes | Out-Null
}

function Assert-InstalledVersion {
    param(
        [Parameter(Mandatory = $true)][string]$ProductCode,
        [Parameter(Mandatory = $true)][string]$Marker,
        [Parameter(Mandatory = $true)][string]$Stage
    )
    Assert-OneRegistration $ProductCode
    Assert-True (Test-ArpProductCode $ProductCode) "$Stage installer metadata registered"
    Assert-Equal $Marker (Get-Marker) "$Stage marker"
}

function Assert-OneRegistration {
    param([Parameter(Mandatory = $true)][string]$ExpectedProduct)
    $related = @(Get-RelatedProducts)
    Assert-Equal 1 $related.Count "exactly one related product registration"
    Assert-Equal $ExpectedProduct.ToUpperInvariant() $related[0].ToUpperInvariant() "registered ProductCode"
}

function Test-ArpProductCode {
    param([Parameter(Mandatory = $true)][string]$ProductCode)
    $paths = @(
        "Registry::HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode",
        "Registry::HKEY_LOCAL_MACHINE\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\$ProductCode"
    )
    return (@($paths | Where-Object { Test-Path -LiteralPath $_ })).Count -ne 0
}

function Get-Marker {
    return [IO.File]::ReadAllText($markerPath)
}

function Assert-ProtectedState {
    param([Parameter(Mandatory = $true)][string]$Stage)
    Assert-CustomerStateRetained $Stage
    Assert-ProtectedAcls
}

function Assert-CustomerStateRetained {
    param([Parameter(Mandatory = $true)][string]$Stage)
    Assert-ProtectedRootsRetained $Stage
    Assert-OwnedFilesRetained $customerOwnedFiles $Stage
    Assert-OwnedFilesRetained @($representativeFiles) $Stage
}

function Assert-ProtectedRootsRetained {
    param([Parameter(Mandatory = $true)][string]$Stage)
    foreach ($path in $protectedRoots) {
        Assert-True (Test-Path -LiteralPath $path -PathType Container) `
            "$Stage keeps $path"
    }
}

function Assert-OwnedFilesRetained {
    param(
        # Empty until the first non-admin probe registers representative files.
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Files,
        [Parameter(Mandatory = $true)][string]$Stage
    )
    foreach ($file in $Files) {
        $contents = [IO.File]::ReadAllText($file.Path)
        Assert-Equal $file.Contents $contents "$Stage keeps $($file.Path)"
    }
}

function Assert-ProtectedAcls {
    foreach ($path in $protectedRoots) { Assert-ProtectedAcl $path }
}

function Assert-ProtectedAcl {
    param([Parameter(Mandatory = $true)][string]$LiteralPath)
    $acl = Get-Acl -LiteralPath $LiteralPath
    Assert-Equal "S-1-5-18" $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value "$LiteralPath owner is SYSTEM"
    Assert-True $acl.AreAccessRulesProtected "$LiteralPath DACL inheritance is disabled"
    $rules = @($acl.Access)
    Assert-Equal 4 $rules.Count "exactly 4 total $LiteralPath ACEs"
    $explicit = @($rules | Where-Object { -not $_.IsInherited })
    Assert-Equal 4 $explicit.Count "exactly 4 explicit $LiteralPath ACEs"
    Assert-ServiceDirectoryAces @($explicit | Where-Object { (Get-RuleSid $_) -eq $MsiServiceSid }) $LiteralPath
    $adminSids = @($explicit | Where-Object { (Get-RuleSid $_) -ne $MsiServiceSid } |
        ForEach-Object { Assert-FullControlAce $_ $LiteralPath -Inheritable })
    $expectedAdmins = @($MsiTrustedSids | Where-Object { $_ -ne $MsiServiceSid })
    Assert-Equal (($expectedAdmins | Sort-Object) -join ",") (($adminSids | Sort-Object) -join ",") "$LiteralPath administrator ACE identities"
    & icacls.exe $LiteralPath 2>&1 | Out-Null
    Assert-Equal 0 $LASTEXITCODE "icacls can inspect $LiteralPath"
}

# The service SID may list, traverse, and add children of a protected
# directory but not delete, re-permission, or take ownership of it; an
# inherit-only ACE gives it full control of everything created inside.
function Assert-ServiceDirectoryAces {
    param(
        [Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Rules,
        [Parameter(Mandatory = $true)][string]$Label
    )
    Assert-Equal 2 $Rules.Count "$Label has two service SID ACEs"
    $inheritOnly = [int][Security.AccessControl.PropagationFlags]::InheritOnly
    $direct = @($Rules | Where-Object { ([int]$_.PropagationFlags -band $inheritOnly) -eq 0 })
    $children = @($Rules | Where-Object { ([int]$_.PropagationFlags -band $inheritOnly) -ne 0 })
    Assert-Equal 1 $direct.Count "$Label has one service SID ACE for the directory itself"
    Assert-Equal "Allow" $direct[0].AccessControlType.ToString() "$Label service directory ACE type"
    Assert-Equal $MsiServiceDirectoryRights ([int]$direct[0].FileSystemRights) "$Label service directory ACE grants read, write, and traverse only"
    Assert-Equal 0 ([int]$direct[0].InheritanceFlags) "$Label service directory ACE is not inherited"
    Assert-Equal 1 $children.Count "$Label has one inherit-only service SID ACE"
    Assert-Equal "Allow" $children[0].AccessControlType.ToString() "$Label service child ACE type"
    Assert-Equal ([int][Security.AccessControl.FileSystemRights]::FullControl) ([int]$children[0].FileSystemRights) "$Label service child ACE grants full control"
    $inherit = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit
    Assert-Equal ([int]$inherit) ([int]$children[0].InheritanceFlags) "$Label service child ACE inherits to containers and files"
    Assert-Equal $inheritOnly ([int]$children[0].PropagationFlags) "$Label service child ACE is inherit-only"
}

function Get-RuleSid {
    param([Parameter(Mandatory = $true)]$Rule)
    return $Rule.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
}

function Assert-TrustedIdentities {
    param(
        [Parameter(Mandatory = $true)][string[]]$Sids,
        [Parameter(Mandatory = $true)][string]$Label
    )
    Assert-Equal (($MsiTrustedSids | Sort-Object) -join ",") (($Sids | Sort-Object) -join ",") "$Label ACE identities"
}

function Invoke-NonAdminProbe {
    param([Parameter(Mandatory = $true)][string]$Stage)
    $files = @(New-RepresentativeFiles -Stage $Stage)
    $workspace = New-ProbeWorkspace $files
    $result = Invoke-ProbeAsTestUser $workspace
    Assert-ProbeIdentity $result
    Assert-ProbeDenied $result $files $Stage
    Assert-ProtectedState "$Stage after non-admin probes"
}

# Fresh files in each protected directory prove inheritance without relying on
# files secured by an earlier operation.
function New-RepresentativeFiles {
    param([Parameter(Mandatory = $true)][string]$Stage)
    return @($protectedRoots | ForEach-Object { New-RepresentativeFile $_ $Stage })
}

function New-RepresentativeFile {
    param(
        [Parameter(Mandatory = $true)][string]$Parent,
        [Parameter(Mandatory = $true)][string]$Stage
    )
    $file = [pscustomobject]@{
        Parent = $Parent
        Path = Join-Path $Parent ("representative-$Stage-" + [Guid]::NewGuid().ToString("N") + ".txt")
        Contents = "representative-$Stage-" + [Guid]::NewGuid().ToString("N")
        CreatePath = Join-Path $Parent ("non-admin-" + [Guid]::NewGuid().ToString("N") + ".txt")
    }
    [IO.File]::WriteAllText($file.Path, $file.Contents)
    Assert-True (Test-Path -LiteralPath $file.Path -PathType Leaf) "representative read target exists"
    Assert-True (-not (Test-Path -LiteralPath $file.CreatePath)) "representative create target is absent"
    Assert-InheritedProtection $file.Path
    [void]$representativeFiles.Add($file)
    return $file
}

function Assert-InheritedProtection {
    param([Parameter(Mandatory = $true)][string]$Path)
    $acl = Get-Acl -LiteralPath $Path
    Assert-True (-not $acl.AreAccessRulesProtected) "$Path inherits its DACL"
    Assert-Equal $MsiTrustedSids.Count @($acl.Access).Count "$Path has only trusted inherited ACEs"
    $sids = @($acl.Access | ForEach-Object {
        Assert-True $_.IsInherited "$Path ACE is inherited"
        Assert-FullControlAce $_ $Path
    })
    Assert-TrustedIdentities $sids $Path
}

function New-ProbeWorkspace {
    param([Parameter(Mandatory = $true)][object[]]$Files)
    $root = Initialize-Directory (Join-Path $artifactsRoot `
        ("probe-" + [Guid]::NewGuid().ToString("N")))
    & icacls.exe $root /grant ("$testUser`:(OI)(CI)F") | Out-Null
    Assert-Equal 0 $LASTEXITCODE "non-admin probe directory permissions"
    $workspace = [pscustomobject]@{
        Script = Join-Path $root "probe.ps1"
        Manifest = Join-Path $root "manifest.json"
        Result = Join-Path $root "result.json"
    }
    $manifest = @{ Files = $Files; ProbeRoot = $root; ControlPath = (Join-Path $root "control.txt") }
    [IO.File]::WriteAllText($workspace.Manifest, ($manifest | ConvertTo-Json -Depth 4))
    Copy-Item -LiteralPath (Join-Path $script:WindowsTestRoot "non-admin-probe.ps1") -Destination $workspace.Script -Force
    return $workspace
}

function Invoke-ProbeAsTestUser {
    param([Parameter(Mandatory = $true)]$Workspace)
    $securePassword = ConvertTo-SecureString $testPassword -AsPlainText -Force
    $credential = New-Object Management.Automation.PSCredential("$env:COMPUTERNAME\$testUser", $securePassword)
    $arguments = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
        ('"{0}"' -f $Workspace.Script), ('"{0}"' -f $Workspace.Manifest), ('"{0}"' -f $Workspace.Result))
    $process = Start-Process -FilePath "powershell.exe" -Credential $credential -ArgumentList $arguments -Wait -PassThru
    Assert-Equal 0 $process.ExitCode "non-admin probe process"
    return (Get-Content -LiteralPath $Workspace.Result -Raw | ConvertFrom-Json)
}

function Assert-ProbeIdentity {
    param([Parameter(Mandatory = $true)]$Result)
    Assert-Equal $testUserSid $Result.Sid "probe runs as the temporary account"
    Assert-Equal $false $Result.Administrator "probe account is not an administrator"
    foreach ($operation in @("Read", "Create", "Regrant")) {
        Assert-Equal "Allowed" $Result.Control.$operation "probe control $operation succeeds"
    }
}

function Assert-ProbeDenied {
    param(
        [Parameter(Mandatory = $true)]$Result,
        [Parameter(Mandatory = $true)][object[]]$Files,
        [Parameter(Mandatory = $true)][string]$Stage
    )
    Assert-Equal $Files.Count @($Result.Results).Count "one result per protected directory"
    foreach ($file in $Files) {
        $entry = @($Result.Results | Where-Object { $_.Path -eq $file.Path })
        Assert-Equal 1 $entry.Count "one result for $($file.Path)"
        foreach ($operation in @("Read", "Create", "Regrant")) {
            Assert-Equal "AccessDenied" $entry[0].$operation "$Stage $operation denied for $($file.Parent)"
        }
        Assert-True (-not (Test-Path -LiteralPath $file.CreatePath)) "non-admin child was not created"
    }
}

# Install success with Start="install" Wait="yes" proves only that the SCM saw
# Running, which the agent reports before its body runs, so this asserts SCM
# configuration; Assert-ServiceRuntimeIdentity checks the running agent.
function Assert-ServiceInstalled {
    param([Parameter(Mandatory = $true)][string]$Stage)
    $service = Get-AgentService
    Assert-True ($null -ne $service) "$Stage installs the miru-agent service"
    Assert-Equal "Auto" $service.StartMode "$Stage service start mode is automatic"
    Assert-Equal $MsiServiceAccount $service.StartName "$Stage service runs as NT SERVICE\miru-agent"
    Assert-Equal $agentPath ($service.PathName.Trim('"')) "$Stage service binary path"
    Assert-ServiceRecovery $Stage
    Assert-ServiceSidIdentity $Stage
    Assert-ServicePrivileges $Stage
}

function Assert-ServiceAbsent {
    $service = Get-AgentService
    Assert-True ($null -eq $service) "miru-agent service removed"
}

function Get-AgentService {
    return Get-CimInstance Win32_Service -Filter "Name='miru-agent'" `
        -ErrorAction SilentlyContinue
}

function Assert-ServiceRecovery {
    param([Parameter(Mandatory = $true)][string]$Stage)
    $out = & sc.exe qfailure miru-agent 2>&1 | Out-String
    Assert-Equal 0 $LASTEXITCODE "$Stage sc.exe qfailure succeeds"
    Assert-True ($out -match 'RESET_PERIOD') "$Stage service has a reset period"
    Assert-True ($out -match 'RESTART') "$Stage service restarts on failure"
}

# The SCM adds the service SID to the token, Windows derives the same SID from
# the service name, and the virtual account name resolves to it, so the SID
# hardcoded in the installer's descriptors is the one the service runs as.
function Assert-ServiceSidIdentity {
    param([Parameter(Mandatory = $true)][string]$Stage)
    $q = & sc.exe qsidtype miru-agent 2>&1 | Out-String
    Assert-Equal 0 $LASTEXITCODE "$Stage sc.exe qsidtype succeeds"
    Assert-True ($q -match 'SERVICE_SID_TYPE:\s+UNRESTRICTED') "$Stage service SID type is unrestricted`n$q"
    $s = & sc.exe showsid miru-agent 2>&1 | Out-String
    Assert-Equal 0 $LASTEXITCODE "$Stage sc.exe showsid succeeds"
    Assert-True ($s -match 'SERVICE SID:\s+(S-1-5-80-[\d-]+)') "$Stage sc.exe showsid reports a service SID`n$s"
    Assert-Equal $MsiServiceSid $Matches[1] "$Stage Windows derives the hardcoded service SID"
    $accountSid = ([Security.Principal.NTAccount]$MsiServiceAccount).Translate([Security.Principal.SecurityIdentifier]).Value
    Assert-Equal $MsiServiceSid $accountSid "$Stage virtual account resolves to the hardcoded SID"
}

# The SCM gives the service token only these privileges, so the service
# cannot use SeImpersonatePrivilege to reach Local System.
function Assert-ServicePrivileges {
    param([Parameter(Mandatory = $true)][string]$Stage)
    $out = & sc.exe qprivs miru-agent 2>&1 | Out-String
    Assert-Equal 0 $LASTEXITCODE "$Stage sc.exe qprivs succeeds"
    $privileges = @([regex]::Matches($out, 'Se\w+Privilege') | ForEach-Object { $_.Value } | Sort-Object)
    Assert-Equal (($MsiServiceRequiredPrivileges | Sort-Object) -join ",") ($privileges -join ",") "$Stage service required privileges`n$out"
}

function Get-AgentLogFiles {
    if (-not (Test-Path -LiteralPath $logsRoot -PathType Container)) { return @() }
    return @(Get-ChildItem -LiteralPath $logsRoot -Filter "miru.log*" -File)
}

# The running service holds the current file open for writing, so each file is
# opened with read-write sharing.
function Get-ActivationWaitCount {
    $count = 0
    foreach ($log in @(Get-AgentLogFiles)) {
        $fs = $null
        $reader = $null
        try {
            $fs = [IO.File]::Open($log.FullName, 'Open', 'Read', 'ReadWrite')
            $reader = New-Object IO.StreamReader($fs)
            $text = $reader.ReadToEnd()
        }
        finally {
            if ($null -ne $reader) { $reader.Dispose() }
            if ($null -ne $fs) { $fs.Dispose() }
        }
        $count += [regex]::Matches($text, 'waiting for provisioning').Count
    }
    return [int]$count
}

# The unprovisioned agent logs one "waiting for provisioning" line per start and
# keeps running. A new line proves the service could write logs\ through the
# installer's ACE; the process token proves it runs as the virtual account.
function Assert-ServiceRuntimeIdentity {
    param(
        [Parameter(Mandatory = $true)][string]$Stage,
        [Parameter(Mandatory = $true)][int]$Baseline
    )
    $service = $null
    $count = 0
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        $service = Get-AgentService
        $count = Get-ActivationWaitCount
        if ($null -ne $service -and $service.State -eq 'Running' -and
            [int]$service.ProcessId -ne 0 -and $count -gt $Baseline) { break }
        Start-Sleep -Seconds 1
    }
    Assert-True ($null -ne $service) "$Stage miru-agent service present"
    $processId = [int]$service.ProcessId
    $observed = "state $($service.State), pid $processId, activation waits $count, baseline $Baseline"
    Assert-Equal "Running" $service.State "$Stage service is running ($observed)"
    Assert-True ($processId -ne 0) "$Stage service has a process ($observed)"
    Assert-True ($count -gt $Baseline) "$Stage service logged a new activation wait ($observed)"
    $proc = Get-CimInstance Win32_Process -Filter "ProcessId=$processId"
    Assert-True ($null -ne $proc) "$Stage service process $processId exists"
    $ownerSid = $proc | Invoke-CimMethod -MethodName GetOwnerSid
    Assert-Equal 0 ([int]$ownerSid.ReturnValue) "$Stage GetOwnerSid succeeds for process $processId"
    Assert-Equal $MsiServiceSid $ownerSid.Sid "$Stage service process token user is the service SID"
    $owner = $proc | Invoke-CimMethod -MethodName GetOwner
    Assert-Equal 0 ([int]$owner.ReturnValue) "$Stage GetOwner succeeds for process $processId"
    Assert-True ($owner.Domain -ieq 'NT SERVICE') "$Stage service process domain is NT SERVICE (actual '$($owner.Domain)')"
    Assert-True ($owner.User -ieq $MsiServiceName) "$Stage service process user is $MsiServiceName (actual '$($owner.User)')"
}

function Invoke-MaintenanceStage {
    param([Parameter(Mandatory = $true)]$Packages)
    $v1Hash = Get-AgentHash
    Add-PermissiveAces -OwnerSid $testUserSid
    Install-Msi $Packages.V1 "fixture-v1-maintenance" -Properties @("REINSTALL=ALL", "REINSTALLMODE=vomus")
    Assert-InstalledVersion $fixtureProducts[0] "fixture-v1" "maintenance"
    Assert-Equal $v1Hash (Get-AgentHash) "maintenance keeps v1 hash"
    Assert-ProtectedState "maintenance"
    Invoke-NonAdminProbe -Stage "maintenance"
    Assert-ServiceInstalled "maintenance"
    Write-Host "PASS same-MSI maintenance repairs ACL and retains v1 state"
}

function Get-AgentHash {
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $agentPath).Hash
}

function Invoke-UpgradeStage {
    param([Parameter(Mandatory = $true)]$Packages)
    Add-PermissiveAces -OwnerSid $testUserSid
    $baseline = Get-ActivationWaitCount
    Install-Msi $Packages.V2 "fixture-v2-upgrade"
    Assert-InstalledVersion $fixtureProducts[1] "fixture-v2" "v2"
    Assert-ProtectedState "upgrade"
    Invoke-NonAdminProbe -Stage "upgrade"
    Assert-ServiceInstalled "upgrade"
    Assert-ServiceRuntimeIdentity "upgrade" $baseline
    Write-Host "PASS v1-to-v2 upgrade repairs ACL and registers one product"
}

function Invoke-DowngradeStage {
    param([Parameter(Mandatory = $true)]$Packages)
    $v2Hash = Get-AgentHash
    Install-Msi $Packages.V1 "fixture-downgrade" -AllowedExitCodes @(1603)
    Assert-InstalledVersion $fixtureProducts[1] "fixture-v2" "downgrade leaves v2"
    Assert-Equal $v2Hash (Get-AgentHash) "downgrade leaves v2 executable"
    Assert-ProtectedState "downgrade rejection"
    Assert-ServiceInstalled "downgrade rejection"
    Write-Host "PASS downgrade rejected with v2 intact"
}

function Invoke-RollbackStage {
    param([Parameter(Mandatory = $true)]$Packages)
    $v2Hash = Get-AgentHash
    $baseline = Get-ActivationWaitCount
    Install-Msi $Packages.V3 "fixture-v3-rollback" -AllowedExitCodes @(1603) -Properties @("FAIL_UPGRADE_FOR_TEST=1")
    Assert-InstalledVersion $fixtureProducts[1] "fixture-v2" "rollback restores v2"
    Assert-Equal $v2Hash (Get-AgentHash) "rollback restores v2 executable"
    Assert-ProtectedState "rollback"
    Assert-ServiceInstalled "rollback"
    Assert-ServiceRuntimeIdentity "rollback" $baseline
    Write-Host "PASS failed v3 upgrade rolls back registration, hash, marker, sentinel, DACL, and full service config"
}

function Invoke-UninstallStage {
    Uninstall-Msi $fixtureProducts[1] "fixture-v2-uninstall"
    Assert-Equal 0 (@(Get-RelatedProducts)).Count "product registration removed"
    Assert-True (-not (Test-ArpProductCode $fixtureProducts[1])) "installer-owned registry metadata removed"
    Assert-True (-not (Test-Path -LiteralPath $agentPath)) "executable removed"
    Assert-True (-not (Test-Path -LiteralPath $markerPath)) "test marker removed"
    Assert-True (Test-Path -LiteralPath $programDataRoot -PathType Container) "ProgramData retained"
    Assert-ProtectedState "uninstall"
    Assert-ServiceAbsent
    Write-Host "PASS uninstall removes package state, service, and retains protected customer state"
}

function Write-FailureEvidence {
    param([Parameter(Mandatory = $true)][Management.Automation.ErrorRecord]$Failure)
    Write-Host "Integration failure: $($Failure.Exception.Message)" -ForegroundColor Red
    try {
        Write-Host "Related products: $(@(Get-RelatedProducts) -join ', ')"
        if (Test-Path -LiteralPath $programDataRoot) { & icacls.exe $programDataRoot }
        if (Test-Path -LiteralPath $agentPath) { Get-FileHash -Algorithm SHA256 -LiteralPath $agentPath }
        if (Test-Path -LiteralPath $markerPath) { Write-Host "Marker: $(Get-Marker)" }
    }
    catch {
        Write-Host "Failure evidence collection also failed: $($_.Exception.Message)" -ForegroundColor Red
    }
}

function Remove-FixtureProducts {
    try {
        foreach ($product in @(Get-RelatedProducts | Where-Object { Test-FixtureProduct $_ })) {
            Uninstall-Msi $product "cleanup-$($product.Trim('{}'))" @(0, 3010, 1605)
        }
        foreach ($product in @(Get-RelatedProducts | Where-Object { Test-FixtureProduct $_ })) {
            Add-CleanupFailure "fixture ProductCode $($product.ToUpperInvariant()) remains installed after cleanup"
        }
    }
    catch {
        Add-CleanupFailure "product cleanup failed: $($_.Exception.Message)"
    }
}

function Add-CleanupFailure {
    param([Parameter(Mandatory = $true)][string]$Message)
    [void]$cleanupFailures.Add($Message)
}

function Remove-TestUser {
    if (-not $createdUser) { return }
    try { Remove-LocalUser -Name $testUser -ErrorAction Stop }
    catch { Add-CleanupFailure "temporary user deletion failed: $($_.Exception.Message)" }
    try {
        if ($null -ne (Get-LocalUser -Name $testUser -ErrorAction SilentlyContinue)) {
            Add-CleanupFailure "temporary user $testUser still exists after deletion"
        }
    }
    catch { Add-CleanupFailure "temporary user deletion verification failed: $($_.Exception.Message)" }
}

function Remove-TestFiles {
    try {
        if (Test-Path -LiteralPath $markerPath) { Remove-Item -LiteralPath $markerPath -Force -ErrorAction Stop }
    }
    catch { Add-CleanupFailure "fixture marker deletion failed: $($_.Exception.Message)" }
    try {
        if (Test-Path -LiteralPath $artifactsRoot) { Remove-Item -LiteralPath $artifactsRoot -Recurse -Force -ErrorAction Stop }
    }
    catch { Add-CleanupFailure "temporary artifact deletion failed: $($_.Exception.Message)" }
}

function Complete-IntegrationRun {
    param([Management.Automation.ErrorRecord]$PrimaryFailure)
    foreach ($cleanupFailure in $cleanupFailures) {
        Write-Host "Cleanup failure: $cleanupFailure" -ForegroundColor Red
    }
    if ($null -ne $PrimaryFailure) { throw $PrimaryFailure }
    if ($cleanupFailures.Count -ne 0) { throw "Integration cleanup failed; see cleanup diagnostics above." }
}
