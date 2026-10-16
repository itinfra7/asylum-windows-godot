<#
.SYNOPSIS
    Builds Asylum: the Rust GDExtension (debug + release), the Godot import and the portable
    Windows export in dist\Asylum\ (optionally zipped for a release).

.DESCRIPTION
    1. cargo build -p asylum-gdext (debug, for the editor) and --release, with the C runtime
       linked statically and local paths remapped out of the binary
    2. copies the DLLs to game\bin\asylum_gdext.debug.dll / asylum_gdext.dll
    3. godot --headless --path game --import
    4. godot --headless --path game --export-release "Windows Desktop" dist\Asylum\Asylum.exe
       (with tools\godot-template\godot.windows.template_release.x86_64.exe when it exists,
       otherwise with the official Godot 4.7.2 export templates)
    5. copies README.md, LICENSE (as LICENSE.txt), THIRD_PARTY_NOTICES.md and licenses\ next to
       the executable
    6. with -Zip: dist\asylum-windows-godot-<version>-windows-x86_64.zip with exactly the
       release files (never userdata\ or override.cfg, which a game run creates)

    Compatible with Windows PowerShell 5.1 and PowerShell 7.

.PARAMETER Godot
    Path of the Godot 4.7.2 console executable
    (default: Godot_v4.7.2-stable_win64.exe\Godot_v4.7.2-stable_win64_console.exe in the
    repository root).

.PARAMETER SkipDebug
    Do not build the debug DLL. The editor loads it, including the editor runs of the import
    and export steps here; use this only when game\bin\asylum_gdext.debug.dll is up to date.

.PARAMETER SkipExport
    Stop after the import step.

.PARAMETER Zip
    Also write the release zip.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\build.ps1 -Zip
#>
[CmdletBinding()]
param(
    [string]$Godot = "",
    [switch]$SkipDebug,
    [switch]$SkipExport,
    [switch]$Zip
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$Root = Split-Path -Parent $PSScriptRoot
$Rust = Join-Path $Root 'rust'
$Game = Join-Path $Root 'game'
$Bin = Join-Path $Game 'bin'
$DistRoot = Join-Path $Root 'dist'
$Dist = Join-Path $DistRoot 'Asylum'
if (-not $Godot) {
    $Godot = Join-Path $Root 'Godot_v4.7.2-stable_win64.exe\Godot_v4.7.2-stable_win64_console.exe'
}
if (-not (Test-Path -LiteralPath $Godot)) {
    throw "Godot executable not found: $Godot (use -Godot <path>)"
}

# Runs a native program; stderr is not treated as a PowerShell error (cargo/godot log there),
# only the exit code decides.
function Invoke-Native {
    param([string]$Exe, [string[]]$Arguments, [string]$What)
    Write-Host "==> $What" -ForegroundColor Cyan
    $old = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $Exe @Arguments
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $old
    }
    if ($code -ne 0) {
        throw "$What failed (exit code $code)"
    }
}

function Copy-Dll {
    param([string]$From, [string]$To, [switch]$MayBeLocked)
    if (-not (Test-Path -LiteralPath $From)) {
        throw "missing build output: $From"
    }
    try {
        Copy-Item -LiteralPath $From -Destination $To -Force
        Write-Host "    $From -> $To"
    } catch {
        if (-not $MayBeLocked) { throw }
        # An open editor keeps the debug DLL loaded; the release export does not need it.
        Write-Warning "could not replace $To (in use?): $($_.Exception.Message)"
    }
}

