<#
.SYNOPSIS
    Provision an installed Miru Agent, or inspect its provisioning state.

.DESCRIPTION
    Normal provisioning reads its secret only from the process-level
    MIRU_PROVISIONING_TOKEN environment variable and never places it on the
    command line. The script invokes the installed console executable directly;
    Windows service registration and lifecycle are intentionally deferred.

.PARAMETER BackendHost
    Backend API URL. Defaults to the production host.

.PARAMETER MqttBrokerHost
    MQTT broker host. Defaults to the production broker.

.PARAMETER Check
    Run the read-only `provision --check` probe. Returns 0 when provisioned, 3
    when not provisioned, and 1 when state cannot be determined.
#>
[CmdletBinding()]
param(
    [string]$BackendHost = "https://api.mirurobotics.com",
    [string]$MqttBrokerHost = "mqtt.mirurobotics.com",
    [switch]$Check
)

$ErrorActionPreference = "Stop"

function Write-ProvisionLog {
    param([string]$Message)

    Write-Host "==> $Message" -ForegroundColor Green
}

function Get-MiruAgentExecutable {
    $programFiles64 = [Environment]::GetEnvironmentVariable("ProgramW6432", "Process")
    if (-not $programFiles64) {
        $programFiles64 = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFiles)
    }
    return Join-Path $programFiles64 "Miru\Agent\miru-agent.exe"
}

function Assert-ProvisionAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "Run provisioning from an elevated Administrator PowerShell session."
    }
}

function Assert-ProvisionArchitecture {
    if (-not [Environment]::Is64BitOperatingSystem) {
        throw "The Miru Agent supports 64-bit Windows only."
    }
    if (-not [Environment]::Is64BitProcess) {
        throw "Run provisioning from 64-bit PowerShell."
    }
}

function Invoke-MiruAgentProcess {
    param(
        [Parameter(Mandatory = $true)][string]$AgentPath,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    & $AgentPath @Arguments | ForEach-Object { Write-Host $_ }
    return $LASTEXITCODE
}

function Invoke-ProvisionCheck {
    param([Parameter(Mandatory = $true)][string]$AgentPath)

    $exitCode = Invoke-MiruAgentProcess -AgentPath $AgentPath -Arguments @("provision", "--check")
    if ($exitCode -eq 0 -or $exitCode -eq 3) {
        return $exitCode
    }
    return 1
}

function Invoke-AgentProvision {
    param(
        [Parameter(Mandatory = $true)][string]$AgentPath,
        [Parameter(Mandatory = $true)][string]$Backend,
        [Parameter(Mandatory = $true)][string]$MqttBroker
    )

    $tokenName = "MIRU_PROVISIONING_TOKEN"
    $hadToken = Test-Path -LiteralPath "Env:$tokenName"
    $originalToken = [Environment]::GetEnvironmentVariable($tokenName, "Process")
    if ([string]::IsNullOrWhiteSpace($originalToken)) {
        throw "Set MIRU_PROVISIONING_TOKEN to a non-empty provisioning token."
    }

    try {
        [Environment]::SetEnvironmentVariable($tokenName, $originalToken, "Process")
        Write-ProvisionLog "Provisioning the Miru Agent"
        $arguments = @(
            "provision",
            "--backend-host=$Backend",
            "--mqtt-broker-host=$MqttBroker"
        )
        $exitCode = Invoke-MiruAgentProcess -AgentPath $AgentPath -Arguments $arguments
        if ($exitCode -ne 0) {
            throw "Provisioning failed with exit code $exitCode."
        }
        Write-ProvisionLog "Provisioned successfully."
        return 0
    }
    finally {
        if ($hadToken) {
            [Environment]::SetEnvironmentVariable($tokenName, $originalToken, "Process")
        }
        else {
            [Environment]::SetEnvironmentVariable($tokenName, $null, "Process")
        }
    }
}

function Invoke-ProvisionMain {
    param(
        [string]$Backend,
        [string]$MqttBroker,
        [bool]$CheckOnly
    )

    $agentPath = Get-MiruAgentExecutable
    if (-not (Test-Path -LiteralPath $agentPath -PathType Leaf)) {
        throw "Miru Agent is not installed at $agentPath. Run install.ps1 first."
    }
    if ($CheckOnly) {
        return Invoke-ProvisionCheck -AgentPath $agentPath
    }

    Assert-ProvisionAdministrator
    Assert-ProvisionArchitecture
    return Invoke-AgentProvision -AgentPath $agentPath -Backend $Backend -MqttBroker $MqttBroker
}

if ($MyInvocation.InvocationName -ne '.') {
    try {
        exit (Invoke-ProvisionMain -Backend $BackendHost -MqttBroker $MqttBrokerHost -CheckOnly $Check.IsPresent)
    }
    catch {
        Write-Host "Error: $($_.Exception.Message)" -ForegroundColor Red
        exit 1
    }
}
