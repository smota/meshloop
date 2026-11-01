param(
    [ValidateSet('Check', 'Prepare', 'Batch')][string]$Mode = 'Check'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$commits = [ordered]@{ A = '7081d6080722633699213f11e7dad56db71fff3c'; B = '2f495950e06095c310fcd3d44fa225d98217536f' }
$pairs = @('AB', 'BA', 'AB', 'BA', 'AB', 'BA')
$expectedProbe = 'C578AD84D32B2893EE2CAB3BE71AA693D0A6E0A7889511C58FABD2B51891D74D'
$probe = Join-Path $PSScriptRoot 'wal_probe.rs'
$control = Join-Path $PSScriptRoot 'storage_control.py'
function Hash([string]$Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash }
function AssertSamples($Result) {
    if ($Result.sample_count -ne 50 -or $Result.ordered_samples_ms.Count -ne 50 -or $Result.p95_index_zero_based -ne 47) { throw 'Sample count/index mismatch' }
    $values = foreach ($sample in $Result.ordered_samples_ms) {
        if ($sample -isnot [ValueType] -or $sample -is [bool]) { throw 'Non-numeric sample' }
        $value = [double]$sample
        if (-not [double]::IsFinite($value) -or $value -lt 0) { throw 'Invalid latency sample' }
        $value
    }
    $expected = @($values | Sort-Object)[47]
    if (-not [double]::IsFinite([double]$Result.p95_ms) -or [double]$Result.p95_ms -ne $expected) { throw 'Independent p95 recomputation mismatch' }
}
if ((Hash $probe) -ne $expectedProbe) { throw 'Reviewed probe bytes changed' }
if ($Mode -eq 'Batch' -and ($env:GITHUB_ACTIONS -ne 'true' -or $env:GITHUB_EVENT_NAME -ne 'pull_request' -or $env:ISSUE82_PR_NUMBER -ne '87' -or $env:GITHUB_RUN_ATTEMPT -ne '1' -or -not $IsWindows)) {
    throw 'Measurements restricted to first-attempt PR87 Windows GitHub Actions job'
}
if ($Mode -eq 'Check') {
    # Deterministic synthetic data only: never start a timed workload in Check.
    $synthetic = [pscustomobject]@{ sample_count = 50; ordered_samples_ms = @(0..49); p95_index_zero_based = 47; p95_ms = 47 }
    AssertSamples $synthetic
    foreach ($invalid in @([double]::NaN, [double]::PositiveInfinity, -1)) {
        $synthetic.ordered_samples_ms[0] = $invalid
        $rejected = $false
        try { AssertSamples $synthetic } catch { $rejected = $true }
        if (-not $rejected) { throw 'Invalid sample self-check failed' }
    }
    foreach ($entry in $commits.GetEnumerator()) {
        $resolved = & git -C $repo rev-parse "$($entry.Value)^{commit}"
        if ($LASTEXITCODE -ne 0 -or $resolved -ne $entry.Value) { throw 'Pinned revision missing' }
    }
    Write-Output 'Static contract check passed. No builds or measurements executed.'
    exit 0
}
$outputRoot = Join-Path $repo ('.agent-runs/issue-82/remote-wal-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $outputRoot | Out-Null
# Reserve three minutes of the thirty-minute job cap for partial-evidence upload.
$deadline = [DateTime]::UtcNow.AddMinutes(27)
$receipts = [System.Collections.Generic.List[object]]::new()
function SaveJson([string]$Name, $Value) {
    $Value | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath (Join-Path $outputRoot $Name) -Encoding utf8
}
function Run([string]$Name, [string]$File, [string[]]$Arguments, [string]$Directory = $repo) {
    $remaining = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
    if ($remaining -le 0) { throw 'Diagnostic deadline exhausted (upload reserve retained)' }
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $File
    $start.WorkingDirectory = $Directory
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $start
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $started = [DateTime]::UtcNow.ToString('o')
    $exitCode = $null
    $timedOut = $false
    $failure = $null
    try {
        if (-not $process.Start()) { throw 'Process did not start' }
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($remaining)) {
            $timedOut = $true
            $process.Kill($true)
            $process.WaitForExit()
        }
        $exitCode = $process.ExitCode
        $stdoutTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $outputRoot "$Name.stdout") -Encoding utf8
        $stderrTask.GetAwaiter().GetResult() | Set-Content -LiteralPath (Join-Path $outputRoot "$Name.stderr") -Encoding utf8
    } catch { $failure = $_.ToString() }
    finally {
        $watch.Stop()
        $receipts.Add([ordered]@{ name = $Name; executable = $File; arguments = $Arguments; directory = $Directory; started_utc = $started; wall_ms = $watch.Elapsed.TotalMilliseconds; exit_code = $exitCode; timed_out = $timedOut; failure = $failure })
        SaveJson 'processes.json' $receipts.ToArray()
        $process.Dispose()
    }
    if ($failure -or $timedOut -or $exitCode -ne 0) { throw "First process failure: $Name; exit=$exitCode timeout=$timedOut $failure" }
    Get-Content -LiteralPath (Join-Path $outputRoot "$Name.stdout") -Raw
}
function AssertFixtureVolume($Result) {
    $fixtureVolume = [System.IO.Path]::GetPathRoot([string]$Result.fixture)
    $expectedVolume = [System.IO.Path]::GetPathRoot([System.IO.Path]::GetTempPath())
    if ($fixtureVolume -ne $expectedVolume) { throw "Fixture volume mismatch: $fixtureVolume vs $expectedVolume" }
    $fixtureVolume
}
try {
    $environment = [ordered]@{
        mode = $Mode; protocol = 'AB BA AB BA AB BA; 50 operations; no warmup/retry; control after each pair'
        commits = $commits; probe_sha256 = Hash $probe; control_sha256 = Hash $control
        runner_name = $env:RUNNER_NAME; runner_os = $env:RUNNER_OS; image_os = $env:ImageOS; image_version = $env:ImageVersion
        workflow_sha = $env:GITHUB_SHA; run_id = $env:GITHUB_RUN_ID; run_attempt = $env:GITHUB_RUN_ATTEMPT
        temp = [System.IO.Path]::GetTempPath(); temp_volume = [System.IO.Path]::GetPathRoot([System.IO.Path]::GetTempPath())
        processor = @(Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors)
        memory = @(Get-CimInstance Win32_ComputerSystem | Select-Object TotalPhysicalMemory)
        os = @(Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, FreePhysicalMemory)
        volumes = @(Get-CimInstance Win32_LogicalDisk | Select-Object DeviceID, FileSystem, Size, FreeSpace)
        rustflags = $env:RUSTFLAGS; cargo_build_jobs = $env:CARGO_BUILD_JOBS; deadline_utc = $deadline.ToString('o')
    }
    SaveJson 'environment.json' $environment
    foreach ($tool in @(@('rustc', '-Vv'), @('cargo', '-V'), @('python', '--version'), @('git', '--version'))) {
        Run "version-$($tool[0])" $tool[0] $tool[1..($tool.Count - 1)] | Out-Null
    }
    $artifacts = [ordered]@{}
    foreach ($entry in $commits.GetEnumerator()) {
        $label = $entry.Key
        $commit = (Run "commit-$label" 'git' @('-C', $repo, 'rev-parse', "$($entry.Value)^{commit}")).Trim()
        if ($commit -ne $entry.Value) { throw "Pinned commit mismatch $label" }
        $tree = (Run "tree-$label" 'git' @('-C', $repo, 'rev-parse', "$commit`^{tree}")).Trim()
        $archive = Join-Path $outputRoot "$label.tar"
        $export = Join-Path $outputRoot $label
        New-Item -ItemType Directory -Path $export | Out-Null
        Run "archive-$label" 'git' @('-C', $repo, 'archive', '--format=tar', "--output=$archive", $commit) | Out-Null
        Run "extract-$label" 'tar' @('-xf', $archive, '-C', $export) | Out-Null
        $protected = @('Cargo.toml', 'Cargo.lock', 'xtask/Cargo.toml')
        $before = [ordered]@{}
        foreach ($relative in $protected) { $before[$relative] = Hash (Join-Path $export $relative) }
        $binDir = Join-Path $export 'xtask/src/bin'
        New-Item -ItemType Directory -Force -Path $binDir | Out-Null
        $copiedProbe = Join-Path $binDir 'wal_probe.rs'
        Copy-Item -LiteralPath $probe -Destination $copiedProbe
        if ((Hash $copiedProbe) -ne $expectedProbe) { throw 'Export probe hash mismatch' }
        $env:WAL_SOURCE_COMMIT = $commit
        $env:WAL_PROBE_SHA256 = $expectedProbe
        $target = Join-Path $outputRoot "target-$label"
        Run "build-$label" 'cargo' @('build', '--release', '--locked', '-p', 'xtask', '--bin', 'wal_probe', '--target-dir', $target) $export | Out-Null
        foreach ($relative in $protected) { if ((Hash (Join-Path $export $relative)) -ne $before[$relative]) { throw "Protected manifest/lock changed: $label $relative" } }
        $binary = Join-Path $target 'release/wal_probe.exe'
        $artifacts[$label] = [ordered]@{ commit = $commit; tree = $tree; archive_sha256 = Hash $archive; binary = $binary; binary_sha256 = Hash $binary; source_sha256 = Hash $copiedProbe; manifests = $before }
        SaveJson 'artifacts.json' $artifacts
    }
    if ($Mode -eq 'Prepare') {
        SaveJson 'status.json' @{ status = 'compiled; zero measurements'; output = $outputRoot }
        Write-Output "Prepared without measurements: $outputRoot"
        exit 0
    }
    for ($pairIndex = 0; $pairIndex -lt $pairs.Count; $pairIndex++) {
        foreach ($letter in $pairs[$pairIndex].ToCharArray()) {
            $label = [string]$letter
            $artifact = $artifacts[$label]
            if ((Hash $artifact.binary) -ne $artifact.binary_sha256) { throw 'Binary changed before measurement' }
            $name = 'pair-{0}-{1}' -f ($pairIndex + 1), $label
            $result = (Run $name $artifact.binary @()) | ConvertFrom-Json
            if ($result.source_commit -ne $artifact.commit -or $result.probe_source_sha256 -ne $expectedProbe) { throw 'Probe output integrity failure' }
            AssertSamples $result
            $volume = AssertFixtureVolume $result
            SaveJson "$name.verified.json" @{ result = $result; verified_volume = $volume; binary_sha256 = $artifact.binary_sha256 }
        }
        $name = 'pair-{0}-storage-control' -f ($pairIndex + 1)
        if ((Hash $control) -ne $environment.control_sha256) { throw 'Control source changed' }
        $result = (Run $name 'python' @($control)) | ConvertFrom-Json
        if ($result.status -ne 'completed' -or $result.actual_pragmas.journal_mode -ne 'wal' -or $result.actual_pragmas.synchronous -ne 2) { throw 'Storage control integrity failure' }
        AssertSamples $result
        $volume = AssertFixtureVolume $result
        SaveJson "$name.verified.json" @{ result = $result; verified_volume = $volume }
    }
    SaveJson 'status.json' @{ status = 'completed exactly once'; pairs = $pairs; product_runs = 12; controls = 6; output = $outputRoot }
} catch {
    SaveJson 'status.json' @{ status = 'stopped on first failure'; error = $_.ToString(); output = $outputRoot }
    throw
} finally {
    Write-Output "Diagnostic evidence: $outputRoot"
}