# --- 1. Rust ---------------------------------------------------------------------------------
# where cargo puts its output (CARGO_TARGET_DIR, CARGO_BUILD_TARGET_DIR or build.target-dir)
$metadata = & cargo metadata --format-version 1 --no-deps --manifest-path (Join-Path $Rust 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$TargetDir = (($metadata -join [char]10) | ConvertFrom-Json).target_directory
$CargoHome = $env:CARGO_HOME
if (-not $CargoHome) {
    $CargoHome = Join-Path $env:USERPROFILE '.cargo'
}
# Static C runtime (as in rust\.cargo\config.toml, which these flags replace) and no local
# paths in panic messages: dependency sources and generated files are named relative to
# neutral prefixes. rustc applies the last matching prefix, so the most specific comes last.
# 0x1F separates the flags, so paths may contain spaces.
$rustFlags = @(
    '-C', 'target-feature=+crt-static',
    "--remap-path-prefix=$Root=/asylum-windows-godot",
    "--remap-path-prefix=$TargetDir=/target",
    "--remap-path-prefix=$(Join-Path $CargoHome 'registry\src')=/cargo/registry/src",
    "--remap-path-prefix=$(Join-Path $CargoHome 'git\checkouts')=/cargo/git/checkouts"
)
$oldFlags = $env:CARGO_ENCODED_RUSTFLAGS
$env:CARGO_ENCODED_RUSTFLAGS = $rustFlags -join [char]0x1F
Push-Location $Rust
try {
    if (-not $SkipDebug) {
        Invoke-Native 'cargo' @('build', '-p', 'asylum-gdext') 'cargo build (debug)'
    }
    Invoke-Native 'cargo' @('build', '-p', 'asylum-gdext', '--release') 'cargo build (release)'
} finally {
    Pop-Location
    $env:CARGO_ENCODED_RUSTFLAGS = $oldFlags
}

# --- 2. DLLs ---------------------------------------------------------------------------------
New-Item -ItemType Directory -Force -Path $Bin | Out-Null
Write-Host '==> copy DLLs to game\bin' -ForegroundColor Cyan
if (-not $SkipDebug) {
    Copy-Dll (Join-Path $TargetDir 'debug\asylum_gdext.dll') (Join-Path $Bin 'asylum_gdext.debug.dll') -MayBeLocked
}
Copy-Dll (Join-Path $TargetDir 'release\asylum_gdext.dll') (Join-Path $Bin 'asylum_gdext.dll')

# --- 3. Godot import ---------------------------------------------------------------------------
# A fresh checkout has no game\.godot yet: the editor would then load the GDExtension in the
# middle of its first scan, and Godot 4.7.2 crashes on exit after such a late load (the import
# itself completes). Listing the extension up front makes the editor load it at start-up, as
# on every later run.
$godotDir = Join-Path $Game '.godot'
$extensionList = Join-Path $godotDir 'extension_list.cfg'
if (-not (Test-Path -LiteralPath $extensionList)) {
    New-Item -ItemType Directory -Force -Path $godotDir | Out-Null
    [System.IO.File]::WriteAllText($extensionList, "res://asylum.gdextension`n", (New-Object System.Text.UTF8Encoding($false)))
}
Invoke-Native $Godot @('--headless', '--path', $Game, '--import') 'godot --import'
if ($SkipExport) {
    return
}

# --- 4. Export -------------------------------------------------------------------------------
# The committed preset leaves custom_template/* empty (the official 4.7.2 templates). When the
# lean 2D/OpenGL template built by tools\build_template.ps1 exists (~36 MB instead of
# ~109 MB), it is written into the preset for this export only, as a path relative to game\
# (Godot's working directory with --path), and the preset is restored afterwards.
$Lean = Join-Path $Root 'tools\godot-template\godot.windows.template_release.x86_64.exe'
$presets = Join-Path $Game 'export_presets.cfg'
$utf8 = New-Object System.Text.UTF8Encoding($false)
$original = [System.IO.File]::ReadAllText($presets)
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
$Exe = Join-Path $Dist 'Asylum.exe'
$Dll = Join-Path $Dist 'asylum_gdext.dll'
foreach ($old in @($Exe, $Dll)) {
    # so that a failed export can never leave the previous build in place
    if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Force }
}
try {
    if (Test-Path -LiteralPath $Lean) {
        $rel = '../tools/godot-template/godot.windows.template_release.x86_64.exe'
        $text = [regex]::Replace($original, 'custom_template/release="[^"]*"', ('custom_template/release="' + $rel + '"'))
        [System.IO.File]::WriteAllText($presets, $text, $utf8)
        Write-Host "==> using lean export template $Lean" -ForegroundColor Cyan
    }
    Invoke-Native $Godot @('--headless', '--path', $Game, '--export-release', 'Windows Desktop', $Exe) 'godot --export-release'
} finally {
    [System.IO.File]::WriteAllText($presets, $original, $utf8)
}
foreach ($out in @($Exe, $Dll)) {
    if (-not (Test-Path -LiteralPath $out)) {
        throw "export did not produce $out"
    }
}

