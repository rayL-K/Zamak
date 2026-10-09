# Zamak 构建成本基准：测量驱动
# 用法：pwsh bench\measure.ps1 [-Lang rust|go|both] [-Scenarios 1,2,...] [-Reps 3]
# 场景：1 冷构建 2 温构建 3 改一行(叶子) 4 改一行(中间层) 5 改私有(core) 6 改公开签名 7 连续20次改一行 8 4路worktree并行 9 check路径
# 缓存隔离：CARGO_HOME / GOMODCACHE / GOCACHE 全部指向 bench\cache\，不污染用户全局缓存。
param(
  [ValidateSet('rust','go','both')][string]$Lang = 'both',
  [int[]]$Scenarios = @(1,2,3,4,5,6,7,8,9),
  [int]$Reps = 3
)
$ErrorActionPreference = 'Stop'
$bench   = Split-Path -Parent $MyInvocation.MyCommand.Path
$genRoot = Join-Path $bench 'generated\medium-cli'
$rustRoot = Join-Path $genRoot 'rust'
$goRoot   = Join-Path $genRoot 'go'
$cacheRoot = Join-Path $bench 'cache'
$outDir  = 'C:\Zamak\results\raw'
$manifest = Get-Content (Join-Path $genRoot 'manifest.json') -Raw | ConvertFrom-Json

New-Item -ItemType Directory -Force -Path $outDir, $cacheRoot | Out-Null

# ---------- 环境隔离 ----------
$env:CARGO_HOME   = Join-Path $cacheRoot 'cargo-home'
$env:GOMODCACHE   = Join-Path $cacheRoot 'gomodcache'
$env:GOCACHE      = Join-Path $cacheRoot 'gocache'
$env:RUSTC_WRAPPER = ''
$env:CARGO_TERM_COLOR = 'never'

function Get-TreeWorkingSet([int]$rootPid) {
  # 采样进程树的 WorkingSet 总和（字节），用于峰值内存
  $procs = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, WorkingSetSize
  $byParent = @{}
  foreach ($p in $procs) {
    $pid_ = [int]$p.ParentProcessId
    if (-not $byParent.ContainsKey($pid_)) { $byParent[$pid_] = @() }
    $byParent[$pid_] += $p
  }
  $total = [long]0
  $stack = [System.Collections.Stack]::new()
  $stack.Push($rootPid)
  $byPid = @{}
  foreach ($p in $procs) { $byPid[[int]$p.ProcessId] = $p }
  $seen = @{}
  while ($stack.Count -gt 0) {
    $cur = [int]$stack.Pop()
    if ($seen.ContainsKey($cur)) { continue }
    $seen[$cur] = $true
    if ($byPid.ContainsKey($cur)) { $total += [long]$byPid[$cur].WorkingSetSize }
    if ($byParent.ContainsKey($cur)) {
      foreach ($child in $byParent[$cur]) { $stack.Push([int]$child.ProcessId) }
    }
  }
  return $total
}

function Invoke-Sampled([string]$FilePath, [string[]]$ArgList, [string]$WorkDir) {
  # 启动进程，轮询采样进程树 WorkingSet，返回 wall + peakMB
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $FilePath -ArgumentList $ArgList -WorkingDirectory $WorkDir -PassThru -NoNewWindow -RedirectStandardOutput (Join-Path $env:TEMP "zamak-bench-stdout.tmp") -RedirectStandardError (Join-Path $env:TEMP "zamak-bench-stderr.tmp")
  $peak = [long]0
  while (-not $p.HasExited) {
    $ws = Get-TreeWorkingSet -rootPid $p.Id
    if ($ws -gt $peak) { $peak = $ws }
    Start-Sleep -Milliseconds 80
  }
  $p.WaitForExit()
  $sw.Stop()
  $errTail = ''
  $errFile = Join-Path $env:TEMP 'zamak-bench-stderr.tmp'
  if ((Test-Path $errFile) -and ((Get-Item $errFile).Length -gt 0)) {
    $errTail = (Get-Content $errFile -Tail 3) -join ' | '
  }
  return [pscustomobject]@{
    wallSec  = [math]::Round($sw.Elapsed.TotalSeconds, 3)
    peakMB   = [math]::Round($peak / 1MB, 1)
    exitCode = $p.ExitCode
    errTail  = $errTail
  }
}

