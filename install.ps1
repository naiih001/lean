# lean installer — Windows (PowerShell 5.1+ / 7+)
# Usage:
#   irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
#   $env:LEAN_VERSION="v0.6.1"; irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
# NOTE: if execution policy blocks `iex`, run this first (current process only):
#   Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
param(
  [string]$Version = $env:LEAN_VERSION,
  [string]$InstallDir = $env:LEAN_INSTALL_DIR,
  [switch]$Help
)

if ($Help) {
  @"
lean installer (windows)

Env:
  LEAN_VERSION      tag to install, e.g. v0.6.1 (default: latest release)
  LEAN_INSTALL_DIR  directory to install to (default: %USERPROFILE%\.lean\bin)

Params:
  -Version VER      override version
  -InstallDir DIR   override install dir
  -Help             show this help

Examples:
  irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
  `$env:LEAN_VERSION="v0.6.1"; irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
"@
  exit 0
}

# TLS 1.2+ (Windows PowerShell 5.1 defaults to TLS 1.0, which github.com rejects)
try { [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor [Net.SecurityProtocolType]::Tls13 } catch {}

# -UseBasicParsing only exists on Windows PowerShell 5.1 (removed in 6+)
$ExtraWebArgs = @{}
if ($PSVersionTable.PSVersion.Major -le 5) { $ExtraWebArgs['UseBasicParsing'] = $true }

$Repo = "naiih001/lean"
$BinName = "lean.exe"
if (-not $InstallDir) { $InstallDir = "$env:USERPROFILE\.lean\bin" }

function Resolve-Version {
  param([string]$v)
  $v = ($v | Out-String).Trim()
  if ($v) {
    if (-not $v.StartsWith("v")) { return "v$v" }
    return $v
  }
  $Headers = @{ "User-Agent" = "lean-installer" }
  # 1. Try GitHub API /releases/latest
  try {
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers $Headers -TimeoutSec 10 @ExtraWebArgs
    if ($rel.tag_name) { return $rel.tag_name }
  } catch {}
  # 2. Fallback: read the redirect target of /releases/latest.
  #    WinPS 5.1 throws on 302 with -MaximumRedirection 0 (location lives on the
  #    exception response); PS 7+ returns a response object instead — handle both.
  try {
    $loc = $null
    try {
      $resp = Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" -MaximumRedirection 0 -Headers $Headers -TimeoutSec 10 @ExtraWebArgs -ErrorAction Stop
      $loc = $resp.Headers.Location
    } catch {
      $loc = $_.Exception.Response.Headers.Location
      if (-not $loc -and $_.Exception.Response.ResponseUri) { $loc = $_.Exception.Response.ResponseUri.ToString() }
    }
    if ($loc -match "/tag/(v[^/]+)") { return $Matches[1] }
  } catch {}
  # 3. Fallback: GitHub API /tags (latest tag)
  try {
    $tags = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/tags?per_page=1" -Headers $Headers -TimeoutSec 10 @ExtraWebArgs
    if ($tags -and $tags.Count -gt 0 -and $tags[0].name) { return $tags[0].name }
  } catch {}
  throw "Could not resolve latest version from GitHub. No releases or tags found for $Repo. Set `$env:LEAN_VERSION = 'v0.6.1' or use -Version v0.6.1 and retry."
}

$Version = Resolve-Version $Version
$Asset = "lean-windows-x86_64.zip"
$Url = "https://github.com/$Repo/releases/download/$Version/$Asset"

Write-Host "→ lean $Version (windows-x86_64)" -ForegroundColor Cyan
Write-Host "→ downloading $Url"

$TempBase = if ($env:TEMP) { $env:TEMP } else { [IO.Path]::GetTempPath() }
$TmpDir = Join-Path $TempBase ("lean-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $TmpDir | Out-Null
$ZipPath = Join-Path $TmpDir $Asset

try {
  try {
    Invoke-WebRequest -Uri $Url -OutFile $ZipPath -Headers @{ "User-Agent" = "lean-installer" } @ExtraWebArgs -ErrorAction Stop
  } catch {
    $msg = $_.Exception.Message
    if ($msg -match "404") {
      throw "Download failed (404): $Url — check the tag exists and the asset was published: https://github.com/$Repo/releases/tag/$Version"
    }
    throw "Download failed: $Url ($msg) — proxies/firewalls may block github.com; retry with -Verbose for detail."
  }
  # checksum: fail hard on mismatch, skip with a note when unpublished
  try {
    $ShaUrl = "$Url.sha256"
    $ShaPath = "$ZipPath.sha256"
    Invoke-WebRequest -Uri $ShaUrl -OutFile $ShaPath -Headers @{ "User-Agent" = "lean-installer" } @ExtraWebArgs -ErrorAction Stop
    if (Test-Path $ShaPath) {
      Write-Host "→ verifying sha256"
      $expected = ((Get-Content $ShaPath -Raw) -split '\s+')[0].Trim().ToLower()
      $actual = (Get-FileHash $ZipPath -Algorithm SHA256).Hash.ToLower()
      if ($expected -ne $actual) {
        throw "sha256 mismatch for $Asset — expected $expected, got $actual. The download may be corrupt; delete $TmpDir and retry."
      }
      Write-Host "  sha256 ok" -ForegroundColor Green
    }
  } catch {
    if ($_.Exception.Message -match "sha256 mismatch") { throw }
    Write-Warning "no .sha256 published for $Version — skipping verification"
  }
  Write-Host "→ extracting to $TmpDir"
  Expand-Archive -Path $ZipPath -DestinationPath $TmpDir -Force
  $BinSrc = Join-Path $TmpDir $BinName
  if (-not (Test-Path $BinSrc)) {
    $BinSrc = (Get-ChildItem -Path $TmpDir -Recurse -Filter $BinName | Select-Object -First 1).FullName
  }
  if (-not $BinSrc -or -not (Test-Path $BinSrc)) { throw "Archive missing $BinName" }

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Copy-Item $BinSrc (Join-Path $InstallDir $BinName) -Force
  Write-Host "✓ installed $InstallDir\$BinName" -ForegroundColor Green
  try {
    $verOut = & (Join-Path $InstallDir $BinName) --help 2>&1 | Select-Object -First 1
    Write-Host "  $verOut"
  } catch {
    Write-Warning "binary sanity check failed — run '$InstallDir\$BinName --help' for detail"
  }
  $inPath = $env:Path -split ";" | Where-Object { $_ -eq $InstallDir }
  if (-not $inPath) {
    Write-Host "  note: $InstallDir is not on PATH." -ForegroundColor Yellow
    Write-Host "  current shell only: `$env:Path += `";$InstallDir`""
    Write-Host "  permanent: add $InstallDir via Settings > System > About > Advanced system settings > Environment Variables (new terminals pick it up)"
  }
  Write-Host "→ run: lean --help"
  Write-Host "  config: set OPENAI_API_KEY and run lean in your project"
} finally {
  if (Test-Path $TmpDir) { Remove-Item $TmpDir -Recurse -Force -ErrorAction SilentlyContinue }
}
