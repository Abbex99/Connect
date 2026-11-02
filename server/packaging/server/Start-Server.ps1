param([switch]$Pair)
$ErrorActionPreference = 'Stop'
$binary = Join-Path $PSScriptRoot 'connect-server.exe'
$data = Join-Path $PSScriptRoot 'data'
if ($Pair) {
    Write-Host 'Detta aterkallar tidigare agar- och administratorroller.'
    $answer = Read-Host 'Skriv ATERSTALL for att bekrafta'
    if ($answer -cne 'ATERSTALL') { exit 1 }
    & $binary run --data $data --pair
} else {
    & $binary manage --data $data --open
}
if ($LASTEXITCODE -ne 0) { throw 'Servern kunde inte starta. Las felmeddelandet ovan.' }
