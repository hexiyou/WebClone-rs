<#
.SYNOPSIS
    （Rust 版）WebClone 引擎一键回归套件。

.DESCRIPTION
    起本地测试服务器（tests\test_server.py），对 target\release\webclone.exe
    跑六组场景，断言退出码与落盘产物：

      R1  single-page clone        -> 页面 + 哈希化 css/js 落盘
      R2  incremental second run   -> 退出 0，manifest 完整
      R3  full-site clone          -> 恰好 3 个页面
      R4  -np boundary             -> 恰好 2 个页面，且未抓到上级页面
      R5  GBK page                 -> 转码为 UTF-8（charset=utf-8）
      R6  --user-agent             -> 服务器侧收到自定义 UA

    脚本内所有输出字符串刻意使用纯 ASCII（规避 Windows PowerShell 5.1 的代码页问题）；
    中文内容的断言全部换成 ASCII 稳定的文件特征。

.PARAMETER CliPath
    命令行 exe 路径。默认取 target\release\webclone.exe。

.PARAMETER PythonPath
    跑测试服务器 / 站点生成器的 Python 可执行文件，默认 "python"。

.PARAMETER Port
    本地测试服务器端口，默认 8765。

.EXAMPLE
    .\regression.ps1
    .\regression.ps1 -CliPath ..\target\release\webclone.exe -PythonPath py
#>
param(
    [string]$CliPath = "",
    [string]$PythonPath = "python",
    [int]$Port = 8765
)

$ErrorActionPreference = "Continue"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path   # tests\
$ProjectRoot = Split-Path -Parent $Root
$OutRoot = Join-Path $Root "_regress"
$SiteDir = Join-Path $Root "site"

# ---- locate CLI ----
if (-not $CliPath) {
    $candidates = @(
        (Join-Path $ProjectRoot "dist\webclone.exe"),
        (Join-Path $ProjectRoot "target\release\webclone.exe")
    )
    foreach ($c in $candidates) {
        if (Test-Path $c) { $CliPath = $c; break }
    }
}
if (-not $CliPath -or -not (Test-Path $CliPath)) {
    Write-Host "FAIL: CLI not found. Build first (build.sh) or pass -CliPath." -ForegroundColor Red
    exit 1
}

# ---- prepare test site ----
if (-not (Test-Path (Join-Path $SiteDir "index.html"))) {
    Write-Host "Generating test site..." -ForegroundColor Yellow
    & $PythonPath (Join-Path $Root "make_test_site.py") $SiteDir
    if ($LASTEXITCODE -ne 0) { Write-Host "FAIL: site generation" -ForegroundColor Red; exit 1 }
}

# ---- start test server ----
Write-Host "Starting test server on 127.0.0.1:$Port ..." -ForegroundColor Yellow
$server = Start-Process -FilePath $PythonPath `
    -ArgumentList @((Join-Path $Root "test_server.py"), "$Port", $SiteDir) `
    -WindowStyle Hidden -PassThru

function Wait-Port {
    param([int]$TargetPort)
    for ($i = 0; $i -lt 30; $i++) {
        try {
            $tcp = New-Object Net.Sockets.TcpClient
            $tcp.Connect("127.0.0.1", $TargetPort)
            $tcp.Close()
            return $true
        } catch {
            Start-Sleep -Milliseconds 500
        }
    }
    return $false
}

$ready = Wait-Port -TargetPort $Port
if (-not $ready) {
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
    Write-Host "FAIL: test server did not come up." -ForegroundColor Red
    exit 1
}

# ---- helpers ----
$results = New-Object System.Collections.Generic.List[string]
$failed = 0

