<#
.SYNOPSIS
    WebClone（Rust 版）一键构建脚本（Windows 侧，对应 C# 版的 build.ps1）。

.DESCRIPTION
    构建 webclone-cli 与 webclone-gui 两个 crate，并把产物汇总到 dist\ 下。

    注意：`bash build.sh` 在部分环境下会被 WSL 的 bash 接走，那会在同一个
    target\ 里编出 Linux 产物，与 Windows 产物互相污染。Windows 上请优先用
    本脚本，或直接在 Git Bash 里执行 `cargo build --release`。

.PARAMETER SkipCli
    只构建图形界面版。

.PARAMETER SkipGui
    只构建命令行版。

.PARAMETER Clean
    先删除 dist\ 与 target\ 再全量构建（冷编译较慢）。

.PARAMETER Profile
    Cargo profile，默认 release（可选 debug）。

.EXAMPLE
    .\build.ps1
    .\build.ps1 -SkipGui
    .\build.ps1 -Clean -Profile debug
#>
param(
    [switch]$SkipCli,
    [switch]$SkipGui,
    [switch]$Clean,
    [string]$Profile = "release"
)

$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Root

$Dist = Join-Path $Root "dist"
$TargetDir = Join-Path $Root "target\$Profile"

# ---- locate cargo ----
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue)
if (-not $cargo) {
    $fallback = Join-Path $env:USERPROFILE ".cargo\bin\cargo.exe"
    if (Test-Path $fallback) {
        $cargo = $fallback
    } else {
        Write-Host "FAIL: cargo not found. Install Rust toolchain first." -ForegroundColor Red
        exit 1
    }
}
$cargoExe = if ($cargo -is [string]) { $cargo } else { $cargo.Source }

if ($Clean) {
    Write-Host "==> Cleaning dist\ and target\" -ForegroundColor Yellow
    if (Test-Path $Dist) { Remove-Item $Dist -Recurse -Force }
    if (Test-Path (Join-Path $Root "target")) { Remove-Item (Join-Path $Root "target") -Recurse -Force }
}

if (-not (Test-Path $Dist)) { New-Item -ItemType Directory -Path $Dist | Out-Null }

function Invoke-Cargo {
    param([string[]]$CargoArgs)
    & $cargoExe @CargoArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL: cargo exited with $LASTEXITCODE" -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

try {
    if (-not $SkipCli) {
        Write-Host "==> Building webclone (CLI)" -ForegroundColor Yellow
        Invoke-Cargo @("build", "-p", "webclone-cli", "--profile", $Profile)
        Copy-Item (Join-Path $TargetDir "webclone.exe") (Join-Path $Dist "webclone.exe") -Force
        Write-Host "    -> dist\webclone.exe"
    }

    if (-not $SkipGui) {
        Write-Host "==> Building WebClone-GUI" -ForegroundColor Yellow
        Invoke-Cargo @("build", "-p", "webclone-gui", "--profile", $Profile)
        Copy-Item (Join-Path $TargetDir "WebClone-GUI.exe") (Join-Path $Dist "WebClone-GUI.exe") -Force
        Write-Host "    -> dist\WebClone-GUI.exe"
    }
}
catch {
    Write-Host "FAIL: $_" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "==> Artifacts:" -ForegroundColor Cyan
Get-ChildItem $Dist | Format-Table Name, @{ Name = "Size"; Expression = { "{0:N2} MB" -f ($_.Length / 1MB) } }
exit 0
