<#
.SYNOPSIS
    Batch-generate screenshots for all .app games in tmp/dingoo_game.

.DESCRIPTION
    Runs the dingoo-emu binary in screenshot mode for every .app file found
    under tmp/dingoo_game recursively. Output PNGs are saved to docs/images
    and named after each game file. Per-game diagnostics plus summary.json and
    summary.csv are written to the report directory. L0 requires a valid load
    report; L1 additionally requires a successful non-solid framebuffer
    capture. When no binary is supplied, the latest release binary is built
    before capture.

.PARAMETER Frames
    Number of frames to emulate before capturing. Default: 60 (one second at
    60 fps). Known slow-starting games use per-game overrides.

.PARAMETER Binary
    Path to the dingoo-emu binary. Default: the Cargo release build output.

.PARAMETER TimeoutSeconds
    Maximum time allowed for each game. Default: 120 seconds.

.PARAMETER ReportDirectory
    Directory for per-game diagnostics and unified JSON/CSV summaries.
    Default: tmp/hle-reports.

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

function ConvertTo-ForwardSlashPath {
    param([string]$Path)

    return $Path.Replace('\', '/')
}

function Get-Sha256 {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return $null
    }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-LogTail {
    param(
        [AllowEmptyString()]
        [string]$Output,
        [int]$Count = 10
    )

    if ([string]::IsNullOrWhiteSpace($Output)) {
        return @()
    }
    return @(
        $Output -split '\r?\n' |
            Where-Object { $_ } |
            Where-Object { $_ -notmatch '^note: run with ' } |
            Select-Object -Last $Count
    )
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
        "7day.app" { return 30 }
        "仙剑奇侠传\仙剑奇侠传.APP" { return 1200 }
        "Decollation-Warrior.app" { return 30 }
        "GooPlayer\GooPlayer.app" { return 300 }
        "Overlord-Fighter.app" { return 30 }
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

$gitCommit = (& git -C $repoRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) {
    throw "Unable to resolve the current Git commit."
}
$gitDirty = @(& git -C $repoRoot status --porcelain --untracked-files=no).Count -gt 0
$binarySha256 = Get-Sha256 $Binary
$summaryJsonPath = Join-Path $ReportDirectory "summary.json"
$summaryCsvPath = Join-Path $ReportDirectory "summary.csv"
$runStartedAt = [DateTimeOffset]::UtcNow

foreach ($summaryPath in @($summaryJsonPath, $summaryCsvPath)) {
    if (Test-Path -LiteralPath $summaryPath) {
        Remove-Item -LiteralPath $summaryPath -Force
    }
}

Write-Host "Using binary: $Binary"
Write-Host "Game dir:     $gameDir"
Write-Host "Output dir:   $outDir"
Write-Host "Report dir:   $ReportDirectory"
Write-Host "Git commit:   $gitCommit$(if ($gitDirty) { ' (dirty)' })"
Write-Host "HLE policy:   $UnknownHlePolicy"
if ($framesSpecified) {
    Write-Host "Frames:       $Frames"
} else {
    Write-Host "Frames:       $Frames (with performance overrides)"
}
Write-Host "Timeout:      $TimeoutSeconds seconds per game"
Write-Host "Levels:       L0=load report, L1=completed non-solid frame"
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

$records = [System.Collections.Generic.List[object]]::new()
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
    $capturePath = Join-Path $ReportDirectory "$safeName.png"
    $reportPath = Join-Path $ReportDirectory "$safeName.json"

    if (Test-Path -LiteralPath $capturePath) {
        Remove-Item -LiteralPath $capturePath -Force
    }
    if (Test-Path -LiteralPath $reportPath) {
        Remove-Item -LiteralPath $reportPath -Force
    }

    Write-Host -NoNewline "  $baseName ($captureFrames frames) ... "

    $timer = [System.Diagnostics.Stopwatch]::StartNew()
    $launchError = $null
    try {
        $result = Invoke-ScreenshotCapture `
            -Executable $Binary `
            -GamePath $game.FullName `
            -ScreenshotPath $capturePath `
            -ReportPath $reportPath `
            -CaptureFrames $captureFrames `
            -Timeout $TimeoutSeconds `
            -HlePolicy $UnknownHlePolicy `
            -AllowedUnknownHle $AllowUnknownHle
    } catch {
        $launchError = $_.Exception.Message
        $result = [pscustomobject]@{
            ExitCode = $null
            TimedOut = $false
            Output = $launchError
        }
    } finally {
        $timer.Stop()
    }

    $diagnostics = $null
    $diagnosticsError = $null
    if (Test-Path -LiteralPath $reportPath -PathType Leaf) {
        try {
            $diagnostics = Get-Content -Raw -LiteralPath $reportPath | ConvertFrom-Json
            foreach ($property in @("schema_version", "content", "run", "framebuffer", "unknown_hle")) {
                if ($null -eq $diagnostics.PSObject.Properties[$property]) {
                    throw "Missing diagnostics property: $property"
                }
            }
            if ($diagnostics.content -ne $game.Name) {
                throw "Diagnostics content does not match the game file."
            }
            if ($diagnostics.run.mode -ne "screenshot" -or
                [int]$diagnostics.run.requested_frames -ne $captureFrames) {
                throw "Diagnostics run configuration does not match the capture."
            }
        } catch {
            $diagnosticsError = $_.Exception.Message
            $diagnostics = $null
        }
    }

    $screenshotExists = Test-Path -LiteralPath $capturePath -PathType Leaf
    $screenshotSize = if ($screenshotExists) {
        (Get-Item -LiteralPath $capturePath).Length
    } else {
        0
    }
    $reportExists = Test-Path -LiteralPath $reportPath -PathType Leaf
    $reportSize = if ($reportExists) {
        (Get-Item -LiteralPath $reportPath).Length
    } else {
        0
    }

    $l0Passed = $null -ne $diagnostics
    $l0Reason = if ($l0Passed) {
        "content_loaded"
    } elseif ($result.TimedOut) {
        "timeout_before_valid_report"
    } elseif ($reportExists) {
        "invalid_diagnostics_report"
    } else {
        "content_load_or_report_failed"
    }

    $framebuffer = if ($null -ne $diagnostics) {
        $diagnostics.framebuffer
    } else {
        $null
    }
    $l1Passed = $false
    $l1Reason = if (-not $l0Passed) {
        "l0_failed"
    } elseif ($result.TimedOut) {
        "timeout"
    } elseif ($null -ne $result.ExitCode -and $result.ExitCode -ne 0) {
        "runtime_error"
    } elseif ([uint64]$diagnostics.run.executed_frames -ne [uint64]$captureFrames) {
        "incomplete_frame_count"
    } elseif (-not $screenshotExists -or $screenshotSize -le 0) {
        "missing_screenshot"
    } elseif ([uint64]$framebuffer.non_black_pixels -eq 0) {
        "black_framebuffer"
    } elseif ([uint64]$framebuffer.unique_colors -le 1) {
        "solid_framebuffer"
    } else {
        $l1Passed = $true
        "non_solid_framebuffer"
    }

    $screenshotSha256 = Get-Sha256 $capturePath
    $screenshotArtifactPath = if ($screenshotExists) {
        ConvertTo-ForwardSlashPath (
            [System.IO.Path]::GetRelativePath($repoRoot, $capturePath)
        )
    } else {
        $null
    }
    if ($l1Passed) {
        try {
            Copy-Item -LiteralPath $capturePath -Destination $outPath -Force
            Remove-Item -LiteralPath $capturePath -Force
            $screenshotArtifactPath = "docs/images/$safeName.png"
        } catch {
            $l1Passed = $false
            $l1Reason = "screenshot_publish_failed"
        }
    }

    $unknownHle = @(
        if ($null -ne $diagnostics) {
            $diagnostics.unknown_hle | ForEach-Object { $_ }
        }
    )
    $unknownHleNames = @($unknownHle | ForEach-Object { $_.name } | Sort-Object -Unique)
    $logTail = Get-LogTail $result.Output
    $processStatus = if ($result.TimedOut) {
        "timeout"
    } elseif ($null -ne $launchError) {
        "launch_error"
    } elseif ($result.ExitCode -eq 0) {
        "completed"
    } else {
        "failed"
    }
    $highestLevel = if ($l1Passed) {
        "L1"
    } elseif ($l0Passed) {
        "L0"
    } else {
        "none"
    }

    $record = [pscustomobject][ordered]@{
        relative_path = "tmp/dingoo_game/$(ConvertTo-ForwardSlashPath $relativePath)"
        content_name = $game.Name
        content_sha256 = Get-Sha256 $game.FullName
        git_commit = $gitCommit
        git_dirty = $gitDirty
        capture_frames = $captureFrames
        duration_ms = [long][math]::Round($timer.Elapsed.TotalMilliseconds)
        process = [pscustomobject][ordered]@{
            status = $processStatus
            exit_code = $result.ExitCode
            timed_out = [bool]$result.TimedOut
        }
        levels = [pscustomobject][ordered]@{
            highest = $highestLevel
            l0 = [pscustomobject][ordered]@{
                status = if ($l0Passed) { "pass" } else { "fail" }
                reason = $l0Reason
            }
            l1 = [pscustomobject][ordered]@{
                status = if ($l1Passed) { "pass" } else { "fail" }
                reason = $l1Reason
            }
        }
        run = if ($null -ne $diagnostics) { $diagnostics.run } else { $null }
        framebuffer = $framebuffer
        unknown_hle = @($unknownHle)
        artifacts = [pscustomobject][ordered]@{
            screenshot_path = $screenshotArtifactPath
            screenshot_bytes = $screenshotSize
            screenshot_sha256 = $screenshotSha256
            diagnostics_path = ConvertTo-ForwardSlashPath (
                [System.IO.Path]::GetRelativePath($repoRoot, $reportPath)
            )
            diagnostics_bytes = $reportSize
            diagnostics_schema_version = if ($null -ne $diagnostics) {
                $diagnostics.schema_version
            } else {
                $null
            }
        }
        log_summary = [pscustomobject][ordered]@{
            line_count = if ([string]::IsNullOrWhiteSpace($result.Output)) {
                0
            } else {
                @($result.Output -split '\r?\n' | Where-Object { $_ }).Count
            }
            tail = @($logTail)
            diagnostics_error = $diagnosticsError
        }
    }
    $records.Add($record)

    if ($l1Passed) {
        Write-Host "L1 PASS ($($framebuffer.unique_colors) colors, $(@($unknownHle).Count) unknown HLE)" -ForegroundColor Green
    } elseif ($l0Passed) {
        Write-Host "L0 PASS / L1 FAIL ($l1Reason)" -ForegroundColor Yellow
    } else {
        Write-Host "L0 FAIL ($l0Reason)" -ForegroundColor Red
    }
    if (-not $l1Passed) {
        foreach ($line in @($logTail | Select-Object -Last 2)) {
            Write-Host "    $line" -ForegroundColor DarkGray
        }
    }
}

$l0PassCount = @($records | Where-Object { $_.levels.l0.status -eq "pass" }).Count
$l1PassCount = @($records | Where-Object { $_.levels.l1.status -eq "pass" }).Count
$processFailureCount = @($records | Where-Object { $_.process.status -ne "completed" }).Count
$unknownGameCount = @($records | Where-Object { $_.unknown_hle.Count -gt 0 }).Count
$unknownNames = @(
    $records.unknown_hle |
        ForEach-Object { $_.name } |
        Sort-Object -Unique
)

$summary = [ordered]@{
    schema_version = 1
    generated_at_utc = [DateTimeOffset]::UtcNow.ToString("o")
    source = [ordered]@{
        git_commit = $gitCommit
        git_dirty = $gitDirty
        binary_path = ConvertTo-ForwardSlashPath (
            [System.IO.Path]::GetRelativePath($repoRoot, $Binary)
        )
        binary_sha256 = $binarySha256
    }
    platform = [ordered]@{
        os = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
        architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
        powershell = $PSVersionTable.PSVersion.ToString()
    }
    config = [ordered]@{
        game_directory = "tmp/dingoo_game"
        screenshot_directory = "docs/images"
        report_directory = ConvertTo-ForwardSlashPath (
            [System.IO.Path]::GetRelativePath($repoRoot, $ReportDirectory)
        )
        default_frames = $Frames
        performance_overrides = -not $framesSpecified
        timeout_seconds = $TimeoutSeconds
        unknown_hle_policy = $UnknownHlePolicy
        allow_unknown_hle = @($AllowUnknownHle)
    }
    level_definitions = [ordered]@{
        l0 = "The content loaded and produced a valid diagnostics report."
        l1 = "L0 passed, the requested frames completed, and the captured framebuffer contains non-black pixels and more than one RGB565 color."
    }
    totals = [ordered]@{
        games = $records.Count
        process_completed = $records.Count - $processFailureCount
        process_failed = $processFailureCount
        l0_pass = $l0PassCount
        l0_fail = $records.Count - $l0PassCount
        l1_pass = $l1PassCount
        l1_fail = $records.Count - $l1PassCount
        games_with_unknown_hle = $unknownGameCount
        distinct_unknown_hle = $unknownNames.Count
        duration_ms = [long][math]::Round(
            ([DateTimeOffset]::UtcNow - $runStartedAt).TotalMilliseconds
        )
    }
    games = @($records)
}

$summary | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $summaryJsonPath -Encoding utf8
$csvRows = @(
    $records | ForEach-Object {
        [pscustomobject][ordered]@{
            relative_path = $_.relative_path
            content_sha256 = $_.content_sha256
            git_commit = $_.git_commit
            git_dirty = $_.git_dirty
            capture_frames = $_.capture_frames
            duration_ms = $_.duration_ms
            process_status = $_.process.status
            exit_code = $_.process.exit_code
            timed_out = $_.process.timed_out
            highest_level = $_.levels.highest
            l0_status = $_.levels.l0.status
            l0_reason = $_.levels.l0.reason
            l1_status = $_.levels.l1.status
            l1_reason = $_.levels.l1.reason
            executed_frames = $_.run.executed_frames
            executed_instructions = $_.run.executed_instructions
            unique_colors = $_.framebuffer.unique_colors
            non_black_pixels = $_.framebuffer.non_black_pixels
            dominant_color_rgb565 = $_.framebuffer.dominant_color_rgb565
            dominant_color_pixels = $_.framebuffer.dominant_color_pixels
            unknown_hle_count = @($_.unknown_hle).Count
            unknown_hle_names = (@($_.unknown_hle | ForEach-Object { $_.name } | Sort-Object -Unique) -join ";")
            screenshot_sha256 = $_.artifacts.screenshot_sha256
            screenshot_path = $_.artifacts.screenshot_path
            diagnostics_path = $_.artifacts.diagnostics_path
            log_tail = ($_.log_summary.tail -join " | ")
        }
    }
)
$csvRows | Export-Csv -LiteralPath $summaryCsvPath -NoTypeInformation -Encoding utf8

Write-Host ""
Write-Host "L0: $l0PassCount passed, $($records.Count - $l0PassCount) failed"
Write-Host "L1: $l1PassCount passed, $($records.Count - $l1PassCount) failed"
Write-Host "Unknown HLE: $unknownGameCount game(s), $($unknownNames.Count) distinct name(s)"
Write-Host "JSON summary: $summaryJsonPath"
Write-Host "CSV summary:  $summaryCsvPath"

if ($l1PassCount -ne $records.Count) {
    exit 1
}
