# Builds a signed Pane release: bumps the version, builds the installer + updater signature, writes latest.json
# and prints the `gh release create` command. Does not publish anything itself.
# Usage: tools/release.ps1 -Version 0.7.0 [-Notes "What changed"]
param(
    [Parameter(Mandatory)][string]$Version,
    [string]$Notes = "Pane $Version"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot
$key = Join-Path $env:USERPROFILE ".tauri\pane-updater.key"
if (-not (Test-Path $key)) { throw "Updater key missing: $key (see CLAUDE.md)" }

# Version in package.json, Cargo.toml and tauri.conf.json (first "version" line in each).
function Set-Version($file, $pattern, $line) {
    $path = Join-Path $root $file
    $text = [IO.File]::ReadAllText($path)
    $text = ([regex]$pattern).Replace($text, $line, 1)
    [IO.File]::WriteAllText($path, $text)
}
Set-Version "package.json" '"version": "[^"]*"' "`"version`": `"$Version`""
Set-Version "src-tauri\tauri.conf.json" '"version": "[^"]*"' "`"version`": `"$Version`""
Set-Version "src-tauri\Cargo.toml" '(?m)^version = "[^"]*"' "version = `"$Version`""

$env:TAURI_SIGNING_PRIVATE_KEY = $key
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
Push-Location $root
try { npx tauri build; if ($LASTEXITCODE) { throw "tauri build failed" } } finally { Pop-Location }

$nsis = Join-Path $root "src-tauri\target\release\bundle\nsis"
$setup = Join-Path $nsis "Pane_${Version}_x64-setup.exe"
$sig = "$setup.sig"
if (-not (Test-Path $sig)) { throw "No signature: $sig" }

$latest = [ordered]@{
    version   = $Version
    notes     = $Notes
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = @{
        "windows-x86_64" = [ordered]@{
            signature = (Get-Content $sig -Raw).Trim()
            url       = "https://github.com/LouisHD14/pane/releases/download/v$Version/Pane_${Version}_x64-setup.exe"
        }
    }
}
$json = Join-Path $nsis "latest.json"
[IO.File]::WriteAllText($json, ($latest | ConvertTo-Json -Depth 4))

Write-Host "`nBuilt $setup`nCommit the version bump, then publish (not as prerelease):`n"
Write-Host "gh release create v$Version `"$setup`" `"$sig`" `"$json`" --repo LouisHD14/pane --target main --title `"Pane $Version`" --notes `"$Notes`""
