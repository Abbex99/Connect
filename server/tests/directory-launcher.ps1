$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot '../Configure-Directory.ps1')
$root=Join-Path ([IO.Path]::GetTempPath()) ('connect-directory-test-'+[guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($root) | Out-Null
$path=Join-Path $root 'web-preferences.json'
$url=Set-ConnectDirectory -SettingsPath $path
if($url -ne 'https://mailhantering.se'){throw 'New profile default failed'}
[IO.File]::WriteAllText($path,'{"directory":"http://127.0.0.1:4747","profile":{"name":"Test"},"custom":{"nested":[1,2,3]}}')
Set-ConnectDirectory -SettingsPath $path | Out-Null
$data=Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
if($data.profile.name -ne 'Test' -or $data.custom.nested.Count -ne 3){throw 'Profile data changed'}
if(@(Get-ChildItem -LiteralPath $root -Filter '*.bak').Count -ne 1){throw 'Backup missing'}
$custom='{"directory":"https://my.example","untouched":true}'
[IO.File]::WriteAllText($path,$custom)
Set-ConnectDirectory -SettingsPath $path | Out-Null
if([IO.File]::ReadAllText($path) -ne $custom){throw 'Custom endpoint overwritten'}
[IO.File]::WriteAllText($path,'not json')
$failed=$false;try{Set-ConnectDirectory -SettingsPath $path}catch{$failed=$true}
if(!$failed -or [IO.File]::ReadAllText($path) -ne 'not json'){throw 'Corrupt preferences overwritten'}
Write-Output 'Directory launcher: new profile, migration, backup, custom endpoint and corrupt file checks passed.'
$resolved=[IO.Path]::GetFullPath($root)
if(!$resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath())+'connect-directory-test-')){throw 'Unsafe cleanup'}
Remove-Item -LiteralPath $resolved -Recurse -Force
