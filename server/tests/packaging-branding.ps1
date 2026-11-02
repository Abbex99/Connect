$ErrorActionPreference = 'Stop'
$sourceRoot = Split-Path $PSScriptRoot -Parent
$tempRoot = Join-Path ([IO.Path]::GetTempPath()) ('connect-branding-test-' + [guid]::NewGuid().ToString('N'))
$script:launches = [Collections.Generic.List[object]]::new()

# These stubs ensure launcher tests never open an app or stop a real process.
function Start-Process {
    param($FilePath, $ArgumentList, $WorkingDirectory, $WindowStyle, [switch]$PassThru)
    $script:launches.Add([pscustomobject]@{ Path = $FilePath; Arguments = $ArgumentList; WindowStyle = $WindowStyle })
    $result = [pscustomobject]@{ HasExited = $true }
    $result | Add-Member -MemberType ScriptMethod -Name WaitForExit -Value {}
    return $result
}
function Invoke-RestMethod { param($Uri, $TimeoutSec) return 'not-running-test-double' }
function Assert-True($Condition, $Message) { if (!$Condition) { throw $Message } }

try {
    New-Item -ItemType Directory -Path $tempRoot | Out-Null
    foreach ($name in @('Build.ps1', 'Pack-Portable.ps1', 'Start-Connect.ps1', 'Start-Serein.ps1')) {
        $tokens = $null
        $parseErrors = $null
        $null = [Management.Automation.Language.Parser]::ParseFile((Join-Path $sourceRoot $name), [ref]$tokens, [ref]$parseErrors)
        Assert-True ($parseErrors.Count -eq 0) "PowerShell parse error: $name"
    }
    $config = Get-Content -LiteralPath (Join-Path $sourceRoot 'desktop/tauri.conf.json') -Raw | ConvertFrom-Json
    Assert-True ($config.productName -eq 'Connect' -and $config.app.windows[0].title -eq 'Connect') 'Window branding differs.'
    Assert-True ($config.identifier -eq 'se.serein.connect') 'Existing application identity changed.'

    $fixture = Join-Path $tempRoot 'source'
    New-Item -ItemType Directory -Path $fixture | Out-Null
    foreach ($name in @('Start-Connect.ps1', 'Start-Serein.ps1', 'Pack-Portable.ps1')) {
        Copy-Item -LiteralPath (Join-Path $sourceRoot $name) -Destination $fixture
    }
    $appBytes = [Text.Encoding]::UTF8.GetBytes('synthetic Connect binary; never executed')
    $version = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($appBytes)).Substring(0,16).ToLowerInvariant()
    $package = Join-Path $fixture "dist/versions/$version"
    New-Item -ItemType Directory -Path $package -Force | Out-Null
    [IO.File]::WriteAllBytes((Join-Path $package 'connect.exe'), $appBytes)
    $files = @('connect-directory.exe', 'WebView2Loader.dll', 'README.md', 'LICENSE-APACHE', 'LICENSE-MIT', 'THIRD_PARTY_NOTICES.md', 'Inter-OFL.txt', 'IROH-TAILSCALE-BSD3.txt', 'LUCIDE-LICENSE.txt', 'WASAPI-LICENSE.txt', 'CONNECT-MEDIA-FILES-LICENSES.txt')
    foreach ($name in $files) { [IO.File]::WriteAllText((Join-Path $package $name), "synthetic test file: $name") }
    foreach ($name in @('Start-Connect.ps1', 'Start-Serein.ps1', 'Starta.cmd')) {
        Copy-Item -LiteralPath (Join-Path $sourceRoot $name) -Destination $package
    }
    [IO.File]::WriteAllText((Join-Path $fixture 'dist/current.txt'), $version)

    & (Join-Path $fixture 'Start-Connect.ps1') -Profile 'brand-test'
    Assert-True ($script:launches.Count -eq 2) 'Expected a directory helper and the app.'
    Assert-True ($script:launches[0].Path -eq (Join-Path $package 'connect-directory.exe')) 'Packaged directory name differs.'
    Assert-True ($script:launches[0].WindowStyle -eq 'Hidden') 'Directory helper must remain hidden.'
    Assert-True ($script:launches[1].Path -eq (Join-Path $package 'connect.exe')) 'App executable name differs.'
    Assert-True (($script:launches[1].Arguments -join ' ') -eq '--profile brand-test') 'Profile arguments were lost.'
    & (Join-Path $fixture 'Start-Serein.ps1') -Profile 'compat-test' -Demo
    Assert-True (($script:launches[2].Arguments -join ' ') -eq '--profile compat-test --demo') 'Compatibility launcher lost arguments.'
    Copy-Item -LiteralPath (Join-Path $fixture 'Start-Connect.ps1') -Destination (Join-Path $fixture 'dist')
    & (Join-Path $fixture 'dist/Start-Connect.ps1') -Demo
    Assert-True ($script:launches[3].Path -eq (Join-Path $package 'connect.exe')) 'Dist-root launcher missed current version.'
    & (Join-Path $package 'Start-Connect.ps1') -Demo
    Assert-True ($script:launches[4].Path -eq (Join-Path $package 'connect.exe')) 'Portable launcher missed its sibling app.'

    $legacy = Join-Path $tempRoot 'legacy'
    New-Item -ItemType Directory -Path $legacy | Out-Null
    Copy-Item -LiteralPath (Join-Path $fixture 'Start-Connect.ps1') -Destination $legacy
    [IO.File]::WriteAllText((Join-Path $legacy 'serein-desktop.exe'), 'synthetic legacy binary; never executed')
    & (Join-Path $legacy 'Start-Connect.ps1') -Demo
    Assert-True ($script:launches[5].Path -eq (Join-Path $legacy 'serein-desktop.exe')) 'Legacy package fallback stopped working.'

    & (Join-Path $fixture 'Pack-Portable.ps1') -OutputDirectory $tempRoot | Out-Null
    $zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $tempRoot "Connect-$version-Windows-x64.zip"))
    try {
        Assert-True ($zip.Entries.Count -eq 16) 'Unexpected portable package file count.'
        foreach ($entry in $zip.Entries) { Assert-True ($entry.FullName.StartsWith('Connect/')) 'Archive folder is not Connect.' }
        Assert-True ($null -ne $zip.GetEntry('Connect/Start-Serein.ps1')) 'Compatibility launcher missing from package.'
        $reader = [IO.StreamReader]::new($zip.GetEntry('Connect/LAS-MIG-FORST.txt').Open())
        try { $instructions = $reader.ReadToEnd() } finally { $reader.Dispose() }
        Assert-True ($instructions.Contains('connect://join/') -and $instructions.Contains('Hämtade filer/Connect')) 'Portable instructions use old product paths.'
    } finally { $zip.Dispose() }
    Write-Output 'Connect branding, launcher compatibility, and portable packaging checks passed (no processes launched).'
} finally {
    $resolved = [IO.Path]::GetFullPath($tempRoot)
    $expectedPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()) + 'connect-branding-test-'
    if (!$resolved.StartsWith($expectedPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe test cleanup path.' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
