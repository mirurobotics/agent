#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet("Debug", "Release")][string]$Configuration = "Release",
    [switch]$ConfirmDisposableTestMachine
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Assert-DisposableTestMachine {
    param([Parameter(Mandatory = $true)][bool]$Confirmed)
    $programDataRoot = Join-Path $env:ProgramData "Miru"
    if ($Confirmed) { return }
    if (Test-Path -LiteralPath $programDataRoot) {
        throw ("Refusing mutation of pre-existing $programDataRoot; " +
            "normal integration requires -ConfirmDisposableTestMachine.")
    }
    throw ("Normal integration is destructive and requires " +
        "-ConfirmDisposableTestMachine on a disposable test machine.")
}

function Initialize-IntegrationPaths {
    param([Parameter(Mandatory = $true)][string]$Configuration)
    $script:repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
    $script:projectPath = Join-Path $script:repositoryRoot `
        "build\windows\miru-agent.wixproj"
    $script:fixtureSource = Join-Path $PSScriptRoot "integration-test.wxs"
    $target = "target\x86_64-pc-windows-msvc\$($Configuration.ToLowerInvariant())"
    $script:binDir = Join-Path $script:repositoryRoot $target
    $runId = "miru-integration-tests-" + [Guid]::NewGuid().ToString("N")
    $script:artifactsRoot = Join-Path ([IO.Path]::GetTempPath()) $runId
    $script:deterministicLogs = Join-Path $script:repositoryRoot `
        "build\windows\artifacts\package-tests\logs"
    $script:sessionLogs = New-MsiSessionLogDirectory $script:deterministicLogs
    $script:programDataRoot = Join-Path $env:ProgramData "Miru"
    $script:logsRoot = Join-Path $script:programDataRoot "logs"
    $script:authRoot = Join-Path $script:programDataRoot "auth"
    $script:tmpRoot = Join-Path $script:programDataRoot "tmp"
    $script:protectedRoots = @(
        $script:programDataRoot,
        $script:logsRoot,
        $script:authRoot,
        $script:tmpRoot
    )
    $script:markerPath = Join-Path $script:programDataRoot "rollback-payload.txt"
    $script:customerOwnedFiles = @(
        (New-CustomerOwnedFile (Join-Path $script:programDataRoot `
            "integration-sentinel.txt") "retain-me"),
        (New-CustomerOwnedFile (Join-Path $script:logsRoot `
            "customer-owned.log") "customer-owned-log-retain"),
        (New-CustomerOwnedFile (Join-Path $script:authRoot `
            "customer-owned.auth") "customer-owned-auth-retain"),
        (New-CustomerOwnedFile (Join-Path $script:tmpRoot `
            "customer-owned.tmp") "customer-owned-tmp-retain")
    )
    $programFiles = [Environment]::GetEnvironmentVariable("ProgramW6432", "Process")
    $script:agentPath = Join-Path $programFiles "Miru\Agent\miru-agent.exe"
}

function New-CustomerOwnedFile {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Contents
    )
    [pscustomobject]@{ Path = $Path; Contents = $Contents }
}

function Initialize-IntegrationRuntime {
    $script:representativeFiles = New-Object System.Collections.ArrayList
    $script:fixtureProducts = @($MsiFixtureProductCodes)
    $script:testUser = "MiruMsiTestUser"
    $script:testPassword = "M!ru-" + [Guid]::NewGuid().ToString("N") + "-9a"
    $script:createdUser = $false
    $script:failureEvidence = New-Object System.Collections.ArrayList
    $script:cleanupFailures = New-Object System.Collections.ArrayList
    $script:integrationFailure = $null
}

function Assert-TestUserAbsent {
    param([Parameter(Mandatory = $true)][string]$Name)
    $existing = Get-LocalUser -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $existing) { return }
    throw "Refusing mutation: the named integration account $Name already exists."
}

function Assert-IntegrationPreconditions {
    Assert-Elevated64BitWindows
    $script:initialRelated = @(Assert-InstalledAllowlistSafe)
    Assert-TestUserAbsent $script:testUser
}

Assert-DisposableTestMachine ([bool]$ConfirmDisposableTestMachine)
Import-Module -Force -Name (Join-Path $PSScriptRoot "MsiTest.psm1")
. (Join-Path $PSScriptRoot "integration-lib.ps1")
Initialize-IntegrationPaths $Configuration
Initialize-IntegrationRuntime
Assert-IntegrationPreconditions
$script:artifactsRoot = Initialize-Directory $script:artifactsRoot
Invoke-IntegrationRun
