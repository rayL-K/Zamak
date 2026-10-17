# 收集所有归到 ZM999 的报错：bench/ai/cases.json 的反例 + Book 的 fails 示例 + tests/ 里的反例。
# 用法：pwsh -File tools/zm999-scan.ps1 [-Zam <path>]
param([string]$Zam = "zam/target/debug/zam.exe")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$env:ZAMAK_LANG = "en"
$work = Join-Path $env:TEMP ("zam999-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $work | Out-Null
$found = @()

function Check-Source([string]$label, [string]$source) {
    $path = Join-Path $work ("case-" + [guid]::NewGuid().ToString("N") + ".zm")
    Set-Content -Path $path -Value $source -NoNewline
    $json = & $Zam check $path --json 2>$null
    if (-not $json) { return }
    try { $parsed = $json | ConvertFrom-Json } catch { return }
    foreach ($d in $parsed.diagnostics) {
        if ($d.code -eq "ZM999") {
            $script:found += [pscustomobject]@{
                Source  = $label
                Message = $d.message
                Line    = $d.line
                Column  = $d.column
            }
        }
    }
}

# 1. bench/ai/cases.json：每条用例故意写错，期望一个错误码。带 files 的是多模块项目，
#    要按项目检查（entry 是 main.zm）；否则就是单文件。
$cases = Get-Content (Join-Path $root "bench/ai/cases.json") -Raw | ConvertFrom-Json
foreach ($case in $cases) {
    if ($case.files) {
        $dir = Join-Path $work ("proj-" + [guid]::NewGuid().ToString("N"))
        $src = Join-Path $dir "src"
        New-Item -ItemType Directory -Force -Path $src | Out-Null
        $name = $case.id -replace '[^A-Za-z0-9_]', '_'
        Set-Content -Path (Join-Path $dir "zam.toml") -NoNewline `
            -Value "[project]`nname = ""$name""`nentry = ""src/main.zm""`nmodules = ""src""`n"
        foreach ($file in $case.files) {
            $target = Join-Path $src $file.path
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
            Set-Content -Path $target -Value $file.source -NoNewline
        }
        $json = & $Zam check $dir --json 2>$null
        if ($json) {
            try { $parsed = $json | ConvertFrom-Json } catch { continue }
            foreach ($d in $parsed.diagnostics) {
                if ($d.code -eq "ZM999") {
                    $script:found += [pscustomobject]@{
                        Source  = "bench/ai:$($case.id)"
                        Message = $d.message
                        Line    = $d.line
                        Column  = $d.column
                    }
                }
            }
        }
    } else {
        Check-Source "bench/ai:$($case.id)" $case.source
    }
}

# 2. Book 的 ```zamak,fails 代码块。
$chapters = Get-ChildItem -Recurse (Join-Path $root "book") -Include *.md -File
foreach ($chapter in $chapters) {
    $text = Get-Content $chapter.FullName -Raw
    $lines = $text -split "`r?`n"
    $i = 0
    while ($i -lt $lines.Count) {
        if ($lines[$i] -match '^\s*```zamak[^-]*fails') {
            $start = $i + 1
            $block = @()
            $i = $start
            while ($i -lt $lines.Count -and $lines[$i] -notmatch '^\s*```') {
                $block += $lines[$i]
                $i++
            }
            Check-Source "book:$([IO.Path]::GetFileName($chapter.FullName)):$start" ($block -join "`n")
        }
        $i++
    }
}

# 3. examples/：多模块项目里单独一个模块不能当单文件检查（它没有 fn main），所以只在
#    项目里没有 zam.toml 的独立示例上跑；其余由 tests/ 自己按项目检查。
$examples = Get-ChildItem -Recurse (Join-Path $root "examples") -Include *.zm -File |
    Where-Object {
        $dir = $_.Directory
        $standalone = $true
        while ($dir -and $dir.FullName.Length -ge $root.Length) {
            if (Test-Path (Join-Path $dir.FullName "zam.toml")) { $standalone = $false; break }
            $dir = $dir.Parent
        }
        $standalone
    }
foreach ($example in $examples) {
    Check-Source "examples:$($example.FullName.Substring($root.Length + 1))" (Get-Content $example.FullName -Raw)
}

# 4. tests/ 里的反例字面量：`"..."` 中看起来像 Zamak 源码的字符串（含 `fn main(` 或 `fn ` + 冒号块）。
#    测试自己会断言错误码，这里只是把没被断言覆盖的 ZM999 找出来。
$testFiles = Get-ChildItem (Join-Path $root "zam/tests") -Include *.rs -File -Recurse
foreach ($testFile in $testFiles) {
    $lines = Get-Content $testFile.FullName
    $i = 0
    while ($i -lt $lines.Count) {
        # 拼接以 " 开头、跨行的源码字面量（Rust 里写成一行带 \n 的字符串）。
        if ($lines[$i] -match '^\s*"((?:[^"\\]|\\.)*)"\s*,?\s*$' -and $lines[$i] -match '\\n' -and $lines[$i] -match 'fn ') {
            $raw = $Matches[1]
            $source = [System.Text.RegularExpressions.Regex]::Unescape('"' + $raw + '"')
            Check-Source "tests:$($testFile.Name):$($i + 1)" $source
        }
        $i++
    }
}

$found | Group-Object Message | Sort-Object Count -Descending |
    ForEach-Object {
        "{0,3}  {1}" -f $_.Count, $_.Name
        $_.Group | ForEach-Object { "       <- $($_.Source) (line $($_.Line), column $($_.Column))" }
    }
"---"
"total ZM999: $($found.Count), distinct: $(($found | Group-Object Message).Count)"
Remove-Item -Recurse -Force $work
