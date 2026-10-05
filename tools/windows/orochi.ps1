# Windows entry point: the Unix runtime, agents and desktop all live in WSL2.
param()
# Parse only the launcher's prefix. PowerShell's advanced parameter binder can discard
# unknown named CLI flags, so leave Orochi's remaining arguments completely untouched.
$Setup = $false
$Desktop = $false
$Distribution = 'Ubuntu-24.04'
$WorkingDirectory = $null
$OrochiArgs = @()
:parseArgs for ($index = 0; $index -lt $args.Count; $index++) {
    switch ([string]$args[$index]) {
        '-Setup' { $Setup = $true; continue parseArgs }
        '-Desktop' { $Desktop = $true; continue parseArgs }
        '-Distribution' {
            if (++$index -ge $args.Count) { throw '-Distribution requires a name.' }
            $Distribution = [string]$args[$index]; continue parseArgs
        }
        '-WorkingDirectory' {
            if (++$index -ge $args.Count) { throw '-WorkingDirectory requires a path.' }
            $WorkingDirectory = [string]$args[$index]; continue parseArgs
        }
        '-OrochiArgs' {
            if (++$index -ge $args.Count) { throw '-OrochiArgs requires an argument array.' }
            $OrochiArgs = @($args[$index]); break
        }
        '--' {
            if ($index + 1 -lt $args.Count) { $OrochiArgs = @($args[($index + 1)..($args.Count - 1)]) }
            break
        }
        default { $OrochiArgs = @($args[$index..($args.Count - 1)]); break }
    }
    break
}
$ErrorActionPreference = 'Stop'
function Invoke-Wsl([string[]]$Arguments) {
    & wsl.exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "WSL command failed ($LASTEXITCODE)." }
}
if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
    throw 'WSL2 is required. Run wsl --install from administrator PowerShell, reboot, then run this script with -Setup.'
}
if ($Setup) {
    $installed = @(& wsl.exe --list --quiet) -join "`n"
    $installed = $installed.Replace([string][char]0, '')
    if ($LASTEXITCODE -ne 0 -or $installed.Split("`n").Trim() -notcontains $Distribution) {
        Invoke-Wsl -Arguments @('--install', '--distribution', $Distribution, '--no-launch')
        Write-Host 'If Windows requests a reboot, reboot and rerun -Setup. Complete Ubuntu user creation when prompted.'
    }
    Invoke-Wsl -Arguments @('--set-version', $Distribution, '2')
    $repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
    $linuxRepo = (& wsl.exe -d $Distribution --exec wslpath -a -u $repo | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $linuxRepo) { throw 'Could not resolve the source checkout in WSL.' }
    $installer = "$linuxRepo/tools/linux/install.sh"
    $flags = @()
    if ($Desktop) { $flags += '--desktop' }
    Invoke-Wsl -Arguments (@('-d', $Distribution, '--exec', 'bash', $installer) + $flags)
    return
}
if (-not $WorkingDirectory) {
    $WorkingDirectory = (& wsl.exe -d $Distribution --exec wslpath -a -u (Get-Location).Path | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Use -WorkingDirectory with a Linux path, for example /home/me/project.' }
}
# Convert only arguments whose values are paths; never reinterpret task text as shell code.
$arguments = @()
if ($null -ne $OrochiArgs) { $arguments = @($OrochiArgs) }
for ($i = 0; $i -lt $arguments.Count - 1; $i++) {
    if ($arguments[$i] -in @('--cwd', '-C', '--config', '--data-dir') -and $arguments[$i + 1] -match '^[A-Za-z]:[\\/]') {
        $arguments[$i + 1] = (& wsl.exe -d $Distribution --exec wslpath -a -u $arguments[$i + 1] | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw 'Could not convert a Windows path to WSL.' }
    }
}
$binary = 'orochi'
if ($Desktop) { $binary = 'orochi-desktop' }
Invoke-Wsl -Arguments (@('-d', $Distribution, '--cd', $WorkingDirectory, '--exec', 'bash', '-c',
    'exec "$HOME/.local/bin/$1" "${@:2}"', 'orochi-launch', $binary) + $arguments)
