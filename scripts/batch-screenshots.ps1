<#
.SYNOPSIS
    Batch-generate screenshots for all .app games in tmp/dingoo_game.

.DESCRIPTION
    Runs the dingoo-emu binary in screenshot mode for every .app file found
    under tmp/dingoo_game recursively. Output PNGs are saved to docs/images
    and named after each game file. Per-game diagnostics plus summary.json and
    summary.csv are written to the report directory. L0 requires a valid load
    report; L1 additionally requires a successful non-solid framebuffer
    capture. Games with a matching input scenario are additionally graded at
    L2 using deterministic per-frame button events and framebuffer checkpoints.
    Games with matching audio scenarios are graded at L3 using exact guest PCM
    evidence, format checks, and bounded virtual queue behavior.
    When no binary is supplied, the latest release binary is built before
    capture.

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

.PARAMETER InputScenarioDirectory
    Directory containing versioned L2 input scenario JSON files. Default:
    compatibility/l2-input.

.PARAMETER AudioScenarioDirectory
    Directory containing versioned L3 audio scenario JSON files. Default:
    compatibility/l3-audio.
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

    [string[]]$AllowUnknownHle = @(),

    [string]$InputScenarioDirectory = "",

    [string]$AudioScenarioDirectory = ""
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
if (-not $InputScenarioDirectory) {
    $InputScenarioDirectory = Join-Path $repoRoot "compatibility\l2-input"
} elseif (-not [System.IO.Path]::IsPathRooted($InputScenarioDirectory)) {
    $InputScenarioDirectory = Join-Path $repoRoot $InputScenarioDirectory
}
if (-not $AudioScenarioDirectory) {
    $AudioScenarioDirectory = Join-Path $repoRoot "compatibility\l3-audio"
} elseif (-not [System.IO.Path]::IsPathRooted($AudioScenarioDirectory)) {
    $AudioScenarioDirectory = Join-Path $repoRoot $AudioScenarioDirectory
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
        "7day-20081217192316.app" { return 30 }
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
        [string[]]$AllowedUnknownHle,
        [AllowNull()]
        [string]$InputScriptPath
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
    if ($InputScriptPath) {
        $startInfo.ArgumentList.Add("--input-script")
        $startInfo.ArgumentList.Add($InputScriptPath)
    }
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
$inputScenarios = @{}
$audioScenarios = @{}

if (Test-Path -LiteralPath $InputScenarioDirectory -PathType Container) {
    foreach ($scenarioFile in Get-ChildItem -LiteralPath $InputScenarioDirectory -Filter "*.json" -File) {
        try {
            $scenario = Get-Content -Raw -LiteralPath $scenarioFile.FullName | ConvertFrom-Json
            foreach ($property in @(
                "schema_version", "content", "relative_path", "content_sha256",
                "frames", "events", "checkpoints"
            )) {
                if ($null -eq $scenario.PSObject.Properties[$property]) {
                    throw "Missing scenario property: $property"
                }
            }
            if ([int]$scenario.schema_version -ne 1) {
                throw "Unsupported scenario schema version: $($scenario.schema_version)"
            }
            if ([int]$scenario.frames -le 0 -or @($scenario.events).Count -eq 0 -or
                @($scenario.checkpoints).Count -eq 0) {
                throw "Scenario must define frames, events, and checkpoints."
            }
            $scenarioKey = (ConvertTo-ForwardSlashPath ([string]$scenario.relative_path)).TrimStart('/')
            if ($inputScenarios.ContainsKey($scenarioKey)) {
                throw "Duplicate input scenario relative_path: $scenarioKey"
            }
            $inputScenarios[$scenarioKey] = [pscustomobject]@{
                Path = $scenarioFile.FullName
                Sha256 = Get-Sha256 $scenarioFile.FullName
                Data = $scenario
            }
        } catch {
            throw "Invalid input scenario '$($scenarioFile.FullName)': $($_.Exception.Message)"
        }
    }
}

if (Test-Path -LiteralPath $AudioScenarioDirectory -PathType Container) {
    foreach ($scenarioFile in Get-ChildItem -LiteralPath $AudioScenarioDirectory -Filter "*.json" -File) {
        try {
            $scenario = Get-Content -Raw -LiteralPath $scenarioFile.FullName | ConvertFrom-Json
            foreach ($property in @(
                "schema_version", "content", "relative_path", "content_sha256",
                "input_script", "input_script_sha256", "expected"
            )) {
                if ($null -eq $scenario.PSObject.Properties[$property]) {
                    throw "Missing audio scenario property: $property"
                }
            }
            if ([int]$scenario.schema_version -ne 1) {
                throw "Unsupported audio scenario schema version: $($scenario.schema_version)"
            }
            foreach ($property in @(
                "sample_rate", "format", "channels", "volume", "pcm_crc32",
                "submitted_bytes", "decoded_frames", "nonzero_samples",
                "min_rms_amplitude", "max_clipped_samples",
                "max_rejected_write_calls", "max_silenced_write_calls",
                "max_underflow_frames", "max_consecutive_underflow_frames",
                "max_buffered_frames"
            )) {
                if ($null -eq $scenario.expected.PSObject.Properties[$property]) {
                    throw "Missing audio expectation property: $property"
                }
            }
            if ([int]$scenario.expected.sample_rate -le 0 -or
                [int]$scenario.expected.channels -notin @(1, 2) -or
                [string]$scenario.expected.format -notin @("U8", "S16Le") -or
                [string]$scenario.expected.pcm_crc32 -notmatch '^[0-9a-fA-F]{8}$' -or
                [double]$scenario.expected.min_rms_amplitude -le 0) {
                throw "Audio expectations contain an invalid format or threshold."
            }
            $scenarioKey = (ConvertTo-ForwardSlashPath ([string]$scenario.relative_path)).TrimStart('/')
            if ($audioScenarios.ContainsKey($scenarioKey)) {
                throw "Duplicate audio scenario relative_path: $scenarioKey"
            }
            $inputPath = [System.IO.Path]::GetFullPath(
                (Join-Path $repoRoot ([string]$scenario.input_script))
            )
            $inputRelativePath = [System.IO.Path]::GetRelativePath($repoRoot, $inputPath)
            if ([System.IO.Path]::IsPathRooted($inputRelativePath) -or
                $inputRelativePath -eq ".." -or $inputRelativePath.StartsWith("..$([System.IO.Path]::DirectorySeparatorChar)")) {
                throw "Audio scenario input_script escapes the repository."
            }
            if (-not (Test-Path -LiteralPath $inputPath -PathType Leaf)) {
                throw "Audio scenario input_script does not exist: $inputPath"
            }
            if (-not ([string]$scenario.input_script_sha256).Equals(
                (Get-Sha256 $inputPath),
                [System.StringComparison]::OrdinalIgnoreCase
            )) {
                throw "Audio scenario input_script_sha256 does not match the file."
            }
            $audioScenarios[$scenarioKey] = [pscustomobject]@{
                Path = $scenarioFile.FullName
                Sha256 = Get-Sha256 $scenarioFile.FullName
                Data = $scenario
                InputPath = $inputPath
            }
        } catch {
            throw "Invalid audio scenario '$($scenarioFile.FullName)': $($_.Exception.Message)"
        }
    }
}

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
Write-Host "L2 scenarios: $($inputScenarios.Count) from $InputScenarioDirectory"
Write-Host "L3 scenarios: $($audioScenarios.Count) from $AudioScenarioDirectory"
if ($framesSpecified) {
    Write-Host "Frames:       $Frames"
} else {
    Write-Host "Frames:       $Frames (with performance overrides)"
}
Write-Host "Timeout:      $TimeoutSeconds seconds per game"
Write-Host "Levels:       L0=load report, L1=completed non-solid frame, L2=scripted checkpoints, L3=verified PCM"
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
$matchedInputScenarios = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)
$matchedAudioScenarios = [System.Collections.Generic.HashSet[string]]::new(
    [System.StringComparer]::OrdinalIgnoreCase
)