# --- 5. Documents ----------------------------------------------------------------------------
Write-Host '==> copy documents' -ForegroundColor Cyan
$docs = @(
    @('README.md', 'README.md'),
    @('LICENSE', 'LICENSE.txt'),
    @('THIRD_PARTY_NOTICES.md', 'THIRD_PARTY_NOTICES.md')
)
foreach ($d in $docs) {
    $from = Join-Path $Root $d[0]
    if (-not (Test-Path -LiteralPath $from)) {
        throw "missing $from"
    }
    Copy-Item -LiteralPath $from -Destination (Join-Path $Dist $d[1]) -Force
}
$licenses = Join-Path $Dist 'licenses'
if (Test-Path -LiteralPath $licenses) { Remove-Item -LiteralPath $licenses -Recurse -Force }
Copy-Item -LiteralPath (Join-Path $Root 'licenses') -Destination $licenses -Recurse -Force

# --- 6. Release zip ----------------------------------------------------------------------------
$release = @('Asylum.exe', 'asylum_gdext.dll', 'README.md', 'LICENSE.txt', 'THIRD_PARTY_NOTICES.md')
if ($Zip) {
    $version = ([regex]::Match([System.IO.File]::ReadAllText((Join-Path $Rust 'Cargo.toml')), '(?m)^version\s*=\s*"([^"]+)"')).Groups[1].Value
    if (-not $version) { throw 'version not found in rust\Cargo.toml' }
    $zipPath = Join-Path $DistRoot "asylum-windows-godot-$version-windows-x86_64.zip"
    Write-Host "==> zip $zipPath" -ForegroundColor Cyan
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    if (Test-Path -LiteralPath $zipPath) { Remove-Item -LiteralPath $zipPath -Force }
    $files = @($release | ForEach-Object { Join-Path $Dist $_ })
    $files += @(Get-ChildItem -LiteralPath $licenses -Recurse -File | ForEach-Object { $_.FullName })
    $archive = [System.IO.Compression.ZipFile]::Open($zipPath, [System.IO.Compression.ZipArchiveMode]::Create)
    $complete = $false
    try {
        foreach ($f in $files) {
            $entry = 'Asylum/' + $f.Substring($Dist.Length).TrimStart('\').Replace('\', '/')
            [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $f, $entry, [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
        $complete = $true
    } finally {
        $archive.Dispose()
        if (-not $complete) { Remove-Item -LiteralPath $zipPath -Force -ErrorAction SilentlyContinue }
    }
}

Write-Host ''
Write-Host "Portable build: $Dist" -ForegroundColor Green
Get-ChildItem -LiteralPath $Dist -Recurse | ForEach-Object {
    $rel = $_.FullName.Substring($Dist.Length).TrimStart('\')
    if ($_.PSIsContainer) {
        '  {0}\' -f $rel
    } else {
        '  {0,-48} {1,12:N0} bytes' -f $rel, $_.Length
    }
}
if ($Zip) {
    '  {0,-48} {1,12:N0} bytes' -f (Split-Path -Leaf $zipPath), (Get-Item -LiteralPath $zipPath).Length
}
