# lean installer — Windows (PowerShell)
# Usage:
#   irm https://raw.githubusercontent.com/naiih001/lean/main/install.ps1 | iex
#   $env:LEAN_VERSION="v0.2.0"; irm ... | iex
param(
  [string]$Version = $env:LEAN_VERSION,
  [string]$InstallDir = $env:LEAN_INSTALL_DIR
)

$Repo = "naiih001/lean"
$BinName = "lean.exe"
if (-not $InstallDir) { $InstallDir = "$env:USERPROFILE\.lean\bin" }

function Resolve-Version {
  param([string]$v)
  if ($v) {
    if (-not $v.StartsWith("v")) { return "v$v" }
    return $v
  }
  try {
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ "User-Agent"="lean-installer" } -TimeoutSec 10
    if ($rel.tag_name) { return $rel.tag_name }
  } catch {}
  # fallback: follow redirect
  try {
    $req = Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" -MaximumRedirection 0 -ErrorAction SilentlyContinue
    $loc = $req.Headers.Location
    if ($loc -match "/tag/(v[^/]+)") { return $Matches[1] }
  } catch {}
  throw "Could not resolve latest version — set `$env:LEAN_VERSION = 'v0.2.0'` and retry."
}

$Version = Resolve-Version $Version
$Asset = "lean-windows-x86_64.zip"
$Url = "https://github.com/$Repo/releases/download/$Version/$Asset"

Write-Host "→ lean $Version (windows-x86_64)" -ForegroundColor Cyan
Write-Host "→ downloading $Url"

$TmpDir = Join-Path $env:TEMP ("lean-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $TmpDir | Out-Null
$ZipPath = Join-Path $TmpDir $Asset

try {
  Invoke-WebRequest -Uri $Url -OutFile $ZipPath -Headers @{ "User-Agent"="lean-installer" }
  # optional sha256
  try {
    $ShaUrl = "$Url.sha256"
    $ShaPath = "$ZipPath.sha256"
    Invoke-WebRequest -Uri $ShaUrl -OutFile $ShaPath -Headers @{ "User-Agent"="lean-installer" } -ErrorAction SilentlyContinue
    if (Test-Path $ShaPath) {
      Write-Host "→ verifying sha256 (if present)"
      # not failing hard if mismatch tool missing
      $expected = (Get-Content $ShaPath).Split(" ")[0]
      $actual = (Get-FileHash $ZipPath -Algorithm SHA256).Hash.ToLower()
      if ($expected.ToLower() -ne $actual) {
        Write-Warning "sha256 mismatch: expected $expected got $actual"
      } else {
        Write-Host "  sha256 ok" -ForegroundColor Green
      }
    }
  } catch {}
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
  } catch {}
  $inPath = $env:Path -split ";" | Where-Object { $_ -eq $InstallDir }
  if (-not $inPath) {
    Write-Host "  note: add to PATH: $InstallDir" -ForegroundColor Yellow
    Write-Host "  run: setx PATH `"%PATH%;$InstallDir`" (new terminals) or `$env:Path += `";$InstallDir`" (current)"
  }
  Write-Host "→ run: lean --help"
  Write-Host "  config: set OPENAI_API_KEY and run lean in your project"
} finally {
  if (Test-Path $TmpDir) { Remove-Item $TmpDir -Recurse -Force -ErrorAction SilentlyContinue }
}
