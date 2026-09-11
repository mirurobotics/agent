<#
.SYNOPSIS
    Provision (activate) an installed Miru Agent on Windows.

.DESCRIPTION
    Windows parity for the modern provisioning flow (frontend emits
    `MIRU_PROVISIONING_TOKEN=... miru-agent provision` on Linux; the pre-0.9
    scripts/install/provision.sh API-key flow is deprecated). Runs the installed
    agent's `provision` subcommand with the provisioning token supplied via the
    MIRU_PROVISIONING_TOKEN environment variable, stopping and restarting the
    service around it.

    Must run elevated — provisioning writes device identity + keys under
    %ProgramData%\Miru, and the service runs as LocalSystem.

    SCAFFOLDING — not yet exercised end-to-end (depends on the MSI install).

.PARAMETER Token
    The provisioning token. Falls back to $env:MIRU_PROVISIONING_TOKEN.

.PARAMETER BackendHost
    Backend API URL. Defaults to the production host.

.PARAMETER MqttBrokerHost
    MQTT broker host. Defaults to the production broker.

.PARAMETER Check
    Run `provision --check` (read-only probe) and exit with the agent's own
    status code: 0 provisioned, 3 not provisioned, 1 undetermined. Preserves the
    exit-code contract from `miru-agent provision --check`.

.EXAMPLE
    $env:MIRU_PROVISIONING_TOKEN = "mpt_..."
    powershell -ExecutionPolicy Bypass -File provision.ps1 -BackendHost https://api.mirurobotics.com
#>
[CmdletBinding()]
param(
    [string]$Token = $env:MIRU_PROVISIONING_TOKEN,
    [string]$BackendHost = "https://api.mirurobotics.com",
    [string]$MqttBrokerHost = "mqtt.mirurobotics.com",
    [switch]$Check
)

$ErrorActionPreference = "Stop"
$ServiceName = "MiruAgent"
$AgentExe = Join-Path ${env:ProgramFiles} "Miru\Agent\miru-agent.exe"

function Write-Log { param($m) Write-Host "==> $m" -ForegroundColor Green }
function Die { param($m) Write-Host "Error: $m" -ForegroundColor Red; exit 1 }

if (-not (Test-Path $AgentExe)) {
    Die "Miru Agent is not installed at $AgentExe. Run install.ps1 first."
}

# Read-only probe: mirror `provision --check` and propagate its exit code.
if ($Check) {
    & $AgentExe provision --check
    exit $LASTEXITCODE
}

if (-not $Token) {
    Die "No provisioning token. Pass -Token or set MIRU_PROVISIONING_TOKEN."
}

# Stop the service so provisioning owns the state directory exclusively.
if ((Get-Service -Name $ServiceName -ErrorAction SilentlyContinue).Status -eq "Running") {
    Write-Log "Stopping the Miru Agent service"
    Stop-Service -Name $ServiceName
}

# Always restart the service on the way out, success or failure.
try {
    Write-Log "Provisioning the Miru Agent..."
    $env:MIRU_PROVISIONING_TOKEN = $Token
    & $AgentExe provision --backend-host=$BackendHost --mqtt-broker-host=$MqttBrokerHost
    if ($LASTEXITCODE -ne 0) { Die "Provisioning failed (exit $LASTEXITCODE)" }
    Write-Log "Provisioned successfully."
}
finally {
    $env:MIRU_PROVISIONING_TOKEN = $null
    Write-Log "Starting the Miru Agent service"
    Start-Service -Name $ServiceName -ErrorAction SilentlyContinue
}
