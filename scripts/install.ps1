# Builds Omniaware and installs it for the current user, with a Start menu shortcut.
#
#   powershell -ExecutionPolicy Bypass -File scripts\install.ps1 [-Startup] [-NoBuild]
#
#   -Startup   also start Omniaware with Windows
#   -NoBuild   install the existing target\release\omniaware.exe as is
#
# Installs to %LOCALAPPDATA%\Programs\Omniaware. Your data in %APPDATA%\Omniaware is not touched.
param([switch]$Startup, [switch]$NoBuild)
$ErrorActionPreference = 'Stop'

# Repo = first folder at or above this script that has a Cargo.toml.
$repo = $PSScriptRoot
while ($repo -and -not (Test-Path (Join-Path $repo 'Cargo.toml'))) { $repo = Split-Path $repo -Parent }
if (-not $repo) { throw "No Cargo.toml at or above $PSScriptRoot" }

$dest = Join-Path $env:LOCALAPPDATA 'Programs\Omniaware'
$exe  = Join-Path $repo 'target\release\omniaware.exe'
$ico  = @((Join-Path $repo 'assets\brand\omniaware.ico')) + @(Get-ChildItem $PSScriptRoot -Filter *.ico | ForEach-Object FullName) |
        Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $NoBuild) {
    Push-Location $repo
    try { cargo build --release; if ($LASTEXITCODE) { throw 'cargo build failed' } }
    finally { Pop-Location }
}
if (-not (Test-Path $exe)) { throw "$exe not found - build first" }

# Single instance: the running copy must exit before the exe can be replaced.
Get-Process omniaware -ErrorAction SilentlyContinue | ForEach-Object { $_ | Stop-Process -Force; $_.WaitForExit(5000) | Out-Null }

New-Item -ItemType Directory -Force $dest | Out-Null
$target = Join-Path $dest 'omniaware.exe'
Copy-Item $exe -Destination $target -Force
$icon = "$target,0"
if ($ico) {
    Copy-Item $ico -Destination (Join-Path $dest 'omniaware.ico') -Force
    $icon = "$(Join-Path $dest 'omniaware.ico'),0"
}

$shell = New-Object -ComObject WScript.Shell
function New-Link([string]$folder) {
    $l = $shell.CreateShortcut((Join-Path $folder 'Omniaware.lnk'))
    $l.TargetPath       = $target
    $l.WorkingDirectory = $dest
    $l.IconLocation     = $icon
    $l.Description      = 'Omniaware - keyboard-first capture and journal'
    $l.Save()
}
New-Link ([Environment]::GetFolderPath('Programs'))
if ($Startup) { New-Link ([Environment]::GetFolderPath('Startup')) }

Start-Process $target
Write-Host "Omniaware installed to $dest" -ForegroundColor Green
