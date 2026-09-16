#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ManifestPath,
    [Parameter(Mandatory = $true)][string]$ResultPath
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Test-AccessDeniedException {
    param([Parameter(Mandatory = $true)]$Exception)
    $cause = $Exception.GetBaseException()
    if ($cause -is [UnauthorizedAccessException]) { return $true }
    $isAccessDenied = $cause -is [ComponentModel.Win32Exception] -and `
        $cause.NativeErrorCode -eq 5
    return $isAccessDenied
}

function Invoke-AccessAttempt {
    param([Parameter(Mandatory = $true)][scriptblock]$Action)
    try {
        & $Action | Out-Null
        return "Allowed"
    }
    catch {
        if (Test-AccessDeniedException $_.Exception) { return "AccessDenied" }
        throw
    }
}

function Set-PermissiveDacl {
    param([Parameter(Mandatory = $true)][string]$Path)
    # Default All also requests an audit update requiring a separate privilege.
    # Persist clears modification flags, so each attempt needs a fresh descriptor.
    $replacement = New-Object Security.AccessControl.DirectorySecurity
    $sections = [Security.AccessControl.AccessControlSections]::Access
    $replacement.SetSecurityDescriptorSddlForm(
        "D:P(A;OICI;FA;;;WD)", $sections)
    [IO.Directory]::SetAccessControl($Path, $replacement)
}

function ConvertTo-ProbeFile {
    param([Parameter(Mandatory = $true)]$Raw)
    if ([string]::IsNullOrWhiteSpace($Raw.Path) -or
        [string]::IsNullOrWhiteSpace($Raw.CreatePath) -or
        [string]::IsNullOrWhiteSpace($Raw.Parent)) {
        throw "Probe manifest file is missing Path, CreatePath, or Parent"
    }
    return [pscustomobject]@{
        Path = [string]$Raw.Path
        CreatePath = [string]$Raw.CreatePath
        Parent = [string]$Raw.Parent
    }
}

function ConvertTo-ProbeManifest {
    param([Parameter(Mandatory = $true)][string]$Path)
    $raw = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace($raw.ControlPath)) {
        throw "Probe manifest is missing ControlPath"
    }
    if ([string]::IsNullOrWhiteSpace($raw.ProbeRoot)) {
        throw "Probe manifest is missing ProbeRoot"
    }
    $files = @($raw.Files | ForEach-Object { ConvertTo-ProbeFile $_ })
    if ($files.Count -eq 0) { throw "Probe manifest Files is empty" }
    return [pscustomobject]@{
        ControlPath = [string]$raw.ControlPath
        ProbeRoot = [string]$raw.ProbeRoot
        Files = $files
    }
}

function Assert-ControlContents {
    param([Parameter(Mandatory = $true)][string]$Path)
    $contents = [IO.File]::ReadAllText($Path)
    if ($contents -ne "probe-control") { throw "Probe control mismatch" }
}

function Invoke-ControlProbe {
    param([Parameter(Mandatory = $true)]$Manifest)
    return [pscustomobject]@{
        Create = Invoke-AccessAttempt {
            [IO.File]::WriteAllText($Manifest.ControlPath, "probe-control")
        }
        Read = Invoke-AccessAttempt { Assert-ControlContents $Manifest.ControlPath }
        Regrant = Invoke-AccessAttempt { Set-PermissiveDacl $Manifest.ProbeRoot }
    }
}

function Invoke-ProtectedFileProbe {
    param([Parameter(Mandatory = $true)]$File)
    return [pscustomobject]@{
        Path = $File.Path
        Read = Invoke-AccessAttempt { [IO.File]::ReadAllText($File.Path) }
        Create = Invoke-AccessAttempt {
            [IO.File]::WriteAllText($File.CreatePath, "unexpected")
        }
        Regrant = Invoke-AccessAttempt { Set-PermissiveDacl $File.Parent }
    }
}

function Invoke-NonAdminProbe {
    param([Parameter(Mandatory = $true)]$Manifest)
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    $builtInAdmin = [Security.Principal.WindowsBuiltInRole]::Administrator
    return [pscustomobject]@{
        Sid = $identity.User.Value
        Administrator = $principal.IsInRole($builtInAdmin)
        Control = Invoke-ControlProbe $Manifest
        Results = @($Manifest.Files | ForEach-Object {
            Invoke-ProtectedFileProbe $_
        })
    }
}

$manifest = ConvertTo-ProbeManifest $ManifestPath
Invoke-NonAdminProbe $manifest | ConvertTo-Json -Depth 4 |
    Set-Content -LiteralPath $ResultPath
