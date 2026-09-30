#Requires -Version 5.1
# Restores the WiX project only from nuget.org package files whose SHA-512
# matches the pins below. packages.lock.json pins the extension, but the
# WixToolset.Sdk in <Project Sdk=...> is resolved by the MSBuild SDK resolver,
# which lock files do not cover. Here both packages are downloaded, verified,
# and served from a local folder that is the only NuGet source, so no MSBuild
# code from an unverified package runs.
#
# The pins are the `packageHash` values from nuget.org's catalog (SHA-512 of
# the .nupkg as served). To update a version, take the new value from
# https://api.nuget.org/v3/registration5-semver1/<id>/<version>.json ->
# catalogEntry -> packageHash, and regenerate packages.lock.json.
param(
    [Parameter(Mandatory = $true)][string]$ProjectPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$pins = [ordered]@{
    "wixtoolset.sdk/7.0.0"         = "E6sydem/3Fj08mhN9BTvNzw5JwdzKpKQ8JA6hs6BDYQt6ytTVr+2kYaZ/AH+x87z9iSZGyblj2y9y2zcNVtKyg=="
    "wixtoolset.util.wixext/7.0.0" = "KyfrcZmG5Nh3yNdqknM5imam1YfQ6eb6Rc+LVYks7hXhPwFAwBsIqyRp/wJ3cEfNy5N88oBTUf50f3yg7esyxQ=="
}

$tempRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$feed = Join-Path $tempRoot "wix-pinned-feed"
$packages = Join-Path $tempRoot "wix-pinned-packages"
foreach ($dir in @($feed, $packages)) {
    if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
    New-Item -ItemType Directory -Path $dir | Out-Null
}

foreach ($pin in $pins.GetEnumerator()) {
    $id, $version = $pin.Key -split "/"
    $file = Join-Path $feed "$id.$version.nupkg"
    Invoke-WebRequest -UseBasicParsing -OutFile $file `
        -Uri "https://api.nuget.org/v3-flatcontainer/$id/$version/$id.$version.nupkg"
    $sha512 = [Security.Cryptography.SHA512]::Create()
    $stream = [IO.File]::OpenRead($file)
    try { $actual = [Convert]::ToBase64String($sha512.ComputeHash($stream)) }
    finally { $stream.Dispose(); $sha512.Dispose() }
    if ($actual -ne $pin.Value) {
        throw "$id $version SHA-512 mismatch: expected $($pin.Value), got $actual"
    }
    Write-Host "Verified $id $version"
}

# The SDK resolver and restore both read nuget.config from the project's
# directory; clear every other source so only the verified files are usable.
$config = Join-Path (Split-Path -Parent (Resolve-Path -LiteralPath $ProjectPath)) "nuget.config"
@"
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="wix-pinned" value="$feed" />
  </packageSources>
</configuration>
"@ | Set-Content -LiteralPath $config -Encoding UTF8

# A fresh packages folder, so nothing previously cached on the runner is used.
$env:NUGET_PACKAGES = $packages
if ($env:GITHUB_ENV) { "NUGET_PACKAGES=$packages" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8 }

& dotnet restore $ProjectPath --locked-mode
if ($LASTEXITCODE -ne 0) { throw "dotnet restore failed with exit code $LASTEXITCODE" }
