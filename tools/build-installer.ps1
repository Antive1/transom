# Builds the release binaries and the Inno Setup installer into dist\.
# Requires Inno Setup 6 (winget install JRSoftware.InnoSetup).
#   -SkipBuild        use the binaries already in -BinDir
#   -BinDir <dir>     where the three executables are (default target\release; the release workflow passes the signed ones)
#   -NoZip            only the installer (the workflow zips the signed binaries itself)
param([switch]$SkipBuild, [string]$BinDir = "target\release", [switch]$NoZip)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"').Matches[0].Groups[1].Value
if (-not $SkipBuild) {
    cargo build --release --locked
    if ($LASTEXITCODE) { throw "cargo build failed" }
}
$bin = (Resolve-Path $BinDir).Path
$exes = 'transom-app.exe', 'transom-settings.exe', 'transom-statusline.exe' | ForEach-Object { Join-Path $bin $_ }

$iscc = @(
    (Get-Command iscc.exe -ErrorAction SilentlyContinue).Source,
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 (ISCC.exe) not found" }

New-Item -ItemType Directory -Force dist | Out-Null
& $iscc "/DAppVersion=$version" "/DSourceDir=$bin" installer\transom.iss
if ($LASTEXITCODE) { throw "ISCC failed" }

if (-not $NoZip) {
    $zip = "dist\transom-$version-x64-portable.zip"
    if (Test-Path $zip) { Remove-Item $zip }
    Compress-Archive -Path ($exes + 'LICENSE') -DestinationPath $zip
    Get-ChildItem dist -Exclude SHA256SUMS.txt | ForEach-Object {
        "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
    } | Set-Content dist\SHA256SUMS.txt
    Get-Content dist\SHA256SUMS.txt
}