function Get-DirSizeMB([string]$Path) {
  if (-not (Test-Path $Path)) { return 0 }
  $sum = (Get-ChildItem $Path -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
  return [math]::Round(($sum ?? 0) / 1MB, 2)
}

function Write-Result($obj) {
  $line = $obj | ConvertTo-Json -Compress -Depth 6
  Add-Content -Path (Join-Path $outDir "runs.jsonl") -Value $line
  Write-Host ("[{0} {1} rep{2}] wall={3}s peak={4}MB {5}" -f $obj.lang, $obj.scenario, $obj.rep, $obj.wallSec, $obj.peakMB, $(if($obj.errTail){"ERR: "+$obj.errTail}else{""}))
}

# ---------- 编辑锚点操作 ----------
function Set-Tweak([string]$LangName, [string]$RelPath, [int]$Value) {
  $root = if ($LangName -eq 'rust') { $rustRoot } else { $goRoot }
  $path = Join-Path $root $RelPath
  $src = Get-Content $path -Raw
  if ($LangName -eq 'rust') {
    $src = $src -replace 'const TWEAK: u64 = \d+;', "const TWEAK: u64 = $Value;"
  } else {
    $src = $src -replace 'const tweakm1 uint64 = \d+', "const tweakm1 uint64 = $Value"
  }
  Set-Content -Path $path -Value $src -NoNewline
}

function Set-SigAnchor([string]$LangName, [bool]$Changed) {
  $root = if ($LangName -eq 'rust') { $rustRoot } else { $goRoot }
  $e = $manifest.edits.sigAnchor
  $path = Join-Path $root ($e."$LangName")
  $src = Get-Content $path -Raw
  if ($LangName -eq 'rust') {
    if ($Changed) { $src = $src.Replace($e.fromRust, $e.toRust) } else { $src = $src.Replace($e.toRust, $e.fromRust) }
  } else {
    if ($Changed) { $src = $src.Replace($e.fromGo, $e.toGo) } else { $src = $src.Replace($e.toGo, $e.fromGo) }
  }
  Set-Content -Path $path -Value $src -NoNewline
  foreach ($c in $manifest.edits.sigCallers) {
    $cp = Join-Path $root ($c."$LangName")
    $cs = Get-Content $cp -Raw
    if ($LangName -eq 'rust') {
      if ($Changed) { $cs = $cs.Replace($c.fromRust, $c.toRust) } else { $cs = $cs.Replace($c.toRust, $c.fromRust) }
    } else {
      if ($Changed) { $cs = $cs.Replace($c.fromGo, $c.toGo) } else { $cs = $cs.Replace($c.toGo, $c.fromGo) }
    }
    Set-Content -Path $cp -Value $cs -NoNewline
  }
}

# ---------- 各语言的构建命令 ----------
function Build-Lang([string]$L, [string]$Mode = 'build') {
  # Mode: build | check
  if ($L -eq 'rust') {
    if ($Mode -eq 'check') { return Invoke-Sampled -FilePath 'cargo' -ArgList @('check','--workspace') -WorkDir $rustRoot }
    return Invoke-Sampled -FilePath 'cargo' -ArgList @('build','--workspace') -WorkDir $rustRoot
  } else {
    if ($Mode -eq 'check') { return Invoke-Sampled -FilePath 'go' -ArgList @('vet','./...') -WorkDir $goRoot }
    return Invoke-Sampled -FilePath 'go' -ArgList @('build','./...') -WorkDir $goRoot
  }
}

function Reset-Cold([string]$L) {
  if ($L -eq 'rust') {
    if (Test-Path (Join-Path $rustRoot 'target')) { Remove-Item (Join-Path $rustRoot 'target') -Recurse -Force }
    if (Test-Path $env:CARGO_HOME) { Remove-Item $env:CARGO_HOME -Recurse -Force }
  } else {
    if (Test-Path $env:GOCACHE) { Remove-Item $env:GOCACHE -Recurse -Force }
    if (Test-Path $env:GOMODCACHE) { Remove-Item $env:GOMODCACHE -Recurse -Force }
    Get-ChildItem $goRoot -Filter 'zambench*' -File -ErrorAction SilentlyContinue | Where-Object { $_.Extension -eq '.exe' } | Remove-Item -Force
    if (Test-Path (Join-Path $goRoot 'medium-cli.exe')) { Remove-Item (Join-Path $goRoot 'medium-cli.exe') -Force }
  }
}

function Reset-Warm([string]$L) {
  # 只清构建产物，保留依赖缓存（= "从零 rebuild"）
  if ($L -eq 'rust') {
    if (Test-Path (Join-Path $rustRoot 'target')) { Remove-Item (Join-Path $rustRoot 'target') -Recurse -Force }
  } else {
    if (Test-Path $env:GOCACHE) { Remove-Item $env:GOCACHE -Recurse -Force }
    Get-ChildItem $goRoot -Filter '*.exe' -File -ErrorAction SilentlyContinue | Remove-Item -Force
  }
}

# ---------- 场景 ----------
function Run-Scenario([string]$L, [int]$Sc, [int]$Rep) {
  $base = @{ lang=$L; scenario=$Sc; rep=$Rep; at=(Get-Date).ToString('s') }
  switch ($Sc) {
    1 { # 冷构建：缓存全清
      Reset-Cold $L
      $r = Build-Lang $L
      $base += @{ wallSec=$r.wallSec; peakMB=$r.peakMB; exitCode=$r.exitCode; errTail=$r.errTail }
    }
    2 { # 温构建：只清产物，保留依赖缓存
      Reset-Warm $L
      $r = Build-Lang $L
      $base += @{ wallSec=$r.wallSec; peakMB=$r.peakMB; exitCode=$r.exitCode; errTail=$r.errTail }
    }
    3 { # 改一行（叶子 cli/m1）
      $rel = $manifest.edits.leaf."$L"
      Set-Tweak $L $rel (Get-Random -Maximum 99999)
      $r = Build-Lang $L
      $base += @{ wallSec=$r.wallSec; peakMB=$r.peakMB; exitCode=$r.exitCode; errTail=$r.errTail; edit=$rel }
    }
    4 { # 改一行（中间层 store/m1）
      $rel = $manifest.edits.middle."$L"
      Set-Tweak $L $rel (Get-Random -Maximum 99999)
      $r = Build-Lang $L
      $base += @{ wallSec=$r.wallSec; peakMB=$r.peakMB; exitCode=$r.exitCode; errTail=$r.errTail; edit=$rel }
    }
    5 { # 改 core 私有（core/m1 的 TWEAK，pub(crate)/包内）
      $rel = $manifest.edits.private."$L"
      Set-Tweak $L $rel (Get-Random -Maximum 99999)
      $r = Build-Lang $L
      $base += @{ wallSec=$r.wallSec; peakMB=$r.peakMB; exitCode=$r.exitCode; errTail=$r.errTail; edit=$rel }
    }
    6 { # 改 core 公开签名（含所有调用点）
      Set-SigAnchor $L $true
      $r = Build-Lang $L
      Set-SigAnchor $L $false
      $base += @{ wallSec=$r.wallSec; peakMB=$r.peakMB; exitCode=$r.exitCode; errTail=$r.errTail; edit='core sig_anchor + callers' }
    }
    7 { # 连续 20 次改一行（叶子），记录每次
      $rel = $manifest.edits.leaf."$L"
      $times = @()
      for ($i = 1; $i -le 20; $i++) {
        Set-Tweak $L $rel $i
        $r = Build-Lang $L
        $times += $r.wallSec
      }
      $base += @{ walls=$times; sumSec=[math]::Round(($times | Measure-Object -Sum).Sum,3); medianSec=(($times | Sort-Object)[10]) }
    }
    8 { # 4 路 worktree 并行（共享缓存，4 份独立 checkout）
      $wtRoot = Join-Path $cacheRoot 'worktrees'
      if (Test-Path $wtRoot) { Remove-Item $wtRoot -Recurse -Force }
      New-Item -ItemType Directory -Force -Path $wtRoot | Out-Null
      $srcRoot = if ($L -eq 'rust') { $rustRoot } else { $goRoot }
      $procs = @()
      $w = [System.Diagnostics.Stopwatch]::StartNew()
      for ($i = 1; $i -le 4; $i++) {
        $dst = Join-Path $wtRoot "wt$i"
        Copy-Item $srcRoot $dst -Recurse -Force -Exclude @('target','*.exe')
        if ($L -eq 'rust') {
          $procs += Start-Process -FilePath 'cargo' -ArgumentList @('build','--workspace') -WorkingDirectory $dst -PassThru -NoNewWindow
        } else {
          $procs += Start-Process -FilePath 'go' -ArgumentList @('build','./...') -WorkingDirectory $dst -PassThru -NoNewWindow
        }
      }
      $peak = [long]0
      while ($procs | Where-Object { -not $_.HasExited }) {
        $ws = ($procs | ForEach-Object { Get-TreeWorkingSet -rootPid $_.Id } | Measure-Object -Sum).Sum
        if ($ws -gt $peak) { $peak = $ws }
        Start-Sleep -Milliseconds 120
      }
      $w.Stop()
      $base += @{ wallSec=[math]::Round($w.Elapsed.TotalSeconds,3); peakMB=[math]::Round($peak/1MB,1); parallel=4 }
    }
    9 { # check 路径：改一行后 cargo check / go vet
      $rel = $manifest.edits.leaf."$L"
      Set-Tweak $L $rel (Get-Random -Maximum 99999)
      $r = Build-Lang $L -Mode 'check'
      $base += @{ wallSec=$r.wallSec; peakMB=$r.peakMB; exitCode=$r.exitCode; errTail=$r.errTail; edit=$rel }
    }
  }
  # 产物体积（每场景记一次，用于对比）
  if ($L -eq 'rust') {
    $base += @{ targetMB = Get-DirSizeMB (Join-Path $rustRoot 'target') }
  } else {
    $base += @{ gocacheMB = Get-DirSizeMB $env:GOCACHE }
  }
  Write-Result ([pscustomobject]$base)
}

# ---------- 主流程 ----------
$langs = if ($Lang -eq 'both') { @('rust','go') } else { @($Lang) }
foreach ($L in $langs) {
  foreach ($Sc in $Scenarios) {
    for ($Rep = 1; $Rep -le $Reps; $Rep++) {
      Run-Scenario $L $Sc $Rep
    }
  }
}
Write-Host "done. results in $outDir\runs.jsonl"
