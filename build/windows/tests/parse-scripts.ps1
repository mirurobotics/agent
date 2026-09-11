[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$installPath = Join-Path $repositoryRoot "scripts\install\install.ps1"
$provisionPath = Join-Path $repositoryRoot "scripts\install\provision.ps1"

function Invoke-ParseFile {
    param([Parameter(Mandatory = $true)][string]$Path)

    $tokens = $null
    $errors = $null
    [System.Management.Automation.Language.Parser]::ParseFile(
        $Path,
        [ref]$tokens,
        [ref]$errors
    ) | Out-Null
    return @($errors)
}

Write-Host "Parsing $installPath"
$installErrors = @(Invoke-ParseFile -Path $installPath)
Write-Host "Parsing $provisionPath"
$provisionErrors = @(Invoke-ParseFile -Path $provisionPath)

$aggregateErrors = @()
foreach ($errorRecord in $installErrors) {
    $aggregateErrors += [pscustomobject]@{ Path = $installPath; Error = $errorRecord }
}
foreach ($errorRecord in $provisionErrors) {
    $aggregateErrors += [pscustomobject]@{ Path = $provisionPath; Error = $errorRecord }
}

foreach ($item in $aggregateErrors) {
    $extent = $item.Error.Extent
    Write-Host ("{0}:{1}:{2}: {3}" -f $item.Path, $extent.StartLineNumber, $extent.StartColumnNumber, $item.Error.Message)
}
Write-Host "Aggregate parse errors: $($aggregateErrors.Count)"
if ($aggregateErrors.Count -ne 0) {
    exit 1
}
exit 0
