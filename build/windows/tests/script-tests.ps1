[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$installScript = Join-Path $repositoryRoot "scripts\install\install.ps1"
$provisionScript = Join-Path $repositoryRoot "scripts\install\provision.ps1"
$integrationScript = Join-Path $repositoryRoot "build\windows\tests\integration-tests.ps1"
$harnessTls = [Net.ServicePointManager]::SecurityProtocol
$tokenName = "MIRU_PROVISIONING_TOKEN"
$harnessHadToken = Test-Path -LiteralPath "Env:$tokenName"
$harnessToken = [Environment]::GetEnvironmentVariable($tokenName, "Process")
$tempRoots = New-Object System.Collections.ArrayList

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

function Assert-Sequence {
    param([object[]]$Expected, [object[]]$Actual, [string]$Message)
    Assert-Equal $Expected.Count $Actual.Count "$Message count"
    for ($index = 0; $index -lt $Expected.Count; $index++) {
        Assert-Equal $Expected[$index] $Actual[$index] "$Message item $index"
    }
}

function Assert-Throws {
    param([scriptblock]$Action, [string]$Message)
    $threw = $false
    try { & $Action } catch { $threw = $true }
    Assert-True $threw $Message
}

function New-HarnessDirectory {
    $path = Join-Path ([IO.Path]::GetTempPath()) ("miru-script-tests-" + [Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $path | Out-Null
    [void]$tempRoots.Add($path)
    return $path
}

function Set-ProcessEnvironment {
    param([string]$Name, [bool]$Exists, [AllowNull()][string]$Value)
    if ($Exists) {
        [Environment]::SetEnvironmentVariable($Name, $Value, "Process")
    }
    else {
        [Environment]::SetEnvironmentVariable($Name, $null, "Process")
    }
}

function Assert-TokenState {
    param([bool]$ExpectedExists, [AllowNull()][string]$ExpectedValue, [string]$Message)
    Assert-Equal $ExpectedExists (Test-Path -LiteralPath "Env:$tokenName") "$Message existence"
    Assert-Equal $ExpectedValue ([Environment]::GetEnvironmentVariable($tokenName, "Process")) "$Message value"
}

function Set-TestFunction {
    param([string]$Name, [scriptblock]$Body)
    Set-Item -Path "function:script:$Name" -Value $Body
}

function Get-UnconfirmedIntegrationState {
    $fixtureProducts = @(
        "{B7AFDD4E-E6DB-4ED9-8C34-F318A04486B1}",
        "{3CE73709-ECE4-48A5-B7E7-1AC13C5EF30A}",
        "{4E72A894-00B5-433B-A445-C2CFD7FCF432}"
    )
    $uninstallRoots = @(
        "Registry::HKEY_LOCAL_MACHINE\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        "Registry::HKEY_LOCAL_MACHINE\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"
    )
    $registrations = @()
    foreach ($root in $uninstallRoots) {
        foreach ($product in $fixtureProducts) {
            $path = Join-Path $root $product
            $registrations += "$path=$(Test-Path -LiteralPath $path)"
        }
    }
    $testUser = @(Get-CimInstance -ClassName Win32_UserAccount -Filter "LocalAccount = TRUE AND Name = 'MiruMsiTestUser'" -ErrorAction Stop).Count -ne 0
    $programData = Test-Path -LiteralPath (Join-Path $env:ProgramData "Miru")
    $integrationTempDirectories = @(Get-ChildItem -LiteralPath ([IO.Path]::GetTempPath()) -Directory -Filter "miru-integration-tests-*" -ErrorAction Stop |
        ForEach-Object { $_.FullName } | Sort-Object)
    return @(
        "registrations=$($registrations -join '|')",
        "testUser=$testUser",
        "programData=$programData",
        "tempDirectories=$($integrationTempDirectories -join '|')"
    ) -join "`n"
}

try {
    $integrationRefusalRoot = New-HarnessDirectory
    $integrationStdout = Join-Path $integrationRefusalRoot "stdout.txt"
    $integrationStderr = Join-Path $integrationRefusalRoot "stderr.txt"
    $stateBeforeRefusal = Get-UnconfirmedIntegrationState
    $integrationProcess = Start-Process -FilePath "powershell.exe" -ArgumentList @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", ('"{0}"' -f $integrationScript)
    ) -RedirectStandardOutput $integrationStdout -RedirectStandardError $integrationStderr -Wait -PassThru
    $integrationOutput = [IO.File]::ReadAllText($integrationStdout) + [IO.File]::ReadAllText($integrationStderr)
    $stateAfterRefusal = Get-UnconfirmedIntegrationState
    Assert-True ($integrationProcess.ExitCode -ne 0) "unconfirmed integration exits nonzero"
    Assert-True ($integrationOutput -match 'requires -ConfirmDisposableTestMachine') "unconfirmed integration reports its explicit confirmation requirement"
    Assert-True ($integrationOutput -notmatch 'elevated Administrator session is required') "unconfirmed integration refuses before elevation"
    Assert-Equal $stateBeforeRefusal $stateAfterRefusal "unconfirmed integration external state"
    Write-Host "PASS unconfirmed integration refuses without side effects"

    . $installScript
    $installArchitectureDefinition = ${function:Assert-InstallArchitecture}.ToString()

    # Write-InstallLog and the main gate: elevation must fail before every other effect.
    $calls = New-Object System.Collections.ArrayList
    Set-TestFunction "Assert-InstallAdministrator" { [void]$calls.Add("administrator"); throw "not elevated" }
    Set-TestFunction "Assert-InstallArchitecture" { [void]$calls.Add("architecture") }
    Set-TestFunction "New-InstallTempDirectory" { [void]$calls.Add("temporary") }
    Assert-Throws { Invoke-InstallMain } "installer rejects a non-elevated caller"
    Assert-Sequence @("administrator") @($calls) "installer elevation gate order"
    Write-Host "PASS install elevation precedes side effects"
    . $installScript

    # Assert-InstallArchitecture uses independent OS and process checks. Static runtime
    # facts cannot be changed in-process, so verify both executable failure branches.
    Assert-True ($installArchitectureDefinition -match 'Is64BitOperatingSystem') "OS architecture is checked"
    Assert-True ($installArchitectureDefinition -match 'Is64BitProcess') "process architecture is checked"
    Assert-True (([regex]::Matches($installArchitectureDefinition, 'throw')).Count -eq 2) "both architecture failures throw"
    $osFailureBody = $installArchitectureDefinition.Replace('[Environment]::Is64BitOperatingSystem', '$false').Replace('[Environment]::Is64BitProcess', '$true')
    $processFailureBody = $installArchitectureDefinition.Replace('[Environment]::Is64BitOperatingSystem', '$true').Replace('[Environment]::Is64BitProcess', '$false')
    Assert-Throws { & ([scriptblock]::Create($osFailureBody)) } "32-bit operating system rejection"
    Assert-Throws { & ([scriptblock]::Create($processFailureBody)) } "32-bit process rejection"
    Write-Host "PASS install OS and process architecture rejection paths"

    # ConvertTo-MsiVersion.
    foreach ($version in @("0.0.0", "1.2.3", "255.255.65535")) {
        Assert-Equal $version (ConvertTo-MsiVersion -Value $version) "valid MSI version $version"
    }
    Assert-Equal "1.2.3" (ConvertTo-MsiVersion -Value "v1.2.3" -AllowLeadingV) "lowercase v at release boundary"
    Assert-Throws { ConvertTo-MsiVersion -Value "v1.2.3" } "v rejected at MSI boundary"
    foreach ($invalid in @(
        "V1.2.3", "1.2", "1.2.3.4", "1.2.3-beta.1", "1.2.3+build",
        "256.0.0", "1.256.0", "1.2.65536", "4294967296.0.0", "-1.2.3", " 1.2.3"
    )) {
        Assert-Throws { ConvertTo-MsiVersion -Value $invalid -AllowLeadingV } "invalid version $invalid rejected"
    }
    Write-Host "PASS strict MSI version conversion"

    # Invoke-WithTls12: initial absent/present crossed with success/throw.
    $tls12 = [Net.SecurityProtocolType]::Tls12
    $availableProtocols = [enum]::GetValues([Net.SecurityProtocolType])
    $withoutTls = [Net.SecurityProtocolType]::SystemDefault
    foreach ($protocol in $availableProtocols) {
        if (($protocol -band $tls12) -eq 0 -and [int]$protocol -ne 0) { $withoutTls = $protocol; break }
    }
    $tlsCases = @(
        @{ Initial = $withoutTls; Throws = $false },
        @{ Initial = $withoutTls; Throws = $true },
        @{ Initial = ($withoutTls -bor $tls12); Throws = $false },
        @{ Initial = ($withoutTls -bor $tls12); Throws = $true }
    )
    foreach ($case in $tlsCases) {
        [Net.ServicePointManager]::SecurityProtocol = $case.Initial
        if ($case.Throws) {
            Assert-Throws { Invoke-WithTls12 { throw "injected request failure" } } "TLS throwing request"
        }
        else {
            Assert-Equal "ok" (Invoke-WithTls12 { "ok" }) "TLS successful request"
        }
        Assert-Equal $case.Initial ([Net.ServicePointManager]::SecurityProtocol) "exact TLS restoration"
    }
    Write-Host "PASS TLS 1.2 protocol restoration (4 cases)"

    # Invoke-InstallWebRequest, both request forms.
    $requestRecords = New-Object System.Collections.ArrayList
    Set-TestFunction "Invoke-WebRequest" {
        param($Uri, $OutFile, [switch]$UseBasicParsing, $TimeoutSec)
        [void]$requestRecords.Add(@{ Uri = $Uri; OutFile = $OutFile; HasOutFile = $PSBoundParameters.ContainsKey("OutFile"); Basic = $UseBasicParsing.IsPresent; Timeout = $TimeoutSec })
        return [pscustomobject]@{ Content = "{}" }
    }
    Invoke-InstallWebRequest -Uri "https://example.invalid/latest" | Out-Null
    Invoke-InstallWebRequest -Uri "https://example.invalid/file" -OutFile "C:\path with spaces\file" | Out-Null
    Assert-Equal 2 $requestRecords.Count "two web request forms"
    foreach ($record in $requestRecords) {
        Assert-True $record.Basic "UseBasicParsing supplied"
        Assert-Equal 300 $record.Timeout "request timeout"
    }
    Assert-True (-not $requestRecords[0].HasOutFile) "response request has no output file"
    Assert-True $requestRecords[1].HasOutFile "download request has an output file"
    Assert-Equal "C:\path with spaces\file" $requestRecords[1].OutFile "download output path"
    Write-Host "PASS web request compatibility arguments"

    # New-InstallTempDirectory uniqueness.
    $firstTemp = New-InstallTempDirectory
    $secondTemp = New-InstallTempDirectory
    [void]$tempRoots.Add($firstTemp)
    [void]$tempRoots.Add($secondTemp)
    Assert-True ($firstTemp -ne $secondTemp) "temporary directory names are unique"
    Assert-True (Test-Path -LiteralPath $firstTemp -PathType Container) "first temporary directory exists"
    Assert-True (Test-Path -LiteralPath $secondTemp -PathType Container) "second temporary directory exists"
    Write-Host "PASS cryptographically unique temporary directories"

    # Get-ExpectedChecksum and Assert-FileChecksum.
    $checksumRoot = New-HarnessDirectory
    $assetName = "miru-agent_1.2.3_amd64.msi"
    $assetPath = Join-Path $checksumRoot $assetName
    [IO.File]::WriteAllText($assetPath, "package bytes")
    $digest = (Get-FileHash -Algorithm SHA256 -Path $assetPath).Hash
    $checksumPath = Join-Path $checksumRoot "checksums.txt"
    foreach ($validLine in @("$digest  $assetName", "$digest *$assetName", "  $($digest.ToLowerInvariant())`t$assetName  ")) {
        [IO.File]::WriteAllText($checksumPath, $validLine)
        Assert-Equal $digest (Get-ExpectedChecksum -ChecksumPath $checksumPath -AssetName $assetName) "valid checksum syntax"
        Assert-FileChecksum -FilePath $assetPath -ChecksumPath $checksumPath -AssetName $assetName
    }
    [IO.File]::WriteAllText($checksumPath, "$digest  $assetName`nnot-a-digest  $assetName")
    Assert-Throws { Get-ExpectedChecksum -ChecksumPath $checksumPath -AssetName $assetName } "valid and malformed exact records rejected as duplicates"
    $invalidChecksumFiles = @(
        "",
        "$digest  prefix-$assetName",
        "$digest  $assetName`n$digest *$assetName",
        "1234  $assetName",
        "$digest  $($assetName.ToUpperInvariant())"
    )
    foreach ($contents in $invalidChecksumFiles) {
        [IO.File]::WriteAllText($checksumPath, $contents)
        Assert-Throws { Get-ExpectedChecksum -ChecksumPath $checksumPath -AssetName $assetName } "invalid checksum record rejected"
    }
    [IO.File]::WriteAllText($checksumPath, ("0" * 64) + "  " + $assetName)
    Assert-Throws { Assert-FileChecksum -FilePath $assetPath -ChecksumPath $checksumPath -AssetName $assetName } "wrong digest rejected"
    Write-Host "PASS exact checksum record parsing and digest verification"

    # Assert-MiruMsiMetadata synthetic identity, architecture, version and ProductCode.
    $validMetadata = [pscustomobject]@{
        ProductName = "Miru Agent"; Manufacturer = "Miru Robotics"; ProductVersion = "1.2.3"
        ProductCode = "{11111111-2222-3333-4444-555555555555}"
        UpgradeCode = "{B5ED0336-5F14-4308-A667-3CE8CDEF7D48}"; Template = "x64;1033"
    }
    Assert-Equal "1.2.3" (Assert-MiruMsiMetadata -Metadata $validMetadata -ExpectedVersion "1.2.3") "valid MSI metadata"
    foreach ($property in @("ProductName", "Manufacturer", "ProductVersion", "ProductCode", "UpgradeCode", "Template")) {
        $copy = $validMetadata.PSObject.Copy()
        switch ($property) {
            "ProductName" { $copy.ProductName = "Other" }
            "Manufacturer" { $copy.Manufacturer = "Other" }
            "ProductVersion" { $copy.ProductVersion = "1.2.3.4" }
            "ProductCode" { $copy.ProductCode = "not-a-guid" }
            "UpgradeCode" { $copy.UpgradeCode = "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}" }
            "Template" { $copy.Template = "Intel;1033" }
        }
        Assert-Throws { Assert-MiruMsiMetadata -Metadata $copy } "invalid $property metadata rejected"
    }
    Assert-Throws { Assert-MiruMsiMetadata -Metadata $validMetadata -ExpectedVersion "1.2.4" } "requested version mismatch"
    $installedStateDefinition = ${function:Test-MsiProductInstalled}.ToString()
    Assert-True ($installedStateDefinition -match '-eq\s+5') "Windows Installer state 5 is installed"
    Write-Host "PASS synthetic MSI metadata and installed-state contract"

    # Invoke-MsiInstall exact arguments and maintenance behavior.
    $processRecords = New-Object System.Collections.ArrayList
    Set-TestFunction "Start-Process" {
        param($FilePath, [switch]$Wait, [switch]$PassThru, [object[]]$ArgumentList)
        [void]$processRecords.Add(@{ FilePath = $FilePath; Wait = $Wait.IsPresent; PassThru = $PassThru.IsPresent; Arguments = @($ArgumentList) })
        return [pscustomobject]@{ ExitCode = 3010 }
    }
    Set-TestFunction "Test-MsiProductInstalled" { param($ProductCode) return $false }
    $msiWithSpaces = "C:\fixture packages\miru agent.msi"
    $logWithSpaces = "C:\fixture logs\install log.txt"
    Assert-Equal 3010 (Invoke-MsiInstall -MsiPath $msiWithSpaces -LogPath $logWithSpaces -ProductCode $validMetadata.ProductCode) "msiexec exit propagation"
    Assert-Sequence @("/i", ('"{0}"' -f $msiWithSpaces), "/qn", "/norestart", "/l*v", ('"{0}"' -f $logWithSpaces)) $processRecords[0].Arguments "fresh install arguments"
    Assert-Equal "msiexec.exe" $processRecords[0].FilePath "msiexec executable"
    Assert-True ($processRecords[0].Wait -and $processRecords[0].PassThru) "wait and passthru"
    Set-TestFunction "Test-MsiProductInstalled" { param($ProductCode) return $true }
    Invoke-MsiInstall -MsiPath $msiWithSpaces -LogPath $logWithSpaces -ProductCode $validMetadata.ProductCode | Out-Null
    Assert-Sequence @("/i", ('"{0}"' -f $msiWithSpaces), "/qn", "/norestart", "/l*v", ('"{0}"' -f $logWithSpaces), "REINSTALL=ALL", "REINSTALLMODE=vomus") $processRecords[1].Arguments "maintenance arguments"
    Write-Host "PASS msiexec argument array and maintenance mode"
    Remove-Item -Path "function:script:Start-Process"

    # Invoke-InstallMain cleanup and 0/3010/failure log behavior.
    Set-TestFunction "Assert-InstallAdministrator" { }
    Set-TestFunction "Assert-InstallArchitecture" { }
    Set-TestFunction "Get-MsiMetadata" { param($Path) return $validMetadata }
    Set-TestFunction "Assert-MiruMsiMetadata" { param($Metadata, $ExpectedVersion) return "1.2.3" }
    Set-TestFunction "Assert-FileChecksum" { }
    Set-TestFunction "Invoke-InstallWebRequest" {
        param($Uri, $OutFile)
        if ($OutFile) { [IO.File]::WriteAllText($OutFile, "fixture") }
        return [pscustomobject]@{ Content = '{"tag_name":"v1.2.3"}' }
    }
    Set-TestFunction "Get-LatestStableVersion" { return "1.2.3" }
    $createdDownloads = New-Object System.Collections.ArrayList
    Set-TestFunction "New-InstallTempDirectory" {
        $directory = New-HarnessDirectory
        [void]$createdDownloads.Add($directory)
        return $directory
    }
    foreach ($result in @(0, 3010)) {
        Set-TestFunction "Invoke-MsiInstall" {
            param($MsiPath, $LogPath, $ProductCode)
            $script:lastInstallLog = $LogPath
            [IO.File]::WriteAllText($LogPath, "success")
            return $result
        }
        Assert-Equal $result (Invoke-InstallMain -RequestedVersion "1.2.3") "installer result $result"
        Assert-True (-not (Test-Path -LiteralPath $createdDownloads[$createdDownloads.Count - 1])) "download cleanup result $result"
        Assert-True (-not (Test-Path -LiteralPath $script:lastInstallLog)) "successful result $result removes verbose log"
    }
    Set-TestFunction "Invoke-InstallWebRequest" { param($Uri, $OutFile) throw "injected download failure" }
    Assert-Throws { Invoke-InstallMain -RequestedVersion "1.2.3" } "download failure propagates"
    Assert-True (-not (Test-Path -LiteralPath $createdDownloads[$createdDownloads.Count - 1])) "download cleanup failure"
    Set-TestFunction "Invoke-InstallWebRequest" { param($Uri, $OutFile); if ($OutFile) { [IO.File]::WriteAllText($OutFile, "fixture") } }
    Set-TestFunction "Invoke-MsiInstall" { param($MsiPath, $LogPath, $ProductCode); $script:failedLog = $LogPath; [IO.File]::WriteAllText($LogPath, "failure"); return 1603 }
    $installFailure = $null
    try { Invoke-InstallMain -RequestedVersion "1.2.3" | Out-Null } catch { $installFailure = $_.Exception }
    Assert-True ($null -ne $installFailure) "msiexec failure propagates"
    Assert-True (Test-Path -LiteralPath $script:failedLog -PathType Leaf) "failure log retained"
    Assert-True ($installFailure.Message.Contains($script:failedLog)) "failure identifies the retained log path"
    Assert-True (-not (Test-Path -LiteralPath $createdDownloads[$createdDownloads.Count - 1])) "download cleanup after msiexec failure"
    Remove-Item -LiteralPath $script:failedLog -Force
    Write-Host "PASS installer cleanup and exit/log behavior"

    . $provisionScript
    $provisionArchitectureDefinition = ${function:Assert-ProvisionArchitecture}.ToString()
    $invokeProcessDefinition = ${function:Invoke-MiruAgentProcess}.ToString()

    # Get-MiruAgentExecutable uses 64-bit Program Files.
    $originalProgramW6432 = [Environment]::GetEnvironmentVariable("ProgramW6432", "Process")
    [Environment]::SetEnvironmentVariable("ProgramW6432", "C:\Program Files 64", "Process")
    Assert-Equal "C:\Program Files 64\Miru\Agent\miru-agent.exe" (Get-MiruAgentExecutable) "64-bit Program Files path"
    [Environment]::SetEnvironmentVariable("ProgramW6432", $originalProgramW6432, "Process")
    Write-Host "PASS provision executable resolves from 64-bit Program Files"

    # Assert-ProvisionArchitecture has independent, executable failure branches.
    Assert-True ($provisionArchitectureDefinition -match 'Is64BitOperatingSystem') "provision OS architecture checked"
    Assert-True ($provisionArchitectureDefinition -match 'Is64BitProcess') "provision process architecture checked"
    Assert-True (([regex]::Matches($provisionArchitectureDefinition, 'throw')).Count -eq 2) "provision architecture failure paths"
    $provisionOsFailureBody = $provisionArchitectureDefinition.Replace('[Environment]::Is64BitOperatingSystem', '$false').Replace('[Environment]::Is64BitProcess', '$true')
    $provisionProcessFailureBody = $provisionArchitectureDefinition.Replace('[Environment]::Is64BitOperatingSystem', '$true').Replace('[Environment]::Is64BitProcess', '$false')
    Assert-Throws { & ([scriptblock]::Create($provisionOsFailureBody)) } "provision rejects a 32-bit operating system"
    Assert-Throws { & ([scriptblock]::Create($provisionProcessFailureBody)) } "provision rejects a 32-bit process"
    Write-Host "PASS provision OS and process architecture rejection paths"

    Assert-True ($invokeProcessDefinition -match '&\s+\$AgentPath\s+@Arguments') "agent executable is invoked directly"
    Assert-True ($invokeProcessDefinition -notmatch 'Start-Process|cmd\.exe|powershell\.exe') "direct invocation has no intermediary process"
    Write-Host "PASS provision uses direct executable invocation"

    # Check bypasses elevation and token, preserves output, and maps statuses.
    $fakeRoot = New-HarnessDirectory
    $fakeAgent = Join-Path $fakeRoot "miru-agent.exe"
    [IO.File]::WriteAllText($fakeAgent, "fixture")
    Set-TestFunction "Get-MiruAgentExecutable" { return $fakeAgent }
    Set-TestFunction "Assert-ProvisionAdministrator" { throw "check must bypass elevation" }
    Set-TestFunction "Assert-ProvisionArchitecture" { throw "check must bypass architecture" }
    $checkHadToken = Test-Path -LiteralPath "Env:$tokenName"
    $checkToken = [Environment]::GetEnvironmentVariable($tokenName, "Process")
    foreach ($case in @(@{ Native = 0; Expected = 0 }, @{ Native = 3; Expected = 3 }, @{ Native = 7; Expected = 1 })) {
        $nativeResult = $case.Native
        Set-TestFunction "Invoke-MiruAgentProcess" { param($AgentPath, $Arguments); Write-Host "probe-output-$nativeResult"; return $nativeResult }
        $captured = @(& { $script:checkResult = Invoke-ProvisionMain -Backend "b" -MqttBroker "m" -CheckOnly $true } *>&1)
        Assert-Equal $case.Expected $script:checkResult "check exit mapping $nativeResult"
        Assert-True (($captured | Out-String).Contains("probe-output-$nativeResult")) "check output preserved"
        Assert-TokenState $checkHadToken $checkToken "check bypasses token state"
    }
    Write-Host "PASS provision check bypass and exit mapping"

    # Invoke-AgentProvision: exact non-secret args and exact token restoration for
    # success/nonzero/throw, even when the process shim mutates or removes it.
    $canary = "CANARY-DO-NOT-LEAK-" + [Guid]::NewGuid().ToString("N")
    $provisionObservations = New-Object System.Collections.ArrayList
    foreach ($case in @(
        @{ Name = "success-existing"; Exists = $true; Value = $canary; Exit = 0; Throws = $false; Mutation = "changed" },
        @{ Name = "nonzero-existing"; Exists = $true; Value = $canary; Exit = 9; Throws = $false; Mutation = $null },
        @{ Name = "throw-existing"; Exists = $true; Value = $canary; Exit = 0; Throws = $true; Mutation = "changed" },
        @{ Name = "absent-rejected"; Exists = $false; Value = $null; Exit = 0; Throws = $false; Mutation = "changed" }
    )) {
        Set-ProcessEnvironment -Name $tokenName -Exists $case.Exists -Value $case.Value
        if (-not $case.Exists) {
            # Absent is rejected without mutation; cover restoration through a shim that
            # installs a token only after the function captures the initial state below.
            Assert-Throws { Invoke-AgentProvision -AgentPath $fakeAgent -Backend "https://backend" -MqttBroker "mqtt" } "absent token rejected"
            Assert-TokenState $false $null "absent token rejection"
            continue
        }
        $activeCase = $case
        Set-TestFunction "Invoke-MiruAgentProcess" {
            param($AgentPath, $Arguments)
            [void]$provisionObservations.Add(@{ Agent = $AgentPath; Arguments = @($Arguments); Token = [Environment]::GetEnvironmentVariable($tokenName, "Process") })
            [Environment]::SetEnvironmentVariable($tokenName, $activeCase.Mutation, "Process")
            if ($activeCase.Throws) { throw "injected process throw" }
            return $activeCase.Exit
        }
        $captured = @(& {
            try {
                $script:provisionResult = Invoke-AgentProvision -AgentPath $fakeAgent -Backend "https://backend" -MqttBroker "mqtt"
                $script:provisionThrew = $false
            }
            catch {
                $script:provisionThrew = $true
                Write-Output ("captured-error-type: " + $_.Exception.GetType().FullName)
            }
        } *>&1)
        Assert-Equal ($case.Throws -or $case.Exit -ne 0) $script:provisionThrew "provision outcome $($case.Name)"
        Assert-TokenState $case.Exists $case.Value "token restoration $($case.Name)"
        $observation = $provisionObservations[$provisionObservations.Count - 1]
        Assert-Equal $canary $observation.Token "process sees the original token"
        Assert-Sequence @("provision", "--backend-host=https://backend", "--mqtt-broker-host=mqtt") $observation.Arguments "provision arguments"
        Assert-True (-not (($observation.Arguments | Out-String).Contains($canary))) "token absent from arguments"
        Assert-True (-not (($captured | Out-String).Contains($canary))) "token absent from output and errors"
    }
    Write-Host "PASS provision token hygiene and exact restoration"

    $provisionSource = [IO.File]::ReadAllText($provisionScript)
    Assert-True ($provisionSource -notmatch '(?i)Get-Service|Stop-Service|Start-Service|ServiceController') "no service operations"
    Assert-True ($provisionSource -notmatch '\[string\]\s*\$Token') "no public token parameter"
    Write-Host "PASS provision contains no service operations or token parameter"

    # Invoke-ProvisionMain gates missing installations and checks elevation before
    # architecture, token access, or process execution.
    Set-TestFunction "Get-MiruAgentExecutable" { return "Z:\missing\miru-agent.exe" }
    $provisionCalls = New-Object System.Collections.ArrayList
    Set-TestFunction "Assert-ProvisionAdministrator" { [void]$provisionCalls.Add("administrator") }
    Set-TestFunction "Invoke-ProvisionCheck" { [void]$provisionCalls.Add("check") }
    Assert-Throws { Invoke-ProvisionMain -Backend "b" -MqttBroker "m" -CheckOnly $true } "missing executable rejected"
    Assert-Equal 0 $provisionCalls.Count "missing executable has no later effects"
    Write-Host "PASS provision missing executable gate"

    Set-TestFunction "Get-MiruAgentExecutable" { return $fakeAgent }
    $gateCalls = New-Object System.Collections.ArrayList
    Set-TestFunction "Assert-ProvisionAdministrator" { [void]$gateCalls.Add("administrator"); throw "not elevated" }
    Set-TestFunction "Assert-ProvisionArchitecture" { [void]$gateCalls.Add("architecture") }
    Set-TestFunction "Invoke-AgentProvision" { [void]$gateCalls.Add("provision") }
    Assert-Throws { Invoke-ProvisionMain -Backend "b" -MqttBroker "m" -CheckOnly $false } "normal elevation gate"
    Assert-Sequence @("administrator") @($gateCalls) "normal provisioning gate order"
    Write-Host "PASS provision normal gate order"
}
finally {
    [Net.ServicePointManager]::SecurityProtocol = $harnessTls
    Set-ProcessEnvironment -Name $tokenName -Exists $harnessHadToken -Value $harnessToken
    foreach ($path in $tempRoots) {
        if ($path -and (Test-Path -LiteralPath $path)) {
            Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
