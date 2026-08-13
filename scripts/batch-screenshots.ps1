<#
.SYNOPSIS
    Batch-generate screenshots for all .app games in tmp/dingoo_game.

.DESCRIPTION
    Runs the dingoo-emu binary in screenshot mode for every .app file found
    under tmp/dingoo_game recursively. Output PNGs are saved to docs/images
    and named after each game file. When no binary is supplied, the latest
    release binary is built before capture.

.PARAMETER Frames
    Number of frames to emulate before capturing. Default: 60 (one second at
    60 fps). Known slow-starting games use per-game overrides.

.PARAMETER Binary
    Path to the dingoo-emu binary. Default: the Cargo release build output.

.PARAMETER TimeoutSeconds
    Maximum time allowed for each game. Default: 120 seconds.

.PARAMETER ReportDirectory
    Directory for per-game unknown HLE JSON reports. Default: tmp/hle-reports.

.PARAMETER UnknownHlePolicy
    Unknown SDK HLE behavior. Use report for compatibility runs or stop for
    strict validation. Default: report.

.PARAMETER AllowUnknownHle
    Exact unknown SDK function names allowed in strict mode.
#>

param(
    [ValidateRange(1, [int]::MaxValue)]
    [int]$Frames = 60,

    [string]$Binary = "",

    [ValidateRange(1, [int]::MaxValue)]
    [int]$TimeoutSeconds = 120,

    [string]$ReportDirectory = "",

    [ValidateSet("report", "stop")]
    [string]$UnknownHlePolicy = "report",

    [string[]]$AllowUnknownHle = @()
)

$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$framesSpecified = $PSBoundParameters.ContainsKey("Frames")

$repoRoot = Split-Path -Parent $PSScriptRoot
$gameDir = Join-Path $repoRoot "tmp\dingoo_game"
$outDir = Join-Path $repoRoot "docs\images"
if (-not $ReportDirectory) {
    $ReportDirectory = Join-Path $repoRoot "tmp\hle-reports"
}

if (-not (Test-Path -LiteralPath $gameDir -PathType Container)) {
    Write-Error "Game directory not found: $gameDir"
    exit 1
}

New-Item -ItemType Directory -Force -Path $outDir | Out-Null
New-Item -ItemType Directory -Force -Path $ReportDirectory | Out-Null
$ReportDirectory = (Resolve-Path -LiteralPath $ReportDirectory).Path

if (-not $Binary) {
    $Binary = Join-Path $repoRoot "target\release\dingoo-emu.exe"
    Write-Host "Building the latest release binary..." -ForegroundColor Yellow
    try {
        Push-Location $repoRoot
        cargo build --release -p dingooemu
        if ($LASTEXITCODE -ne 0) {
            throw "Release build failed with exit code $LASTEXITCODE."
        }
    } finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    Write-Error "Emulator binary not found: $Binary"
    exit 1
}

$Binary = (Resolve-Path -LiteralPath $Binary).Path

function ConvertTo-ScreenshotName {
    param([string]$BaseName)

    $safeName = $BaseName -replace '\s+', '_'
    $safeName = $safeName -replace '[<>:"/\\|?*\x00-\x1F]', '_'
    $safeName = $safeName.TrimEnd([char[]]@('.', ' '))

    if ([string]::IsNullOrWhiteSpace($safeName)) {
        return "game"
    }

    return $safeName
}

function Get-CaptureFrames {
    param(
        [string]$RelativePath,
        [int]$DefaultFrames,
        [bool]$UsePerformanceOverrides
    )

    if (-not $UsePerformanceOverrides) {
        return $DefaultFrames
    }

    switch ($RelativePath) {
        "7day-20081217192316.app" { return 30 }
        "仙剑奇侠传\仙剑奇侠传.APP" { return 1200 }
        "Decollation-Warrior.app" { return 30 }
        "GooPlayer\GooPlayer.app" { return 300 }
        "Hell Striker II-20090122224048.app" { return 300 }
        "Overlord-Fighter.app" { return 120 }
        "SameGoo\samegoo.app" { return 300 }
        "Snake.app" { return 30 }
        default { return $DefaultFrames }
    }
}

