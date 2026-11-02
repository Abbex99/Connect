@echo off
title Connect Server
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start-Server.ps1"
if errorlevel 1 pause