function Invoke-Clone {
    # Start-Process + PassThru 取退出码：绕开 pwsh 7 里
    # "& $exe | Out-Null" 触发 "Cannot run a document in the middle of a pipeline" 的问题
    param([string[]]$CloneArgs)
    $p = Start-Process -FilePath $CliPath -ArgumentList $CloneArgs `
        -WindowStyle Hidden -Wait -PassThru
    return $p.ExitCode
}

function Check {
    param([string]$Name, [bool]$Ok)
    if ($Ok) {
        $results.Add("PASS  $Name")
        Write-Host "PASS  $Name" -ForegroundColor Green
    } else {
        $script:failed++
        $results.Add("FAIL  $Name")
        Write-Host "FAIL  $Name" -ForegroundColor Red
    }
}

function Count-Files {
    param([string]$Dir, [string]$Pattern)
    if (-not (Test-Path $Dir)) { return 0 }
    return @(Get-ChildItem $Dir -Recurse -Filter $Pattern -File).Count
}

$uaServer = $null

try {
    $base = "http://127.0.0.1:$Port"

    # R1: single-page clone
    $out1 = Join-Path $OutRoot "single"
    $code = Invoke-Clone @("$base/index.html", "-o", $out1, "--no-cross-origin")
    $r1dir = Join-Path $out1 "127.0.0.1_$Port"
    $r1ok = ($code -eq 0) `
        -and (Test-Path (Join-Path $r1dir "index.html")) `
        -and ((Count-Files (Join-Path $r1dir "css") "*.css") -ge 3) `
        -and ((Count-Files (Join-Path $r1dir "img") "*.png") -ge 8) `
        -and ((Count-Files (Join-Path $r1dir "font") "*.woff2") -eq 1)
    Check "R1 single-page clone (assets hashed + downloaded)" $r1ok

    # R2: incremental second run
    $code = Invoke-Clone @("$base/index.html", "-o", $out1, "--no-cross-origin")
    $r2ok = ($code -eq 0) -and (Test-Path (Join-Path $out1 ".webclone-manifest.json"))
    Check "R2 incremental rerun (manifest intact, no failures)" $r2ok

    # R3: full-site clone
    $out3 = Join-Path $OutRoot "full"
    $code = Invoke-Clone @("$base/", "-o", $out3, "-m", "full", "-d", "3", "--no-cross-origin")
    $pages3 = Count-Files (Join-Path $out3 "127.0.0.1_$Port") "*.html"
    $r3ok = ($code -eq 0) -and ($pages3 -eq 3)
    Check "R3 full-site clone (3 pages: index + page2 + page3)" $r3ok

    # R4: -np boundary, start from a deep page
    $out4 = Join-Path $OutRoot "np"
    $code = Invoke-Clone @("$base/sub/page2.html", "-o", $out4, "-m", "full", "-d", "2", "--no-cross-origin")
    $npDir = Join-Path $out4 "127.0.0.1_$Port"
    $pages4 = Count-Files $npDir "*.html"
    $r4ok = ($code -eq 0) `
        -and ($pages4 -eq 2) `
        -and (Test-Path (Join-Path $npDir "sub\page2.html")) `
        -and (Test-Path (Join-Path $npDir "sub\deeper\page3.html")) `
        -and (-not (Test-Path (Join-Path $npDir "index.html")))
    Check "R4 -np boundary (no parent traversal)" $r4ok

    # R5: GBK page transcoded to UTF-8
    $out5 = Join-Path $OutRoot "gbk"
    $code = Invoke-Clone @("$base/gbk.html", "-o", $out5, "--no-cross-origin")
    $gbkFile = Join-Path $out5 "127.0.0.1_$Port\gbk.html"
    $r5ok = $false
    if ($code -eq 0 -and (Test-Path $gbkFile)) {
        $head = (Get-Content $gbkFile -TotalCount 8 -Encoding UTF8) -join "`n"
        $r5ok = $head -match "charset=utf-8"
    }
    Check "R5 GBK page transcoded (charset rewritten to utf-8)" $r5ok

    # R6: --user-agent honored (custom UA reaches the server verbatim)
    $uaPort = $Port + 1
    $uaLog = Join-Path $OutRoot "ua_seen.txt"
    $uaServer = Start-Process -FilePath $PythonPath `
        -ArgumentList @((Join-Path $Root "ua_server.py"), "$uaPort", $uaLog) `
        -WindowStyle Hidden -PassThru

    $r6ok = $false
    if (Wait-Port -TargetPort $uaPort) {
        $customUa = "WebClone-Regression/1.0"
        $out6 = Join-Path $OutRoot "ua"
        $code = Invoke-Clone @("http://127.0.0.1:$uaPort/", "-o", $out6, "--user-agent", $customUa)
        Start-Sleep -Milliseconds 300

        if ((Test-Path $uaLog) -and $code -eq 0) {
            $seen = (Get-Content $uaLog -Encoding UTF8 | Where-Object { $_.Trim() -ne "" })
            $r6ok = ($seen -contains $customUa)
        }
    }
    Check "R6 --user-agent honored (server saw custom UA)" $r6ok
}
finally {
    if ($uaServer -and -not $uaServer.HasExited) {
        Stop-Process -Id $uaServer.Id -Force -ErrorAction SilentlyContinue
    }
    if ($server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
}

Write-Host ""
Write-Host "=== Regression summary ===" -ForegroundColor Cyan
foreach ($line in $results) {
    if ($line -like "FAIL*") { Write-Host $line -ForegroundColor Red }
    else { Write-Host $line -ForegroundColor Green }
}

# 防"脚本中途终止导致的假阳性"：必须跑满 6 组才算通过
if ($results.Count -lt 6) {
    Write-Host ""
    Write-Host "RESULT: suite aborted, only $($results.Count)/6 checks ran" -ForegroundColor Red
    exit 1
}

if ($failed -gt 0) {
    Write-Host ""
    Write-Host "RESULT: $failed check(s) FAILED" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "RESULT: all checks passed" -ForegroundColor Green
exit 0
