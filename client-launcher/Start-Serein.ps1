param([string]$Profile = 'default', [switch]$Demo)
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'Start-Connect.ps1') -Profile $Profile -Demo:$Demo
