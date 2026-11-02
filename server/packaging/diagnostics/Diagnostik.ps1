param([string]$ConnectExe, [switch]$NoPicker)
$ErrorActionPreference = 'Stop'
$report = [System.Collections.Generic.List[string]]::new()
function Record([string]$line) { $report.Add($line); Write-Host $line }
Record 'Connect - anslutningsdiagnostik v1'
Record ('Tid UTC: ' + [DateTime]::UtcNow.ToString('yyyy-MM-dd HH:mm:ss'))
Record ('Windows-version: ' + [Environment]::OSVersion.Version.ToString())
Record ('64-bitars Windows: ' + [Environment]::Is64BitOperatingSystem)
Record 'Inga chattar, profiler, inbjudningar eller privata IP-adresser samlas in.'
if (!$ConnectExe -and !$NoPicker) {
    Write-Host 'Valj den connect.exe du anvander. Avbryt hoppar over versionskontrollen.'
    Add-Type -AssemblyName System.Windows.Forms
    $picker = New-Object System.Windows.Forms.OpenFileDialog
    $picker.Title = 'Valj din connect.exe (valfritt)'
    $picker.Filter = 'Connect (connect.exe)|connect.exe'
    if ($picker.ShowDialog() -eq 'OK') { $ConnectExe = $picker.FileName }
    $picker.Dispose()
}
if ($ConnectExe) {
    try {
        $hash = (Get-FileHash -LiteralPath $ConnectExe -Algorithm SHA256).Hash.ToLowerInvariant()
        Record ('Appens SHA256: ' + $hash)
        Record ('Matchar senaste bevarade bygge: ' + $hash.StartsWith('ebea803d2b6bbd49'))
    } catch { Record 'Appversion: kunde inte lasa vald fil.' }
} else { Record 'Appversion: ej kontrollerad.' }

foreach ($relay in @('euc1-1.relay.n0.iroh.link.', 'use1-1.relay.n0.iroh.link.', 'usw1-1.relay.n0.iroh.link.', 'aps1-1.relay.n0.iroh.link.')) {
    $tcp = $null
    try {
        $dns = [System.Net.Dns]::GetHostAddressesAsync($relay)
        if (!$dns.Wait(5000)) { Record ($relay + ' DNS: tidsgrans'); continue }
        if ($dns.Result.Count -eq 0) { Record ($relay + ' DNS: inga adresser'); continue }
        Record ($relay + ' DNS: OK')
        $tcp = New-Object System.Net.Sockets.TcpClient
        $connect = $tcp.ConnectAsync($relay, 443)
        if (!$connect.Wait(5000)) { Record ($relay + ' TCP 443: tidsgrans') }
        else { Record ($relay + ' TCP 443: OK') }
    } catch { Record ($relay + ' DNS/TCP: misslyckades (' + $_.Exception.GetBaseException().GetType().Name + ')') }
    finally { if ($tcp) { $tcp.Dispose() } }
}

Record 'Startar riktigt relachattest (hogst 90 sekunder)...'
$process = New-Object System.Diagnostics.Process
$process.StartInfo.FileName = Join-Path $PSScriptRoot 'connect-relay-test.exe'
$process.StartInfo.Arguments = 'external_relay_carries_real_chat_without_direct_ip_transport --ignored --exact --nocapture'
$process.StartInfo.UseShellExecute = $false
$process.StartInfo.CreateNoWindow = $true
$process.StartInfo.RedirectStandardOutput = $true
$process.StartInfo.RedirectStandardError = $true
$process.StartInfo.EnvironmentVariables['RUST_BACKTRACE'] = '0'
$process.StartInfo.EnvironmentVariables['RUST_LOG'] = 'off'
try {
    [void]$process.Start()
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (!$process.WaitForExit(90000)) {
        $process.Kill()
        $process.WaitForExit()
        Record 'Relachattest: TIDSGRANS'
    } else {
        $output = $stdout.Result + $stderr.Result
        if ($process.ExitCode -eq 0 -and $output.Contains('1 passed; 0 failed')) {
            Record 'Relachattest: OK - anslutning och testmeddelande via externt rela.'
        } else {
            Record ('Relachattest: MISSLYCKADES (kod ' + $process.ExitCode + ')')
            if ($output.Contains('Servern kunde inte')) { Record 'Steg: anslutningen till testvarden misslyckades.' }
            if ($output.Contains('expected room update')) { Record 'Steg: testmeddelandet kom inte fram i tid.' }
            if ($output.Contains('failed to lookup address')) { Record 'Steg: namnuppslagning misslyckades.' }
        }
    }
} catch { Record ('Relachattest: kunde inte koras (' + $_.Exception.GetBaseException().GetType().Name + ')') }
finally { $process.Dispose() }
Record 'Testet provar denna dators relavag, inte er specifika server eller inbjudan.'
Record 'Det andrar inga brandvaggs-, natverks- eller Connect-installningar.'
$destination = Join-Path $PSScriptRoot ('Connect-rapport-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.txt')
try {
    [IO.File]::WriteAllLines($destination, $report, [Text.UTF8Encoding]::new($true))
    Write-Host ('KLART. Skicka rapportfilen: ' + [IO.Path]::GetFileName($destination))
} catch {
    Write-Host 'Rapporten kunde inte sparas. Kopiera texten i detta fonster.'
    exit 1
}
