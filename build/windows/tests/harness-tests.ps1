#Requires -Version 5.1
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
Import-Module -Force -Name (Join-Path $PSScriptRoot "MsiTest.psm1")
. (Join-Path $PSScriptRoot "integration-lib.ps1")

$fixtureProducts = @($MsiFixtureProductCodes)
$foreignProduct = "{11111111-1111-1111-1111-111111111111}"
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("miru-harness-tests-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $testRoot | Out-Null
$caseCount = 0

function Assert-Test {
    param([bool]$Condition, [string]$Label)
    if (-not $Condition) { throw "HARNESS ASSERT: $Label" }
}

function Register-HarnessMocks {
    function script:Write-Host {
        param([object]$Object, [ConsoleColor]$ForegroundColor)
        [void]$script:hostLines.Add([string]$Object)
    }

    function script:Start-Process {
        [CmdletBinding()]
        param([string]$FilePath, [string[]]$ArgumentList, [switch]$Wait, [switch]$PassThru)
        Assert-Test ($FilePath -eq "msiexec.exe" -and $Wait -and $PassThru) "only mocked synchronous MSI processes"
        $logIndex = [Array]::IndexOf($ArgumentList, "/l*v")
        Assert-Test ($logIndex -ge 0) "MSI verbose logging requested"
        $path = $ArgumentList[$logIndex + 1].Trim('"')
        $name = [IO.Path]::GetFileNameWithoutExtension($path)
        Assert-Test ($script:hostLines -contains "MSI log ($name): $path") "log location reported before MSI starts"
        if ($name -eq "manual-uninstall") { Assert-RepresentativeContents }
        $contents = "$name/$($script:processCalls.Count)/$([Guid]::NewGuid())"
        [IO.File]::WriteAllText($path, $contents)
        $code = 0
        if ($script:exitCodes.ContainsKey($name)) { $code = $script:exitCodes[$name] }
        [void]$script:processCalls.Add([pscustomobject]@{ Name = $name; Path = $path; Contents = $contents; Arguments = $ArgumentList })
        if ($ArgumentList[0] -eq "/x") {
            Assert-Test ($fixtureProducts -contains $ArgumentList[1]) "cleanup never uninstalls a foreign product"
            if (@(0, 3010, 1605) -contains $code) { [void]$script:relatedProducts.Remove($ArgumentList[1]) }
        }
        return [pscustomobject]@{ ExitCode = $code }
    }

    function script:Get-RelatedProducts {
        $script:relatedReads++
        if ($script:diagnosticsFail -and $script:relatedReads -eq 1) { throw [InvalidOperationException]::new("injected diagnostics failure") }
        return @($script:relatedProducts)
    }

    function script:Remove-LocalUser {
        [CmdletBinding()]
        param([string]$Name)
        [void]$script:cleanupAttempts.Add("user")
        if ($script:cleanupFault -eq "user") { throw [InvalidOperationException]::new("injected user removal failure") }
        $script:userExists = $false
    }

    function script:Get-LocalUser {
        [CmdletBinding()]
        param([string]$Name)
        [void]$script:cleanupAttempts.Add("verify-user")
        if ($script:cleanupFault -eq "verify-user") { throw [InvalidOperationException]::new("injected user verification failure") }
        if ($script:userExists) { return [pscustomobject]@{ Name = $Name } }
    }

    function script:Remove-Item {
        [CmdletBinding()]
        param([string]$LiteralPath, [switch]$Recurse, [switch]$Force)
        $label = if ($LiteralPath -eq $script:artifactsRoot) { "artifacts" } else { "marker" }
        [void]$script:cleanupAttempts.Add($label)
        if ($script:cleanupFault -eq $label) { throw [IO.IOException]::new("injected removal failure") }
        Microsoft.PowerShell.Management\Remove-Item -LiteralPath $LiteralPath -Recurse:$Recurse -Force:$Force
    }

    function script:Stop-Transcript {
        [void]$script:cleanupAttempts.Add("transcript")
        if ($script:cleanupFault -eq "transcript") { throw [InvalidOperationException]::new("injected transcript failure") }
    }
}

function Invoke-TestOperation {
    try { Invoke-Msi @("/i", "fixture.msi") "primary" @(0, 3010) | Out-Null }
    catch { $script:primaryException = $_.Exception; throw }
}

function Assert-LogsSurvive {
    foreach ($call in $script:processCalls) {
        Assert-Test (-not $call.Path.StartsWith($script:artifactsRoot + [IO.Path]::DirectorySeparatorChar)) "log outside temporary build output"
        Assert-Test ($call.Path.StartsWith($script:deterministicLogs + [IO.Path]::DirectorySeparatorChar)) "log beneath durable artifact directory"
        Assert-Test ([IO.File]::ReadAllText($call.Path) -ceq $call.Contents) "distinct MSI log bytes survive cleanup"
    }
}

function Assert-RepresentativeContents {
    Assert-Test ($script:representativeFiles.Count -eq 4) "manual smoke populates all four protected directories"
    foreach ($parent in $script:protectedRoots) {
        $files = @($script:representativeFiles | Where-Object { $_.Parent -eq $parent })
        Assert-Test ($files.Count -eq 1) "one retained representative per protected directory"
        Assert-Test (Test-Path -LiteralPath $files[0].Path -PathType Leaf) "representative exists before and after uninstall"
        Assert-Test ([IO.File]::ReadAllText($files[0].Path) -ceq $files[0].Contents) "representative bytes retained"
    }
}

function Assert-Failure {
    param($Actual, [bool]$Expected, [bool]$Primary)
    Assert-Test (($null -ne $Actual) -eq $Expected) "expected final failure status"
    if ($Primary) {
        Assert-Test ([object]::ReferenceEquals($script:primaryException, $Actual.Exception)) "original exception preserved"
    }
    elseif ($Expected) {
        Assert-Test ($Actual.Exception -is [Management.Automation.RuntimeException]) "cleanup-only runtime failure"
    }
}

function Invoke-Case {
    param([string]$Name, [scriptblock]$Body)
    $script:caseCount++
    $script:caseRoot = Join-Path $testRoot ([string]$script:caseCount)
    $script:repositoryRoot = Join-Path $script:caseRoot "repository"
    $script:artifactsRoot = Join-Path $script:caseRoot "temporary-build"
    $script:deterministicLogs = Join-Path $script:repositoryRoot "build\windows\artifacts\package-tests\logs"
    $script:sessionLogs = New-MsiSessionLogDirectory $script:deterministicLogs
    $script:programDataRoot = Join-Path $script:caseRoot "data"
    $script:logsRoot = Join-Path $script:programDataRoot "logs"
    $script:protectedRoots = @($script:programDataRoot, $script:logsRoot, (Join-Path $script:programDataRoot "auth"), (Join-Path $script:programDataRoot "tmp"))
    $script:representativeFiles = New-Object Collections.ArrayList
    $script:sentinelPath = Join-Path $script:programDataRoot "sentinel.txt"
    $script:customerLogPath = Join-Path $script:logsRoot "customer.log"
    $script:customerLogContents = "retained"
    $script:agentPath = Join-Path $script:caseRoot "absent-agent.exe"
    $script:markerPath = Join-Path $script:caseRoot "marker.txt"
    $script:testUser = "HarnessMockUser"
    $script:createdUser = $true
    $script:userExists = $true
    $script:transcriptStarted = $true
    $script:integrationFailure = $null
    $script:primaryException = $null
    $script:cleanupFault = ""
    $script:diagnosticsFail = $false
    $script:relatedReads = 0
    $script:exitCodes = @{}
    $script:hostLines = New-Object Collections.ArrayList
    $script:processCalls = New-Object Collections.ArrayList
    $script:cleanupAttempts = New-Object Collections.ArrayList
    $script:cleanupFailures = New-Object Collections.ArrayList
    $script:failureEvidence = New-Object Collections.ArrayList
    $script:relatedProducts = New-Object Collections.ArrayList
    [void]$script:relatedProducts.Add($fixtureProducts[0])
    [void]$script:relatedProducts.Add($foreignProduct)
    New-Item -ItemType Directory -Path $script:artifactsRoot -Force | Out-Null
    [IO.File]::WriteAllText($script:markerPath, "fixture-marker")
    Register-HarnessMocks
    # Per-case mocks are defined unscoped inside the body: dynamic scoping lets lib functions called from the body
    # resolve them, and they die with the body scope instead of leaking into later cases.
    & $Body
    Microsoft.PowerShell.Utility\Write-Host "PASS harness $Name"
}

try {
    foreach ($code in @(0, 3010, 1603)) {
        Invoke-Case "MSI result $code" {
            $script:exitCodes["result"] = $code
            $failure = $null
            $result = $null
            try { $result = Invoke-Msi @("/i", "fixture.msi", "REINSTALL=ALL") "result" @(0, 3010) }
            catch { $failure = $_ }
            Assert-Test (($null -ne $failure) -eq ($code -eq 1603)) "allowed MSI codes"
            if ($code -ne 1603) { Assert-Test ($result -eq $code) "numeric MSI return value" }
            else { Assert-Test ($script:failureEvidence -contains $script:processCalls[0].Path) "failed MSI log recorded" }
            Assert-Test (($script:processCalls[0].Arguments[0..2] -join '|') -eq '/i|fixture.msi|REINSTALL=ALL') "original arguments forwarded"
            Remove-Item -LiteralPath $script:artifactsRoot -Recurse -Force
            Assert-LogsSurvive
        }
    }

    Invoke-Case "unique persistent sessions" {
        Invoke-Msi @("/i", "fixture.msi") "same-name" @(0) | Out-Null
        $script:sessionLogs = New-MsiSessionLogDirectory $script:deterministicLogs
        Invoke-Msi @("/i", "fixture.msi") "same-name" @(0) | Out-Null
        Assert-Test ($script:processCalls[0].Path -ne $script:processCalls[1].Path) "sessions do not overwrite identical operation names"
        Remove-Item -LiteralPath $script:artifactsRoot -Recurse -Force
        Assert-LogsSurvive
    }

    foreach ($primary in @($false, $true)) {
        foreach ($cleanup in @($false, $true)) {
            Invoke-Case "completion primary=$primary cleanup=$cleanup" {
                $record = $null
                if ($primary) {
                    $script:primaryException = [InvalidOperationException]::new("injected primary")
                    $record = [Management.Automation.ErrorRecord]::new($script:primaryException, "InjectedPrimary", [Management.Automation.ErrorCategory]::InvalidOperation, $null)
                }
                if ($cleanup) { [void]$script:cleanupFailures.Add("injected cleanup") }
                $failure = $null
                try { Complete-IntegrationRun $record } catch { $failure = $_ }
                Assert-Failure $failure ($primary -or $cleanup) $primary
                Assert-Test (@($script:hostLines | Where-Object { $_ -like 'Cleanup failure:*' }).Count -eq [int]$cleanup) "cleanup diagnostics reported"
            }
        }
    }

    foreach ($rebootStage in @("none", "install", "maintenance", "upgrade", "uninstall", "failed-maintenance")) {
        Invoke-Case "manual stage results $rebootStage" {
            function Assert-InstalledAllowlistSafe { param([switch]$RequireClean) }
            function Get-MiruArpProducts { }
            function Start-Transcript { param([string]$LiteralPath, [switch]$Force) }
            function Get-ComputerInfo { [pscustomobject]@{ WindowsProductName = "Harness"; WindowsVersion = "test"; OsBuildNumber = "0" } }
            function Get-MsiIdentity { param([string]$Path) [pscustomobject]@{ ProductName = "Harness" } }
            function Add-PermissiveAces {
                foreach ($path in $script:protectedRoots) { New-Item -ItemType Directory -Path $path -Force | Out-Null }
            }
            function Assert-ProtectedAcls {
                foreach ($path in $script:protectedRoots) { Assert-Test (Test-Path -LiteralPath $path -PathType Container) "protected directory still exists" }
            }
            function Get-Acl {
                param([string]$LiteralPath)
                Assert-Test (Test-Path -LiteralPath $LiteralPath -PathType Leaf) "ACL probe targets a real representative file"
                Assert-Test ($script:protectedRoots -contains [IO.Path]::GetDirectoryName($LiteralPath)) "ACL probe stays inside sandbox directories"
                $rules = @("S-1-5-18", "S-1-5-32-544") | ForEach-Object {
                    $identity = New-Object psobject -Property @{ Value = $_ }
                    Add-Member -InputObject $identity -MemberType ScriptMethod -Name Translate -Value { param($type) $this }
                    [pscustomobject]@{
                        IsInherited = $true
                        AccessControlType = [Security.AccessControl.AccessControlType]::Allow
                        FileSystemRights = [Security.AccessControl.FileSystemRights]::FullControl
                        IdentityReference = $identity
                    }
                }
                return [pscustomobject]@{ AreAccessRulesProtected = $false; Access = @($rules) }
            }
            function Assert-NoService { }
            function Get-RelatedProducts { return @($fixtureProducts[0]) }
            $ConfirmDisposableCleanVm = $true
            $TranscriptPath = Join-Path $script:caseRoot "transcript.txt"
            foreach ($version in @("1.0.0", "1.1.0")) {
                $directory = Join-Path $script:repositoryRoot ("build\windows\artifacts\package-tests\v" + $(if ($version -eq "1.0.0") { "1" } else { "2" }))
                New-Item -ItemType Directory -Path $directory -Force | Out-Null
                [IO.File]::WriteAllText((Join-Path $directory "miru-agent-$version.msi"), "fake MSI bytes")
            }
            if ($rebootStage -eq "failed-maintenance") { $script:exitCodes["manual-maintenance"] = 1603 }
            elseif ($rebootStage -ne "none") { $script:exitCodes["manual-$rebootStage"] = 3010 }
            $failure = $null
            try { Invoke-ManualSmoke | Out-Null } catch { $failure = $_ }
            $failed = $rebootStage -eq "failed-maintenance"
            Assert-Test (($null -ne $failure) -eq $failed) "manual stage failure status"
            Assert-Test ($script:processCalls.Count -eq $(if ($failed) { 2 } else { 4 })) "expected manual MSI stages ran"
            $stages = if ($failed) { @("install") } else { @("install", "maintenance", "upgrade", "uninstall") }
            $previous = -1
            foreach ($stage in $stages) {
                $expectedCode = if ($stage -eq $rebootStage) { 3010 } else { 0 }
                $position = $script:hostLines.IndexOf("Manual $stage reboot result: $expectedCode")
                Assert-Test ($position -gt $previous) "stage result appears in execution order"
                $pass = @($script:hostLines | Where-Object { $_ -match "^PASS manual (v1 )?$stage " })
                Assert-Test ($pass.Count -eq 1) "stage PASS output retained"
                $previous = $position
            }
            if ($failed) { Assert-Test (@($script:hostLines | Where-Object { $_ -match '^PASS manual maintenance ' }).Count -eq 0) "failed stage does not report PASS" }
            Assert-RepresentativeContents
            Assert-CustomerStateRetained "harness after manual smoke"
            Assert-LogsSurvive
        }
    }

    foreach ($scenario in @(
        @{ Primary = $false; Fault = "" },
        @{ Primary = $true; Fault = "" },
        @{ Primary = $false; Fault = "transcript" },
        @{ Primary = $false; Fault = "artifacts" },
        @{ Primary = $true; Fault = "transcript" }
    )) {
        Invoke-Case "manual cleanup primary=$($scenario.Primary) fault=$($scenario.Fault)" {
            function Invoke-ManualSmoke { Invoke-TestOperation }
            $script:cleanupFault = $scenario.Fault
            if ($scenario.Primary) { $script:exitCodes["primary"] = 1603 }
            $failure = $null
            try { Invoke-ManualRun } catch { $failure = $_ }
            Assert-Failure $failure ($scenario.Primary -or $scenario.Fault -ne "") $scenario.Primary
            Assert-Test ($script:processCalls.Count -eq 1) "manual operation generated a verbose log"
            Assert-Test ($script:cleanupAttempts -contains "transcript" -and $script:cleanupAttempts -contains "artifacts") "manual cleanup attempts continue"
            Assert-Test ((Test-Path -LiteralPath $script:artifactsRoot) -eq ($scenario.Fault -eq "artifacts")) "manual temporary cleanup outcome"
            Assert-LogsSurvive
        }
    }

    foreach ($scenario in @(
        @{ Primary = $false; CleanupMsi = $false; Diagnostics = $false; Fault = "" },
        @{ Primary = $true; CleanupMsi = $false; Diagnostics = $false; Fault = "" },
        @{ Primary = $false; CleanupMsi = $true; Diagnostics = $false; Fault = "" },
        @{ Primary = $true; CleanupMsi = $true; Diagnostics = $false; Fault = "" },
        @{ Primary = $true; CleanupMsi = $false; Diagnostics = $true; Fault = "" },
        @{ Primary = $true; CleanupMsi = $false; Diagnostics = $false; Fault = "user" },
        @{ Primary = $true; CleanupMsi = $false; Diagnostics = $false; Fault = "verify-user" },
        @{ Primary = $true; CleanupMsi = $false; Diagnostics = $false; Fault = "marker" },
        @{ Primary = $true; CleanupMsi = $false; Diagnostics = $false; Fault = "artifacts" }
    )) {
        Invoke-Case "integration cleanup primary=$($scenario.Primary) MSI=$($scenario.CleanupMsi) diagnostics=$($scenario.Diagnostics) fault=$($scenario.Fault)" {
            function Invoke-IntegrationLifecycle { Invoke-TestOperation }
            $script:cleanupFault = $scenario.Fault
            $script:diagnosticsFail = $scenario.Diagnostics
            if ($scenario.Primary) { $script:exitCodes["primary"] = 1603 }
            if ($scenario.CleanupMsi) { $script:exitCodes["cleanup-$($fixtureProducts[0].Trim('{}'))"] = 1603 }
            $failure = $null
            try { Invoke-IntegrationRun } catch { $failure = $_ }
            Assert-Failure $failure ($scenario.Primary -or $scenario.CleanupMsi -or $scenario.Fault -ne "") $scenario.Primary
            Assert-Test ($script:processCalls.Count -eq 2) "primary and allowlisted cleanup MSI both ran"
            Assert-Test ($script:relatedProducts -contains $foreignProduct) "foreign product retained"
            foreach ($attempt in @("user", "verify-user", "marker", "artifacts")) {
                Assert-Test ($script:cleanupAttempts -contains $attempt) "cleanup proceeds through $attempt"
            }
            $hasCleanupFailure = $scenario.CleanupMsi -or $scenario.Fault -ne ""
            Assert-Test (($script:cleanupFailures.Count -gt 0) -eq $hasCleanupFailure) "cleanup failure collected"
            Assert-Test ((@($script:hostLines | Where-Object { $_ -like 'Cleanup failure:*' }).Count -gt 0) -eq $hasCleanupFailure) "cleanup failure reported"
            Assert-Test ((Test-Path -LiteralPath $script:artifactsRoot) -eq ($scenario.Fault -eq "artifacts")) "integration temporary cleanup outcome"
            Assert-LogsSurvive
        }
    }
    Microsoft.PowerShell.Utility\Write-Host "PASS $caseCount installer harness cases"
}
finally {
    Microsoft.PowerShell.Management\Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
}