function Invoke-ScreenshotCapture {
    param(
        [string]$Executable,
        [string]$GamePath,
        [string]$ScreenshotPath,
        [string]$ReportPath,
        [int]$CaptureFrames,
        [int]$Timeout,
        [string]$HlePolicy,
        [string[]]$AllowedUnknownHle
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $Executable
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.ArgumentList.Add($GamePath)
    $startInfo.ArgumentList.Add("--screenshot")
    $startInfo.ArgumentList.Add($ScreenshotPath)
    $startInfo.ArgumentList.Add("--screenshot-frames")
    $startInfo.ArgumentList.Add($CaptureFrames.ToString())
    $startInfo.ArgumentList.Add("--unknown-hle-policy")
    $startInfo.ArgumentList.Add($HlePolicy)
    $startInfo.ArgumentList.Add("--hle-report")
    $startInfo.ArgumentList.Add($ReportPath)
    foreach ($name in $AllowedUnknownHle) {
        $startInfo.ArgumentList.Add("--allow-unknown-hle")
        $startInfo.ArgumentList.Add($name)
    }
    $startInfo.Environment["RUST_LOG"] =
        "dingoo_emu=info,dingooemu_core::app_loader=info,dingooemu_core::cpu=off,dingooemu_core::emulator=warn"
    $startInfo.Environment["RUST_LOG_STYLE"] = "never"

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo

    try {
        if (-not $process.Start()) {
            throw "Failed to start emulator process."
        }

        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()

        if (-not $process.WaitForExit($Timeout * 1000)) {
            $process.Kill($true)
            $process.WaitForExit()
            return [pscustomobject]@{
                ExitCode = $null
                TimedOut = $true
                Output = "Timed out after $Timeout seconds."
            }
        }

        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $output = @($stdout.Trim(), $stderr.Trim()) |
            Where-Object { $_ } |
            Join-String -Separator [Environment]::NewLine

        return [pscustomobject]@{
            ExitCode = $process.ExitCode
            TimedOut = $false
            Output = $output
        }
    } finally {
        $process.Dispose()
    }
}

Write-Host "Using binary: $Binary"
Write-Host "Game dir:     $gameDir"
Write-Host "Output dir:   $outDir"
Write-Host "Report dir:   $ReportDirectory"
Write-Host "HLE policy:   $UnknownHlePolicy"
if ($framesSpecified) {
    Write-Host "Frames:       $Frames"
} else {
    Write-Host "Frames:       $Frames (with performance overrides)"
}
Write-Host "Timeout:      $TimeoutSeconds seconds per game"
Write-Host ""

$games = @(
    Get-ChildItem -LiteralPath $gameDir -Filter "*.app" -Recurse -File |
        Sort-Object FullName
)

if ($games.Count -eq 0) {
    Write-Warning "No .app files found under $gameDir"
    exit 0
}

Write-Host "Found $($games.Count) game(s).`n"

$success = 0
$failed = 0
$usedNames = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)

foreach ($game in $games) {
    $baseName = [System.IO.Path]::GetFileNameWithoutExtension($game.Name)
    $relativePath = [System.IO.Path]::GetRelativePath($gameDir, $game.FullName)
    $relativeBaseName = [System.IO.Path]::ChangeExtension($relativePath, $null)
    $relativeName = $relativeBaseName -replace '[/\\]+', '__'
    $safeName = ConvertTo-ScreenshotName $relativeName
    $captureFrames = Get-CaptureFrames $relativePath $Frames (-not $framesSpecified)

    $uniqueName = $safeName
    $suffix = 2
    while (-not $usedNames.Add($uniqueName)) {
        $uniqueName = "${safeName}_$suffix"
        $suffix++
    }

    $safeName = $uniqueName
    $outPath = Join-Path $outDir "$safeName.png"
    $reportPath = Join-Path $ReportDirectory "$safeName.json"

    if (Test-Path -LiteralPath $outPath) {
        Remove-Item -LiteralPath $outPath -Force
    }
    if (Test-Path -LiteralPath $reportPath) {
        Remove-Item -LiteralPath $reportPath -Force
    }

    Write-Host -NoNewline "  $baseName ($captureFrames frames) ... "

    try {
        $result = Invoke-ScreenshotCapture `
            -Executable $Binary `
            -GamePath $game.FullName `
            -ScreenshotPath $outPath `
            -ReportPath $reportPath `
            -CaptureFrames $captureFrames `
            -Timeout $TimeoutSeconds `
            -HlePolicy $UnknownHlePolicy `
            -AllowedUnknownHle $AllowUnknownHle

        if ($result.TimedOut) {
            Write-Host "FAILED (timeout)" -ForegroundColor Red
            $failed++
        } elseif ($result.ExitCode -ne 0) {
            Write-Host "FAILED (exit $($result.ExitCode))" -ForegroundColor Red
            if ($result.Output) {
                $detailLines = @($result.Output -split '\r?\n' |
                    Where-Object { $_ } |
                    Where-Object { $_ -notmatch '^note: run with ' } |
                    Select-Object -Last 2)
                foreach ($line in $detailLines) {
                    Write-Host "    $line" -ForegroundColor DarkGray
                }
            }
            $failed++
        } elseif ((Test-Path -LiteralPath $outPath -PathType Leaf) -and
            (Test-Path -LiteralPath $reportPath -PathType Leaf)) {
            $size = (Get-Item -LiteralPath $outPath).Length
            $reportSize = (Get-Item -LiteralPath $reportPath).Length
            $report = Get-Content -Raw -LiteralPath $reportPath | ConvertFrom-Json
            $hasUnknownHleList = $null -ne $report.PSObject.Properties["unknown_hle"]
            if ($size -gt 0 -and $reportSize -gt 0 -and $hasUnknownHleList) {
                Write-Host "OK ($([math]::Round($size / 1KB)) KB)" -ForegroundColor Green
                $success++
            } else {
                Write-Host "FAILED (empty or invalid output)" -ForegroundColor Red
                $failed++
            }
        } else {
            Write-Host "FAILED (missing screenshot or HLE report)" -ForegroundColor Red
            $failed++
        }
    } catch {
        Write-Host "FAILED ($($_.Exception.Message))" -ForegroundColor Red
        $failed++
    }
}

Write-Host ""
Write-Host "Done: $success succeeded, $failed failed out of $($games.Count) total."

if ($failed -gt 0) {
    exit 1
}
