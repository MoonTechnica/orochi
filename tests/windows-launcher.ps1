# Portable PowerShell tests: exercise command construction without requiring or changing WSL.
$ErrorActionPreference = 'Stop'
$launcher = Join-Path $PSScriptRoot '../tools/windows/orochi.ps1'
$global:calls = [System.Collections.Generic.List[object]]::new()
$global:failWsl = $false
$global:installedWsl = $true
function global:wsl.exe {
    $global:calls.Add(@($args))
    $global:LASTEXITCODE = 0
    if ($global:failWsl) { $global:LASTEXITCODE = 1; return }
    if ($args -contains 'wslpath') { '/mnt/c/converted path' }
    elseif ($args -contains '--list' -and $global:installedWsl) { 'Ubuntu-24.04' }
}
function Assert-Equal($actual, $expected) {
    if (($actual | ConvertTo-Json -Compress) -ne ($expected | ConvertTo-Json -Compress)) {
        throw "Expected $(ConvertTo-Json -Compress $expected); got $(ConvertTo-Json -Compress $actual)"
    }
}
$task = 'review; $(touch /tmp/should-not-exist) "quoted text"'
& $launcher -WorkingDirectory '/home/test/project with spaces' -OrochiArgs @('--cwd', 'C:\repo with spaces', $task)
$last = $global:calls[$global:calls.Count - 1]
Assert-Equal $last[-1] $task
Assert-Equal $last[-2] '/mnt/c/converted path'
Assert-Equal $last[3] '/home/test/project with spaces'
Assert-Equal $last[5] 'bash'
Assert-Equal $last[9] 'orochi'
& $launcher -WorkingDirectory '/home/test/project' --permission allow 'A task'
Assert-Equal $global:calls[$global:calls.Count - 1][-3..-1] @('--permission', 'allow', 'A task')
& $launcher -Desktop -WorkingDirectory '/home/test/project'
Assert-Equal $global:calls[$global:calls.Count - 1][-1] 'orochi-desktop'
& $launcher -Setup
Assert-Equal ($global:calls | Where-Object { $_ -contains '--set-version' } | Select-Object -Last 1)[-1] '2'
Assert-Equal ($global:calls | Where-Object { $_ -contains 'bash' } | Select-Object -Last 1)[-1] '/mnt/c/converted path/tools/linux/install.sh'
$global:installedWsl = $false
& $launcher -Setup -Desktop
Assert-Equal ($global:calls | Where-Object { $_ -contains '--install' } | Select-Object -Last 1)[1] '--distribution'
Assert-Equal ($global:calls | Where-Object { $_ -contains 'bash' } | Select-Object -Last 1)[-1] '--desktop'
$global:failWsl = $true
$threw = $false
try { & $launcher -WorkingDirectory '/home/test/project' } catch { $threw = $true }
if (-not $threw) { throw 'A failed WSL launch must fail the launcher too.' }
Write-Host 'Windows launcher tests passed'
