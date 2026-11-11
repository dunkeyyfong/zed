# ZedG fork: build the Windows installer for the fork.
#
# A trimmed-down version of bundle-windows.ps1 that:
#   - installs side by side with upstream Zed (own AppId, name, data dirs),
#   - skips the remote server, the explorer appx package, signing and Sentry,
#   - can build with the quick `ci-test` cargo profile.
#
# Usage: script/bundle-windows-zedg.ps1 [-Profile release|ci-test] [-Architecture x86_64]
# Env:   ZEDG_BUILD_STAMP (UTC yyyyMMddHHmm) is baked into the binary for auto-update.

[CmdletBinding()]
Param(
    [Parameter()][ValidateSet('release', 'ci-test')][string]$Profile = 'release',
    [Parameter()][string]$Architecture = 'x86_64',
    [Parameter()][string]$Repo = 'dunkeyyfong/zed'
)

. "$PSScriptRoot/lib/workspace.ps1"

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

if ($Architecture -ne 'x86_64') {
    throw "ZedG only builds x86_64 for now"
}

$target = "$Architecture-pc-windows-msvc"
$CargoOutDir = "./target/$target/$Profile"

# Enter the MSVC developer shell, wherever Visual Studio is installed.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
Push-Location
& "$vsPath\Common7\Tools\Launch-VsDevShell.ps1" -Arch amd64 -HostArch amd64
Pop-Location

Push-Location -Path crates/zed
$channel = (Get-Content "RELEASE_CHANNEL").Trim()
Pop-Location
if ($channel -ne 'stable') {
    throw "ZedG builds the stable channel, but RELEASE_CHANNEL is '$channel'"
}
$env:ZED_RELEASE_CHANNEL = $channel
$env:RELEASE_CHANNEL = $channel
$env:ZEDG_UPDATE_REPO = $Repo
if ([string]::IsNullOrWhiteSpace($env:ZEDG_BUILD_STAMP)) {
    Write-Host "ZEDG_BUILD_STAMP is not set: this build will never auto-update"
}

ParseZedWorkspace
$innoDir = "$env:ZED_WORKSPACE\inno\$Architecture"

function PrepareForBundle {
    if (Test-Path "$innoDir") {
        Remove-Item -Path "$innoDir" -Recurse -Force
    }
    New-Item -Path "$innoDir" -ItemType Directory -Force | Out-Null
    Copy-Item -Path "$env:ZED_WORKSPACE\crates\zed\resources\windows\*" -Destination "$innoDir" -Recurse -Force
    New-Item -Path "$innoDir\bin" -ItemType Directory -Force | Out-Null
    New-Item -Path "$innoDir\tools" -ItemType Directory -Force | Out-Null
    rustup target add $target
}

function GenerateLicenses {
    if ($Profile -eq 'release') {
        . $PSScriptRoot/generate-licenses.ps1
    } elseif (-not (Test-Path "assets/licenses.md")) {
        # Test builds skip cargo-about to save time.
        Set-Content -Path "assets/licenses.md" -Value "Licenses are listed in full release builds of ZedG."
    }
}

function BuildZedAndItsFriends {
    Write-Output "Building Zed, cli and auto_update_helper ($Profile, stamp '$env:ZEDG_BUILD_STAMP')"
    $cargoArgs = @()
    if ($Profile -eq 'release') {
        # Same size/speed tuning as upstream release bundles.
        $cargoArgs += @('--config', '.cargo/bundle-config.toml')
    }
    $cargoArgs += @('build', '--profile', $Profile, '--package', 'zed', '--package', 'cli', '--package', 'auto_update_helper', '--target', $target)
    cargo @cargoArgs
    Copy-Item -Path "$CargoOutDir\zed.exe" -Destination "$innoDir\Zed.exe" -Force
    Copy-Item -Path "$CargoOutDir\cli.exe" -Destination "$innoDir\bin\zed.exe" -Force
    Copy-Item -Path "$CargoOutDir\auto_update_helper.exe" -Destination "$innoDir\tools\auto_update_helper.exe" -Force
}

