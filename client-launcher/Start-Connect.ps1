param([string]$Profile = 'default', [switch]$Demo)
$ErrorActionPreference = 'Stop'
if ($Profile -notmatch '^[a-z0-9_-]{1,32}$') { throw 'Invalid local profile name.' }

# Supports the source tree, dist/current.txt, and an extracted portable package.
$binaryRoot = Join-Path $PSScriptRoot 'dist'
if ((Test-Path -LiteralPath (Join-Path $PSScriptRoot 'connect.exe')) -or
    (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'serein-desktop.exe')) -or
    (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'current.txt'))) { $binaryRoot = $PSScriptRoot }
$versionFile = Join-Path $binaryRoot 'current.txt'
if (Test-Path -LiteralPath $versionFile) {
    $version = (Get-Content -LiteralPath $versionFile -Raw).Trim()
    if ($version -notmatch '^[a-f0-9]{16}$') { throw 'Invalid packaged version.' }
    $binaryRoot = Join-Path $binaryRoot (Join-Path 'versions' $version)
}
if (!(Test-Path -LiteralPath (Join-Path $binaryRoot 'connect.exe')) -and
    !(Test-Path -LiteralPath (Join-Path $binaryRoot 'serein-desktop.exe'))) {
    $binaryRoot = Join-Path $PSScriptRoot 'target/debug'
}
$appPath = Join-Path $binaryRoot 'connect.exe'
# Existing packages remain usable until the next Connect build replaces the pointer.
if (!(Test-Path -LiteralPath $appPath)) { $appPath = Join-Path $binaryRoot 'serein-desktop.exe' }
if (!(Test-Path -LiteralPath $appPath)) { throw 'Build Connect using Build.ps1 first.' }
$directoryPath = Join-Path $binaryRoot 'connect-directory.exe'
if (!(Test-Path -LiteralPath $directoryPath)) { $directoryPath = Join-Path $binaryRoot 'serein-directory.exe' }
$compiler = Join-Path $PSScriptRoot '.tools/mingw64/bin'
if (Test-Path -LiteralPath $compiler) { $env:PATH = $compiler + ';' + $env:PATH }
$directoryProcess = $null
$useLocalDirectory = $true
if (!$Demo -and (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'Configure-Directory.ps1'))) {
    . (Join-Path $PSScriptRoot 'Configure-Directory.ps1')
    if (Get-Process -Name 'connect','serein-desktop' -ErrorAction SilentlyContinue) {
        Write-Host 'Connect ar redan igang. Stang appen och starta via Starta.cmd for att aktivera den gemensamma adressboken.'
    } else {
        $settingsPath = Join-Path $env:LOCALAPPDATA ('SereinConnect/' + $Profile + '/web-preferences.json')
        $configuredDirectory = Set-ConnectDirectory -SettingsPath $settingsPath
        $useLocalDirectory = $configuredDirectory -match '^http://(127\.0\.0\.1|localhost):4747/?$'
    }
}
try {
    if (!$Demo -and $useLocalDirectory) {
        $running = $false
        try { $running = (Invoke-RestMethod -Uri 'http://127.0.0.1:4747/health' -TimeoutSec 2) -eq 'serein-directory-v1' } catch {}
        if (!$running) {
            $env:SEREIN_DIRECTORY_BIND = '127.0.0.1:4747'
            $env:SEREIN_DIRECTORY_DATA = Join-Path $env:LOCALAPPDATA 'SereinConnect/directory/records.json'
            $directoryProcess = Start-Process -FilePath $directoryPath -WorkingDirectory $PSScriptRoot -WindowStyle Hidden -PassThru
        }
    }
    $arguments = @('--profile', $Profile)
    if ($Demo) { $arguments += '--demo' }
    $application = Start-Process -FilePath $appPath -ArgumentList $arguments -WorkingDirectory $PSScriptRoot -WindowStyle Normal -PassThru
    $application.WaitForExit()
} finally {
    if ($directoryProcess -and !$directoryProcess.HasExited) { $directoryProcess.Kill() }
}
