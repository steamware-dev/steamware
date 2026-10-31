<#
  Sign the Windows release artifacts and record their hashes.

  Tauri will not sign anything on its own, and a certificate cannot live in this repository, so signing is
  a release step that is run rather than a property of the source. `npm run release` builds, signs, and then
  refuses to report success while any artifact is still unsigned.

  Configure exactly one of these before releasing:

    STEAMWARE_SIGN_PFX          path to a .pfx file (also set STEAMWARE_SIGN_PFX_PASSWORD)
    STEAMWARE_SIGN_THUMBPRINT   thumbprint of a code-signing cert in Cert:\CurrentUser\My

  Optional:
    STEAMWARE_TIMESTAMP_URL     RFC 3161 timestamp server, default http://timestamp.digicert.com

  Usage:
    powershell -File scripts/sign-release.ps1            # sign + verify + hashes (warns if unconfigured)
    powershell -File scripts/sign-release.ps1 -Require   # same, but unsigned output is a failure
#>
param(
  [string[]]$Paths,
  [switch]$Require
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$release = Join-Path $repo 'src-tauri\target\release'
$timestamp = if ($env:STEAMWARE_TIMESTAMP_URL) { $env:STEAMWARE_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }

function Find-SignTool {
  $roots = @(
    (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'),
    (Join-Path $env:ProgramFiles 'Windows Kits\10\bin')
  ) | Where-Object { $_ -and (Test-Path $_) }

  foreach ($root in $roots) {
    $candidate = Get-ChildItem $root -Directory |
      Sort-Object Name -Descending |
      ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
      Where-Object { Test-Path $_ } |
      Select-Object -First 1
    if ($candidate) { return $candidate }
  }

  $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
  if ($onPath) { return $onPath.Source }
  return $null
}

function Get-SignArguments {
  if ($env:STEAMWARE_SIGN_PFX) {
    $pfx = $env:STEAMWARE_SIGN_PFX
    if (-not (Test-Path $pfx)) { throw "STEAMWARE_SIGN_PFX points at a file that does not exist: $pfx" }
    $signArgs = @('sign', '/fd', 'sha256', '/f', $pfx)
    if ($env:STEAMWARE_SIGN_PFX_PASSWORD) { $signArgs += @('/p', $env:STEAMWARE_SIGN_PFX_PASSWORD) }
    return $signArgs
  }
  if ($env:STEAMWARE_SIGN_THUMBPRINT) {
    return @('sign', '/fd', 'sha256', '/sha1', $env:STEAMWARE_SIGN_THUMBPRINT, '/s', 'My')
  }
  return $null
}

if (-not $Paths) {
  $Paths = @()
  $app = Join-Path $release 'steamware.exe'
  if (Test-Path $app) { $Paths += $app }
  $installers = @(Get-ChildItem (Join-Path $release 'bundle\nsis') -Filter '*.exe' -ErrorAction SilentlyContinue)
  if ($installers.Count -gt 0) {
    # Only this version's installer: a stale setup left in the bundle folder is not something a release
    # should sign, publish, or hash alongside the new one.
    $version = (Get-Content (Join-Path $repo 'package.json') -Raw | ConvertFrom-Json).version
    $current = @($installers | Where-Object { $_.Name -like "*_${version}_*" })
    if ($current.Count -gt 0) {
      $Paths += $current.FullName
    } else {
      Write-Warning "No setup matching version $version found; signing every installer in bundle\nsis."
      $Paths += $installers.FullName
    }
  }
  # A portable/standalone copy, if one was staged next to the installer output.
  $portable = Join-Path $release 'portable\steamware.exe'
  if (Test-Path $portable) { $Paths += $portable }
}

$Paths = @($Paths | Where-Object { $_ -and (Test-Path $_) })
if ($Paths.Count -eq 0) {
  Write-Error "No artifacts to sign. Build first: npm run release (or npm run tauri build)."
  exit 1
}

function Write-Manifest {
  param([string[]]$Files)

  $unsigned = 0
  $lines = foreach ($path in $Files) {
    $signature = Get-AuthenticodeSignature $path
    $hash = (Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant()
    $state = if ($signature.Status -eq 'Valid') { 'signed  ' } else { 'UNSIGNED'; $unsigned++ }
    Write-Host ("{0}  {1}  {2}" -f $state, $hash, (Split-Path -Leaf $path))
    if ($signature.SignerCertificate) { Write-Host ("           by {0}" -f $signature.SignerCertificate.Subject) }
    "{0}  {1}" -f $hash, (Split-Path -Leaf $path)
  }

  $manifest = Join-Path $release 'SHA256SUMS.txt'
  $lines | Set-Content -Path $manifest -Encoding ascii
  Write-Host "wrote $manifest"
  return $unsigned
}

$signArgs = Get-SignArguments
$signTool = Find-SignTool

if (-not $signArgs -or -not $signTool) {
  $why = if (-not $signArgs) {
    'no certificate configured (set STEAMWARE_SIGN_PFX or STEAMWARE_SIGN_THUMBPRINT)'
  } else {
    'signtool.exe was not found (install the Windows SDK)'
  }
  Write-Warning "Signing skipped: $why."
  foreach ($path in $Paths) { Write-Warning "  UNSIGNED  $path" }
  # The hashes are still worth publishing: they are the only check a user has while the build is unsigned.
  $null = Write-Manifest $Paths
  if ($Require) {
    Write-Error 'Refusing to call this a release: artifacts are unsigned.'
    exit 1
  }
  exit 0
}

foreach ($path in $Paths) {
  Write-Host "signing $path"
  & $signTool @($signArgs + @('/tr', $timestamp, '/td', 'sha256', $path))
  if ($LASTEXITCODE -ne 0) {
    Write-Error "signtool failed for $path (exit $LASTEXITCODE)"
    exit 1
  }
}

$failed = Write-Manifest $Paths
if ($failed -gt 0) {
  Write-Error "$failed artifact(s) are still unsigned."
  exit 1
}
