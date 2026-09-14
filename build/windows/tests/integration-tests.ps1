#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")][string]$Configuration = "Release",
    [switch]$ConfirmDisposableTestMachine,
    [switch]$ManualProductionSmoke,
    [switch]$ConfirmDisposableCleanVm,
    [string]$TranscriptPath = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
Import-Module -Force -Name (Join-Path $PSScriptRoot "MsiTest.psm1")
. (Join-Path $PSScriptRoot "integration-lib.ps1")

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$projectPath = Join-Path $repositoryRoot "build\windows\miru-agent.wixproj"
$fixtureSource = Join-Path $PSScriptRoot "integration-test.wxs"
$binDir = Join-Path $repositoryRoot "target\x86_64-pc-windows-msvc\$($Configuration.ToLowerInvariant())"
$artifactsRoot = Join-Path ([IO.Path]::GetTempPath()) ("miru-integration-tests-" + [Guid]::NewGuid().ToString("N"))
$deterministicLogs = Join-Path $repositoryRoot "build\windows\artifacts\package-tests\logs"
$sessionLogs = New-MsiSessionLogDirectory $deterministicLogs
$programDataRoot = Join-Path $env:ProgramData "Miru"
$logsRoot = Join-Path $programDataRoot "logs"
$protectedRoots = @($programDataRoot, $logsRoot, (Join-Path $programDataRoot "auth"), (Join-Path $programDataRoot "tmp"))
$representativeFiles = New-Object System.Collections.ArrayList
$markerPath = Join-Path $programDataRoot "rollback-payload.txt"
$sentinelPath = Join-Path $programDataRoot "integration-sentinel.txt"
$customerLogPath = Join-Path $logsRoot "customer-owned.log"
$customerLogContents = "customer-owned-log-retain"
$agentPath = Join-Path ([Environment]::GetEnvironmentVariable("ProgramW6432", "Process")) "Miru\Agent\miru-agent.exe"
$fixtureProducts = @($MsiFixtureProductCodes)
$testUser = "MiruMsiTestUser"
$testPassword = "M!ru-" + [Guid]::NewGuid().ToString("N") + "-9a"
$createdUser = $false
$failureEvidence = New-Object System.Collections.ArrayList
$cleanupFailures = New-Object System.Collections.ArrayList
$integrationFailure = $null
$transcriptStarted = $false

if (-not $ManualProductionSmoke -and -not $ConfirmDisposableTestMachine) {
    if (Test-Path -LiteralPath $programDataRoot) {
        throw "Refusing mutation of pre-existing $programDataRoot; normal integration requires -ConfirmDisposableTestMachine."
    }
    throw "Normal integration is destructive and requires -ConfirmDisposableTestMachine on a disposable test machine."
}

Assert-Elevated64BitWindows
if ($ManualProductionSmoke) {
    Invoke-ManualRun
    exit 0
}

$initialRelated = @(Assert-InstalledAllowlistSafe)
$existingUser = Get-LocalUser -Name $testUser -ErrorAction SilentlyContinue
if ($null -ne $existingUser) {
    throw "Refusing mutation: the named integration account $testUser already exists."
}
New-Item -ItemType Directory -Path $artifactsRoot -Force | Out-Null
Invoke-IntegrationRun