foreach ($game in $games) {
    $baseName = [System.IO.Path]::GetFileNameWithoutExtension($game.Name)
    $relativePath = [System.IO.Path]::GetRelativePath($gameDir, $game.FullName)
    $relativePathKey = ConvertTo-ForwardSlashPath $relativePath
    $relativeBaseName = [System.IO.Path]::ChangeExtension($relativePath, $null)
    $relativeName = $relativeBaseName -replace '[/\\]+', '__'
    $safeName = ConvertTo-ScreenshotName $relativeName
    $captureFrames = Get-CaptureFrames $relativePath $Frames (-not $framesSpecified)
    $contentSha256 = Get-Sha256 $game.FullName
    $inputScenario = if ($inputScenarios.ContainsKey($relativePathKey)) {
        [void]$matchedInputScenarios.Add($relativePathKey)
        $inputScenarios[$relativePathKey]
    } else {
        $null
    }
    $audioScenario = if ($audioScenarios.ContainsKey($relativePathKey)) {
        [void]$matchedAudioScenarios.Add($relativePathKey)
        $audioScenarios[$relativePathKey]
    } else {
        $null
    }
    $inputScenarioError = $null
    $inputScriptPath = $null
    if ($null -ne $inputScenario) {
        if ($inputScenario.Data.content -ne $game.Name) {
            $inputScenarioError = "scenario_content_mismatch"
        } elseif (-not ([string]$inputScenario.Data.content_sha256).Equals(
            $contentSha256,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            $inputScenarioError = "scenario_content_hash_mismatch"
        } else {
            $captureFrames = [int]$inputScenario.Data.frames
            $inputScriptPath = $inputScenario.Path
        }
    }
    $audioScenarioError = $null
    if ($null -ne $audioScenario) {
        $expectedInputPath = if ($null -ne $inputScenario) {
            ConvertTo-ForwardSlashPath (
                [System.IO.Path]::GetRelativePath($repoRoot, $inputScenario.Path)
            )
        } else {
            $null
        }
        if ($audioScenario.Data.content -ne $game.Name) {
            $audioScenarioError = "audio_scenario_content_mismatch"
        } elseif (-not ([string]$audioScenario.Data.content_sha256).Equals(
            $contentSha256,
            [System.StringComparison]::OrdinalIgnoreCase
        )) {
            $audioScenarioError = "audio_scenario_content_hash_mismatch"
        } elseif ($null -eq $inputScenario -or
            [string]$audioScenario.Data.input_script -ne $expectedInputPath -or
            -not ([string]$audioScenario.Data.input_script_sha256).Equals(
                $inputScenario.Sha256,
                [System.StringComparison]::OrdinalIgnoreCase
            )) {
            $audioScenarioError = "audio_scenario_input_mismatch"
        }
    }

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
            -AllowedUnknownHle $AllowUnknownHle `
            -InputScriptPath $inputScriptPath
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
            foreach ($property in @(
                "schema_version", "content", "run", "framebuffer", "input", "audio", "unknown_hle"
            )) {
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
    $inputDiagnostics = if ($null -ne $diagnostics) { $diagnostics.input } else { $null }
    $l2Tested = $null -ne $inputScenario
    $l2Passed = $false
    $checkpointMetadataMatches = $true
    if ($null -ne $inputDiagnostics -and $null -ne $inputScenario) {
        $actualCheckpoints = @($inputDiagnostics.checkpoints)
        $expectedCheckpoints = @($inputScenario.Data.checkpoints)
        if ($actualCheckpoints.Count -eq $expectedCheckpoints.Count) {
            for ($checkpointIndex = 0; $checkpointIndex -lt $expectedCheckpoints.Count; $checkpointIndex++) {
                $actualCheckpoint = $actualCheckpoints[$checkpointIndex]
                $expectedCheckpoint = $expectedCheckpoints[$checkpointIndex]
                if ($actualCheckpoint.name -ne $expectedCheckpoint.name -or
                    [int]$actualCheckpoint.frame -ne [int]$expectedCheckpoint.frame -or
                    $actualCheckpoint.expected_framebuffer_crc32 -ne
                        $expectedCheckpoint.expected_framebuffer_crc32 -or
                    $actualCheckpoint.control_framebuffer_crc32 -ne
                        $expectedCheckpoint.control_framebuffer_crc32) {
                    $checkpointMetadataMatches = $false
                    break
                }
            }
        } else {
            $checkpointMetadataMatches = $false
        }
    }
    $l2Reason = if (-not $l2Tested) {
        "no_input_scenario"
    } elseif ($null -ne $inputScenarioError) {
        $inputScenarioError
    } elseif (-not $l1Passed) {
        "l1_failed"
    } elseif ($null -eq $inputDiagnostics) {
        "missing_input_diagnostics"
    } elseif ($inputDiagnostics.content -ne $game.Name -or
        $inputDiagnostics.relative_path -ne $relativePathKey -or
        -not ([string]$inputDiagnostics.content_sha256).Equals(
            $contentSha256,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -or [int]$inputDiagnostics.frames -ne $captureFrames -or
        [int]$inputDiagnostics.event_count -ne @($inputScenario.Data.events).Count -or
        -not $checkpointMetadataMatches) {
        "input_diagnostics_mismatch"
    } elseif ([int]$inputDiagnostics.nonzero_input_frames -le 0) {
        "no_nonzero_input_frames"
    } elseif (@($inputDiagnostics.checkpoints).Count -ne @($inputScenario.Data.checkpoints).Count) {
        "incomplete_input_checkpoints"
    } elseif (-not [bool]$inputDiagnostics.all_checkpoints_passed -or
        @($inputDiagnostics.checkpoints | Where-Object {
            $_.status -ne "pass" -or -not [bool]$_.differs_from_control
        }).Count -gt 0) {
        "framebuffer_checkpoint_mismatch"
    } else {
        $l2Passed = $true
        "scripted_checkpoints_matched"
    }
    $audioDiagnostics = if ($null -ne $diagnostics) { $diagnostics.audio } else { $null }
    $l3Tested = $null -ne $audioScenario
    $l3Passed = $false
    $audioExpected = if ($l3Tested) { $audioScenario.Data.expected } else { $null }
    $audioConfigurations = if ($null -ne $audioDiagnostics) {
        @($audioDiagnostics.configurations)
    } else {
        @()
    }
    $l3Reason = if (-not $l3Tested) {
        "no_audio_scenario"
    } elseif ($null -ne $audioScenarioError) {
        $audioScenarioError
    } elseif (-not $l2Passed) {
        "l2_failed"
    } elseif ($null -eq $audioDiagnostics) {
        "missing_audio_diagnostics"
    } elseif ([int]$audioDiagnostics.schema_version -ne 1) {
        "unsupported_audio_diagnostics"
    } elseif ([uint64]$audioDiagnostics.open_count -le 0 -or
        [uint64]$audioDiagnostics.successful_write_calls -le 0) {
        "no_audio_stream"
    } elseif ($audioConfigurations.Count -ne 1 -or
        [uint64]$audioConfigurations[0].sample_rate -ne [uint64]$audioExpected.sample_rate -or
        [string]$audioConfigurations[0].format -ne [string]$audioExpected.format -or
        [uint64]$audioConfigurations[0].channels -ne [uint64]$audioExpected.channels -or
        [uint64]$audioConfigurations[0].volume -ne [uint64]$audioExpected.volume) {
        "audio_format_mismatch"
    } elseif ([uint64]$audioDiagnostics.nonzero_samples -eq 0 -or
        [double]$audioDiagnostics.rms_amplitude -lt [double]$audioExpected.min_rms_amplitude) {
        "silent_audio"
    } elseif (-not ([string]$audioDiagnostics.pcm_crc32).Equals(
            [string]$audioExpected.pcm_crc32,
            [System.StringComparison]::OrdinalIgnoreCase
        ) -or
        [uint64]$audioDiagnostics.submitted_bytes -ne [uint64]$audioExpected.submitted_bytes -or
        [uint64]$audioDiagnostics.decoded_frames -ne [uint64]$audioExpected.decoded_frames -or
        [uint64]$audioDiagnostics.nonzero_samples -ne [uint64]$audioExpected.nonzero_samples) {
        "pcm_checkpoint_mismatch"
    } elseif ([uint64]$audioDiagnostics.rejected_write_calls -gt
            [uint64]$audioExpected.max_rejected_write_calls -or
        [uint64]$audioDiagnostics.silenced_write_calls -gt
            [uint64]$audioExpected.max_silenced_write_calls) {
        "audio_write_rejected"
    } elseif ([uint64]$audioDiagnostics.underflow_frames -gt
            [uint64]$audioExpected.max_underflow_frames -or
        [uint64]$audioDiagnostics.max_consecutive_underflow_frames -gt
            [uint64]$audioExpected.max_consecutive_underflow_frames) {
        "audio_queue_underflow"
    } elseif ([uint64]$audioDiagnostics.max_buffered_frames -gt
            [uint64]$audioExpected.max_buffered_frames) {
        "audio_queue_unbounded"
    } elseif ([uint64]$audioDiagnostics.clipped_samples -gt
            [uint64]$audioExpected.max_clipped_samples) {
        "audio_clipping_exceeded"
    } else {
        $l3Passed = $true
        "pcm_checkpoint_matched"
    }
    if ($l1Passed -and (-not $l2Tested -or $l2Passed) -and
        (-not $l3Tested -or $l3Passed)) {
        try {
            Copy-Item -LiteralPath $capturePath -Destination $outPath -Force
            Remove-Item -LiteralPath $capturePath -Force
            $screenshotArtifactPath = "docs/images/$safeName.png"
        } catch {
            $l1Passed = $false
            $l1Reason = "screenshot_publish_failed"
            if ($l2Tested) {
                $l2Passed = $false
                $l2Reason = "screenshot_publish_failed"
            }
            if ($l3Tested) {
                $l3Passed = $false
                $l3Reason = "screenshot_publish_failed"
            }
        }
    }
    $highestLevel = if ($l3Passed) {
        "L3"
    } elseif ($l2Passed) {
        "L2"
    } elseif ($l1Passed) {
        "L1"
    } elseif ($l0Passed) {
        "L0"
    } else {
        "none"
    }

    $record = [pscustomobject][ordered]@{
        relative_path = "tmp/dingoo_game/$(ConvertTo-ForwardSlashPath $relativePath)"
        content_name = $game.Name
        content_sha256 = $contentSha256
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
            l2 = [pscustomobject][ordered]@{
                status = if (-not $l2Tested) {
                    "not_tested"
                } elseif ($l2Passed) {
                    "pass"
                } else {
                    "fail"
                }
                reason = $l2Reason
            }
            l3 = [pscustomobject][ordered]@{
                status = if (-not $l3Tested) {
                    "not_tested"
                } elseif ($l3Passed) {
                    "pass"
                } else {
                    "fail"
                }
                reason = $l3Reason
            }
        }
        run = if ($null -ne $diagnostics) { $diagnostics.run } else { $null }
        framebuffer = $framebuffer
        input = $inputDiagnostics
        audio = $audioDiagnostics
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
            input_script_path = if ($null -ne $inputScenario) {
                ConvertTo-ForwardSlashPath (
                    [System.IO.Path]::GetRelativePath($repoRoot, $inputScenario.Path)
                )
            } else {
                $null
            }
            input_script_sha256 = if ($null -ne $inputScenario) {
                $inputScenario.Sha256
            } else {
                $null
            }
            audio_scenario_path = if ($null -ne $audioScenario) {
                ConvertTo-ForwardSlashPath (
                    [System.IO.Path]::GetRelativePath($repoRoot, $audioScenario.Path)
                )
            } else {
                $null
            }
            audio_scenario_sha256 = if ($null -ne $audioScenario) {
                $audioScenario.Sha256
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

    if ($l3Passed) {
        Write-Host "L3 PASS ($($audioDiagnostics.nonzero_samples) nonzero samples, PCM $($audioDiagnostics.pcm_crc32))" -ForegroundColor Green
    } elseif ($l3Tested) {
        Write-Host "$(if ($l2Passed) { 'L2 PASS' } else { 'L2 FAIL' }) / L3 FAIL ($l3Reason)" -ForegroundColor Yellow
    } elseif ($l2Passed) {
        Write-Host "L2 PASS ($(@($inputDiagnostics.checkpoints).Count) checkpoints)" -ForegroundColor Green
    } elseif ($l2Tested) {
        Write-Host "$(if ($l1Passed) { 'L1 PASS' } else { 'L1 FAIL' }) / L2 FAIL ($l2Reason)" -ForegroundColor Yellow
    } elseif ($l1Passed) {
        Write-Host "L1 PASS ($($framebuffer.unique_colors) colors, $(@($unknownHle).Count) unknown HLE)" -ForegroundColor Green
    } elseif ($l0Passed) {
        Write-Host "L0 PASS / L1 FAIL ($l1Reason)" -ForegroundColor Yellow
    } else {
        Write-Host "L0 FAIL ($l0Reason)" -ForegroundColor Red
    }
    if (-not $l1Passed -or ($l2Tested -and -not $l2Passed) -or
        ($l3Tested -and -not $l3Passed)) {
        foreach ($line in @($logTail | Select-Object -Last 2)) {
            Write-Host "    $line" -ForegroundColor DarkGray
        }
    }
}

$l0PassCount = @($records | Where-Object { $_.levels.l0.status -eq "pass" }).Count
$l1PassCount = @($records | Where-Object { $_.levels.l1.status -eq "pass" }).Count
$l2TestedCount = @($records | Where-Object { $_.levels.l2.status -ne "not_tested" }).Count
$l2PassCount = @($records | Where-Object { $_.levels.l2.status -eq "pass" }).Count
$l3TestedCount = @($records | Where-Object { $_.levels.l3.status -ne "not_tested" }).Count
$l3PassCount = @($records | Where-Object { $_.levels.l3.status -eq "pass" }).Count
$processFailureCount = @($records | Where-Object { $_.process.status -ne "completed" }).Count
$unknownGameCount = @($records | Where-Object { $_.unknown_hle.Count -gt 0 }).Count
$unknownNames = @(
    $records.unknown_hle |
        ForEach-Object { $_.name } |
        Sort-Object -Unique
)
$unusedInputScenarios = @(
    $inputScenarios.Keys | Where-Object { -not $matchedInputScenarios.Contains($_) }
)
$unusedAudioScenarios = @(
    $audioScenarios.Keys | Where-Object { -not $matchedAudioScenarios.Contains($_) }
)

$summary = [ordered]@{
    schema_version = 3
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
        input_scenario_directory = ConvertTo-ForwardSlashPath (
            [System.IO.Path]::GetRelativePath($repoRoot, $InputScenarioDirectory)
        )
        audio_scenario_directory = ConvertTo-ForwardSlashPath (
            [System.IO.Path]::GetRelativePath($repoRoot, $AudioScenarioDirectory)
        )
    }
    level_definitions = [ordered]@{
        l0 = "The content loaded and produced a valid diagnostics report."
        l1 = "L0 passed, the requested frames completed, and the captured framebuffer contains non-black pixels and more than one RGB565 color."
        l2 = "L1 passed and versioned per-frame input produced every expected RGB565 framebuffer checkpoint for matching content."
        l3 = "L2 passed and the matching audio scenario produced the expected non-silent PCM stream, format, and bounded queue evidence."
    }
    totals = [ordered]@{
        games = $records.Count
        process_completed = $records.Count - $processFailureCount
        process_failed = $processFailureCount
        l0_pass = $l0PassCount
        l0_fail = $records.Count - $l0PassCount
        l1_pass = $l1PassCount
        l1_fail = $records.Count - $l1PassCount
        l2_tested = $l2TestedCount
        l2_pass = $l2PassCount
        l2_fail = $l2TestedCount - $l2PassCount
        l3_tested = $l3TestedCount
        l3_pass = $l3PassCount
        l3_fail = $l3TestedCount - $l3PassCount
        input_scenarios_unused = $unusedInputScenarios.Count
        audio_scenarios_unused = $unusedAudioScenarios.Count
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
            l2_status = $_.levels.l2.status
            l2_reason = $_.levels.l2.reason
            l3_status = $_.levels.l3.status
            l3_reason = $_.levels.l3.reason
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
            input_script_sha256 = $_.artifacts.input_script_sha256
            input_event_count = if ($null -ne $_.input) { $_.input.event_count } else { $null }
            input_nonzero_frames = if ($null -ne $_.input) { $_.input.nonzero_input_frames } else { $null }
            input_checkpoint_count = if ($null -ne $_.input) { @($_.input.checkpoints).Count } else { $null }
            audio_scenario_sha256 = $_.artifacts.audio_scenario_sha256
            audio_configurations = if ($null -ne $_.audio) {
                @($_.audio.configurations | ForEach-Object {
                    "$($_.sample_rate)/$($_.format)/$($_.channels)/$($_.volume)"
                }) -join ";"
            } else {
                $null
            }
            audio_pcm_crc32 = if ($null -ne $_.audio) { $_.audio.pcm_crc32 } else { $null }
            audio_open_count = if ($null -ne $_.audio) { $_.audio.open_count } else { $null }
            audio_write_count = if ($null -ne $_.audio) { $_.audio.successful_write_calls } else { $null }
            audio_submitted_bytes = if ($null -ne $_.audio) { $_.audio.submitted_bytes } else { $null }
            audio_decoded_frames = if ($null -ne $_.audio) { $_.audio.decoded_frames } else { $null }
            audio_nonzero_samples = if ($null -ne $_.audio) { $_.audio.nonzero_samples } else { $null }
            audio_peak_amplitude = if ($null -ne $_.audio) { $_.audio.peak_amplitude } else { $null }
            audio_rms_amplitude = if ($null -ne $_.audio) { $_.audio.rms_amplitude } else { $null }
            audio_clipped_samples = if ($null -ne $_.audio) { $_.audio.clipped_samples } else { $null }
            audio_rejected_writes = if ($null -ne $_.audio) { $_.audio.rejected_write_calls } else { $null }
            audio_queue_full_events = if ($null -ne $_.audio) { $_.audio.queue_full_events } else { $null }
            audio_underflow_frames = if ($null -ne $_.audio) { $_.audio.underflow_frames } else { $null }
            audio_max_consecutive_underflow_frames = if ($null -ne $_.audio) { $_.audio.max_consecutive_underflow_frames } else { $null }
            audio_max_buffered_frames = if ($null -ne $_.audio) { $_.audio.max_buffered_frames } else { $null }
            diagnostics_path = $_.artifacts.diagnostics_path
            log_tail = ($_.log_summary.tail -join " | ")
        }
    }
)
$csvRows | Export-Csv -LiteralPath $summaryCsvPath -NoTypeInformation -Encoding utf8

Write-Host ""
Write-Host "L0: $l0PassCount passed, $($records.Count - $l0PassCount) failed"
Write-Host "L1: $l1PassCount passed, $($records.Count - $l1PassCount) failed"
Write-Host "L2: $l2PassCount passed, $($l2TestedCount - $l2PassCount) failed, $($records.Count - $l2TestedCount) not tested"
Write-Host "L3: $l3PassCount passed, $($l3TestedCount - $l3PassCount) failed, $($records.Count - $l3TestedCount) not tested"
Write-Host "Unused L2 scenarios: $($unusedInputScenarios.Count)"
Write-Host "Unused L3 scenarios: $($unusedAudioScenarios.Count)"
Write-Host "Unknown HLE: $unknownGameCount game(s), $($unknownNames.Count) distinct name(s)"
Write-Host "JSON summary: $summaryJsonPath"
Write-Host "CSV summary:  $summaryCsvPath"

if ($l1PassCount -ne $records.Count -or $l2PassCount -ne $l2TestedCount -or
    $l3PassCount -ne $l3TestedCount -or $unusedInputScenarios.Count -ne 0 -or
    $unusedAudioScenarios.Count -ne 0) {
    exit 1
}