function CollectRuntimeFiles {
    # Keep these versions in sync with bundle-windows.ps1.
    Invoke-WebRequest -Uri "https://codeload.github.com/GPUOpen-LibrariesAndSDKs/AGS_SDK/zip/refs/tags/v6.3.0" -OutFile ".\AGS_SDK_v6.3.0.zip"
    Expand-Archive -Path ".\AGS_SDK_v6.3.0.zip" -DestinationPath "." -Force
    $conpty = ".\Microsoft.Windows.Console.ConPTY.1.25.260710002-preview.nupkg"
    Invoke-WebRequest -Uri "https://github.com/microsoft/terminal/releases/download/v1.25.1912.0/Microsoft.Windows.Console.ConPTY.1.25.260710002-preview.nupkg" -OutFile $conpty
    Expand-Archive -Path $conpty -DestinationPath ".\conpty" -Force

    New-Item -Type Directory -Path "$innoDir\x64" -Force | Out-Null
    New-Item -Type Directory -Path "$innoDir\arm64" -Force | Out-Null
    Move-Item -Path ".\AGS_SDK-6.3.0\ags_lib\lib\amd_ags_x64.dll" -Destination "$innoDir\amd_ags_x64.dll" -Force
    Move-Item -Path ".\conpty\build\native\runtimes\x64\OpenConsole.exe" -Destination "$innoDir\x64\OpenConsole.exe" -Force
    Move-Item -Path ".\conpty\build\native\runtimes\arm64\OpenConsole.exe" -Destination "$innoDir\arm64\OpenConsole.exe" -Force
    Move-Item -Path ".\conpty\runtimes\win-x64\native\conpty.dll" -Destination "$innoDir\conpty.dll" -Force
    Move-Item -Path "$innoDir\zed.sh" -Destination "$innoDir\bin\zed" -Force
}

function BuildInstaller {
    $innoSetupPath = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    if (-not (Test-Path $innoSetupPath)) {
        choco install innosetup -y --no-progress
    }

    $setupName = "ZedG-$Architecture-setup"
    $definitions = @{
        "ZedgFork"       = $Repo
        # Own AppId so the fork installs next to upstream Zed, never over it.
        "AppId"          = "{{F9890D55-C080-45D6-BB57-864DBA0884C5}"
        "AppIconName"    = "app-icon"
        "OutputDir"      = "$env:ZED_WORKSPACE\target"
        "AppSetupName"   = $setupName
        "AppName"        = "Zed G"
        "AppDisplayName" = "Zed G"
        "RegValueName"   = "ZedG"
        # Must match crates\zed\src\zed\windows_only_instance.rs (release_channel::app_identifier).
        "AppMutex"       = "ZedG-Editor-Stable-Instance-Mutex"
        "AppExeName"     = "Zed"
        "ResourcesDir"   = "$innoDir"
        "ShellNameShort" = "Zed &G"
        "AppUserId"      = "DunkeyyFong.ZedG"
        "Version"        = "$env:RELEASE_VERSION"
        "SourceDir"      = "$env:ZED_WORKSPACE"
        "AppxFullName"   = "DunkeyyFong.ZedG.NoAppx"
    }

    $defs = @()
    foreach ($key in $definitions.Keys) {
        $defs += "/d$key=`"$($definitions[$key])`""
    }

    Write-Host "Running Inno Setup: $innoSetupPath"
    $process = Start-Process -FilePath $innoSetupPath -ArgumentList (@("$innoDir\zed.iss") + $defs) -NoNewWindow -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "Inno Setup failed: $($process.ExitCode)"
    }

    $setupPath = "target/$setupName.exe"
    Write-Host "Installer: $setupPath"
    if ($env:GITHUB_ENV) {
        Write-Output "SETUP_PATH=$setupPath" >> $env:GITHUB_ENV
    }
}

PrepareForBundle
GenerateLicenses
BuildZedAndItsFriends
CollectRuntimeFiles
BuildInstaller
