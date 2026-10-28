[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repo = Split-Path $PSScriptRoot -Parent
$scratch = Join-Path $repo ('.agent-runs/workspace-lifecycle/layout-spike/' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $scratch | Out-Null
$scratch = (Resolve-Path -LiteralPath $scratch).Path
$results = [System.Collections.Generic.List[object]]::new()
function Assert-True($Condition, $Message) {
    if (-not $Condition) { throw $Message }
}
function Invoke-Git([string]$At, [string[]]$Arguments) {
    $output = & git -C $At @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $Arguments failed: $output" }
    return $output
}
function Remove-FixturePath([string]$Path) {
    $absolute = [IO.Path]::GetFullPath($Path)
    Assert-True ($absolute.StartsWith($scratch + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) "Removal outside fixture: $absolute"
    if (Test-Path -LiteralPath $absolute) {
        $item = Get-Item -LiteralPath $absolute -Force
        Assert-True (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) "Refuse reparse point: $absolute"
        Remove-Item -LiteralPath $absolute -Recurse -Force
    }
}
try {
    foreach ($layout in @('nested', 'external')) {
        foreach ($long in @($false, $true)) {
            $caseName = $layout + $(if ($long) { '-long' } else { '-normal' })
            $case = Join-Path $scratch $caseName
            $origin = Join-Path $case 'origin'
            New-Item -ItemType Directory -Path (Join-Path $origin 'src') -Force | Out-Null
            [IO.File]::WriteAllText((Join-Path $origin '.gitignore'), "/target/`n/.meshloop/`n/unknown-cache/`n")
            [IO.File]::WriteAllText((Join-Path $origin 'Cargo.toml'), "[package]`nname = `"layout_fixture`"`nversion = `"0.1.0`"`nedition = `"2021`"`n`n[workspace]`n")
            $source = "fn main() { println!(`"layout-spike-marker`"); }`n"
            [IO.File]::WriteAllText((Join-Path $origin 'src/main.rs'), $source)
            [IO.File]::WriteAllText((Join-Path $origin 'Cargo.lock'), "version = 4`n`n[[package]]`nname = `"layout_fixture`"`nversion = `"0.1.0`"`n")
            $null = Invoke-Git $origin @('init', '-q')
            $null = Invoke-Git $origin @('config', 'core.longpaths', 'true')
            $null = Invoke-Git $origin @('config', 'core.autocrlf', 'false')
            $null = Invoke-Git $origin @('add', '.')
            $null = Invoke-Git $origin @('-c', 'user.name=Layout Fixture', '-c', 'user.email=layout@example.invalid', 'commit', '-qm', 'fixture')
            $before = @(Invoke-Git $origin @('worktree', 'list', '--porcelain')) -join "`n"
            $root = if ($layout -eq 'nested') { Join-Path $origin '.meshloop/worktrees' } else { Join-Path $case 'external-worktrees' }
            if ($long) {
                while ($root.Length -lt 270) { $root = Join-Path $root 'representative-root-segment' }
            }
            $worktree = Join-Path $root 'run-0001'
            $addOutput = & git -C $origin worktree add --detach $worktree HEAD 2>&1
            $addExit = $LASTEXITCODE
            if ($addExit -ne 0) {
                Assert-True $long "Normal worktree add failed: $addOutput"
                $afterFailure = @(Invoke-Git $origin @('worktree', 'list', '--porcelain')) -join "`n"
                Assert-True ($before -eq $afterFailure) 'Failed add left a registration; inspect fixture manually'
                $results.Add([ordered]@{ case = $caseName; worktreePathLength = $worktree.Length; addExit = $addExit; limitation = ($addOutput -join "`n"); registrationsRestored = $true })
                Remove-FixturePath $case
                continue
            }
            $build = & cargo build --offline --locked --manifest-path (Join-Path $worktree 'Cargo.toml') 2>&1
            $buildExit = $LASTEXITCODE
            Assert-True ($buildExit -eq 0) "Cargo failed in $caseName : $build"
            $target = Join-Path $worktree 'target'
            Assert-True (Test-Path -LiteralPath $target) 'Expected local Cargo target'
            $ignored = @(Invoke-Git $worktree @('status', '--porcelain', '--ignored')) -join "`n"
            Assert-True ($ignored -match '!! target/') 'Generated target must be ignored'
            $parentStatus = @(Invoke-Git $origin @('status', '--porcelain', '--untracked-files=all')) -join "`n"
            Assert-True ([string]::IsNullOrWhiteSpace($parentStatus)) 'Parent worktree polluted'
            Push-Location $origin
            try {
                $files = @(& rg --files --hidden -g '!.git/**')
                Assert-True ($LASTEXITCODE -eq 0) 'rg file discovery failed'
                Assert-True (-not ($files -match '\.meshloop|external-worktrees')) 'Recursive discovery entered ignored worktree'
                $matches = @(& rg -l 'layout-spike-marker' .)
                Assert-True ($LASTEXITCODE -eq 0 -and $matches.Count -eq 1) 'rg recursively searched a worktree'
                $metadata = & cargo metadata --offline --locked --no-deps --format-version 1 2>&1
                Assert-True ($LASTEXITCODE -eq 0) "Cargo metadata failed: $metadata"
                $parsed = ($metadata -join "`n") | ConvertFrom-Json
                Assert-True ($parsed.packages.Count -eq 1 -and $parsed.workspace_members.Count -eq 1) 'Cargo recursively discovered fixture worktree'
            } finally { Pop-Location }
            # Negative controls: classification blocks cleanup; Git is not asked to delete them.
            $tracked = Join-Path $worktree 'src/main.rs'
            [IO.File]::WriteAllText($tracked, $source + '// dirty control')
            $unknown = Join-Path $worktree 'unknown.txt'
            [IO.File]::WriteAllText($unknown, 'untracked control')
            $ignoredUnknown = Join-Path $worktree 'unknown-cache/keep.txt'
            New-Item -ItemType Directory -Path (Split-Path $ignoredUnknown) | Out-Null
            [IO.File]::WriteAllText($ignoredUnknown, 'unknown ignored control')
            $negative = @(Invoke-Git $worktree @('status', '--porcelain', '--ignored', '--untracked-files=all')) -join "`n"
            Assert-True ($negative -match ' M src/main.rs' -and $negative -match '\?\? unknown.txt' -and $negative -match '!! unknown-cache/keep.txt') 'Negative controls were not detected'
            Assert-True ((Get-Content -LiteralPath $ignoredUnknown -Raw) -eq 'unknown ignored control') 'Ignored control changed'
            Assert-True (Test-Path -LiteralPath $target) 'Generated output removed before classification'
            # Only explicit fixture controls are restored/removed; no git clean/reset/force.
            [IO.File]::WriteAllText($tracked, $source)
            Remove-FixturePath $unknown
            Remove-FixturePath (Split-Path $ignoredUnknown)
            Remove-FixturePath $target
            Assert-True ([string]::IsNullOrWhiteSpace((@(Invoke-Git $worktree @('status', '--porcelain', '--ignored')) -join "`n"))) 'Unexpected file blocks teardown'
            $null = Invoke-Git $origin @('worktree', 'remove', $worktree)
            $after = @(Invoke-Git $origin @('worktree', 'list', '--porcelain')) -join "`n"
            Assert-True ($before -eq $after) 'Worktree registration not restored'
            Assert-True ([string]::IsNullOrWhiteSpace((@(Invoke-Git $origin @('status', '--porcelain')) -join "`n"))) 'Parent residue after teardown'
            $results.Add([ordered]@{ case = $caseName; worktreePathLength = $worktree.Length; rustSourcePathLength = $tracked.Length; buildExit = $buildExit; parentClean = $true; discoveryExcluded = $true; negativeControlsPreserved = $true; normalRemoval = $true; registrationsRestored = $true })
            Remove-FixturePath $case
        }
    }
    $report = [ordered]@{ platform = [Environment]::OSVersion.VersionString; powershell = $PSVersionTable.PSVersion.ToString(); git = (& git --version); cargo = (& cargo --version); rustc = (& rustc --version); cases = $results; fixtureReport = [IO.Path]::GetRelativePath($repo, (Join-Path $scratch 'report.json')); fixtureTeardown = 'all disposable repositories removed'; observationAssertions = 'passed'; defaultPlacementAcceptance = 'BLOCKED pending human decision on measured Windows limits' }
    $report | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $scratch 'report.json')
    $report | ConvertTo-Json -Depth 5
} catch {
    Write-Error "Fixture failed; retained for inspection at $scratch . No production worktrees were touched. $_"
    exit 1
}
