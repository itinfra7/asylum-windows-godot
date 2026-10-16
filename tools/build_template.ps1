<#
.SYNOPSIS
    Builds the lean, portable Godot 4.7.2 Windows release export template used by Asylum.

.DESCRIPTION
    The official template is ~109 MB and contains 3D, physics, navigation, XR, Vulkan,
    Direct3D 12 and many modules this 2D game never uses. This script builds a 2D-only,
    OpenGL-only template (~36 MB, LTO, optimized for speed) from the 4.7.2-stable source and
    applies tools/godot-patches/*.patch:
      portable-user-data-dir.patch               user:// is <exe folder>/userdata
      borderless-gl-and-per-monitor-dpi.patch    borderless OpenGL windows fill the whole
                                                 monitor; per-monitor DPI awareness

    The source is compiled through a temporary drive letter mapped to build\ (subst), so the
    file names the binary embeds (__FILE__ in error messages) contain no local path.

    Output: tools/godot-template/godot.windows.template_release.x86_64.exe
    (tools/build.ps1 uses it automatically when present).

    Requirements: Git, Python 3 with SCons (pip install scons), Visual Studio Build Tools (MSVC).
    Keep the SCons option deprecated=yes (the default): gdext 0.5.5 calls the deprecated
    GDExtension function get_godot_version. The GDExtension is built with gdext's
    lazy-function-tables feature so it never asks for the classes compiled out here.

.PARAMETER Scons
    SCons command (default: scons on the PATH).

.PARAMETER Jobs
    Parallel compile jobs (default: the number of logical processors).

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\build_template.ps1
#>
[CmdletBinding()]
param(
    [string]$Scons = "scons",
    [int]$Jobs = [Environment]::ProcessorCount
)

$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$Src = Join-Path $Root 'build\godot'
$OutDir = Join-Path $Root 'tools\godot-template'
# Godot 4.7.2-stable, as named in THIRD_PARTY_NOTICES.md
$GodotCommit = 'ed1daf0bf001b61586d9930840f2f1394092c079'

if (-not (Test-Path -LiteralPath $Src)) {
    New-Item -ItemType Directory -Force (Split-Path $Src) | Out-Null
    & git -c core.longpaths=true clone --depth 1 --branch 4.7.2-stable https://github.com/godotengine/godot.git $Src
    if ($LASTEXITCODE -ne 0) { throw 'git clone failed' }
}
$head = (& git -C $Src rev-parse HEAD)
if ($LASTEXITCODE -ne 0 -or "$head".Trim() -ne $GodotCommit) {
    throw "build\godot is at '$head', expected Godot 4.7.2-stable ($GodotCommit)"
}

Push-Location -LiteralPath $Src
try {
    foreach ($p in Get-ChildItem -LiteralPath (Join-Path $Root 'tools\godot-patches') -Filter *.patch) {
        # git reports a failed check on stderr; with ErrorActionPreference=Stop, Windows
        # PowerShell would turn that into a terminating error, so check with Continue.
        $ErrorActionPreference = 'Continue'
        & git apply --check -R $p.FullName 2>&1 | Out-Null
        $already = ($LASTEXITCODE -eq 0)
        $ok = $already
        if (-not $already) {
            & git apply $p.FullName 2>&1 | ForEach-Object { Write-Host $_ }
            $ok = ($LASTEXITCODE -eq 0)
        }
        $ErrorActionPreference = 'Stop'
        if ($already) {
            Write-Host "already applied $($p.Name)"
        } elseif ($ok) {
            Write-Host "applied $($p.Name)"
        } else {
            throw "patch $($p.Name) does not apply"
        }
    }
} finally {
    Pop-Location
}

# A free drive letter for the path-neutral build.
$used = [System.IO.DriveInfo]::GetDrives() | ForEach-Object { $_.Name.Substring(0, 1).ToUpperInvariant() }
$letter = 'ZYXWVUTSRQPONMLKJIHG'.ToCharArray() | Where-Object { $used -notcontains [string]$_ } | Select-Object -First 1
if (-not $letter) { throw 'no free drive letter for subst' }
$Drive = "${letter}:"
& subst $Drive (Split-Path -LiteralPath $Src)
if ($LASTEXITCODE -ne 0) { throw "subst $Drive failed" }
try {
    Push-Location -LiteralPath "$Drive\godot"
    $sconsArgs = @(
        "-j$Jobs", 'progress=no', 'platform=windows', 'target=template_release', 'arch=x86_64',
        'production=yes', 'lto=full', 'optimize=speed', 'debug_symbols=no',
        'vulkan=no', 'd3d12=no', 'angle=no', 'opengl3=yes', 'accesskit=no',
        'disable_3d=yes', 'disable_physics_2d=yes', 'disable_physics_3d=yes',
        'disable_navigation_2d=yes', 'disable_navigation_3d=yes', 'disable_xr=yes',
        'minizip=no', 'brotli=no',
        'modules_enabled_by_default=no', 'module_freetype_enabled=yes', 'module_text_server_fb_enabled=yes'
    )
    & $Scons @sconsArgs
    if ($LASTEXITCODE -ne 0) {
        # MSVC LTCG occasionally fails when both executables link in parallel; retry serially.
        & $Scons ($sconsArgs + '-j1')
        if ($LASTEXITCODE -ne 0) { throw 'scons failed' }
    }
} finally {
    if ((Get-Location).Path -like "$Drive*") { Pop-Location }
    & subst $Drive /d | Out-Null
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
Copy-Item -Force -LiteralPath (Join-Path $Src 'bin\godot.windows.template_release.x86_64.exe') -Destination $OutDir
Get-Item -LiteralPath (Join-Path $OutDir 'godot.windows.template_release.x86_64.exe') | Format-Table Name, Length
