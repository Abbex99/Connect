# Shared by the portable launcher and isolated packaging tests.
function Set-ConnectDirectory {
    param([string]$SettingsPath, [string]$DirectoryUrl = 'https://mailhantering.se')
    $uri = [Uri]$DirectoryUrl
    if (!$uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or $uri.UserInfo -or $uri.Query -or $uri.Fragment) { throw 'Ogiltig adressbok. Anvand HTTPS.' }
    $exists = Test-Path -LiteralPath $SettingsPath
    if ($exists) {
        if ((Get-Item -LiteralPath $SettingsPath).Length -gt 128MB) { throw 'Profilinstallningarna ar for stora for automatisk uppdatering.' }
        $settings = [IO.File]::ReadAllText($SettingsPath) | ConvertFrom-Json
        if ($null -eq $settings -or $settings -is [Array] -or $settings -is [string]) { throw 'Profilinstallningarna kunde inte lasas.' }
        $old = [string]$settings.directory
        if ($old -and $old -notin @('http://127.0.0.1:4747','http://127.0.0.1:4747/','http://localhost:4747','http://localhost:4747/')) { return $old }
    } else { $settings = [pscustomobject]@{} }
    $settings | Add-Member -NotePropertyName directory -NotePropertyValue $DirectoryUrl -Force
    $parent = Split-Path -Parent $SettingsPath
    [IO.Directory]::CreateDirectory($parent) | Out-Null
    $temp = Join-Path $parent ('preferences-' + [guid]::NewGuid().ToString('N') + '.tmp')
    [IO.File]::WriteAllText($temp,($settings | ConvertTo-Json -Depth 100 -Compress),[Text.UTF8Encoding]::new($false))
    if ($exists) {
        $backup = $SettingsPath + '.before-online-' + [guid]::NewGuid().ToString('N') + '.bak'
        [IO.File]::Replace($temp,$SettingsPath,$backup)
    } else { [IO.File]::Move($temp,$SettingsPath) }
    return $DirectoryUrl
}
