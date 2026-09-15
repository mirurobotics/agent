#Requires -Version 5.1
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ManifestPath,
    [Parameter(Mandatory = $true)][string]$ResultPath
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Get-AccessOutcome {
    param([Parameter(Mandatory = $true)][scriptblock]$Action)
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
    param([Parameter(Mandatory = $true)][string]$Path)
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
