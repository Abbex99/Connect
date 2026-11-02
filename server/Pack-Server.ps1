param([string]$OutputDirectory = [Environment]::GetFolderPath('Desktop'))
$ErrorActionPreference = 'Stop'
$binary = Join-Path $PSScriptRoot 'target/debug/connect-server.exe'
$version = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.Substring(0,16).ToLowerInvariant()
$destination = Join-Path $OutputDirectory "Connect-Server-$version-Windows-x64.zip"
$archive = [IO.Compression.ZipFile]::Open($destination,[IO.Compression.ZipArchiveMode]::Create)
try {
    $files = @{ 'connect-server.exe' = $binary }
    $files['IROH-TAILSCALE-BSD3.txt'] = Join-Path $PSScriptRoot 'vendor/iroh/LICENSE-BSD3'
    foreach ($name in @('Start-Server.ps1','Starta Server.cmd','Starta Server.bat','README.md','ADRESSBOK.md','connect-server.service','SERVER-LICENSES.txt')) { $files[$name] = Join-Path $PSScriptRoot ('packaging/server/'+$name) }
    foreach ($name in @('LICENSE-MIT','LICENSE-APACHE','THIRD_PARTY_NOTICES.md')) { $files[$name] = Join-Path (Split-Path $PSScriptRoot -Parent) $name }
    foreach ($entry in $files.GetEnumerator()) { $null = [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive,$entry.Value,('Connect Server/'+$entry.Key),[IO.Compression.CompressionLevel]::Optimal) }
} finally { $archive.Dispose() }
Write-Output $destination
