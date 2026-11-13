@echo off
rem Double-click to install the Orbit Start CPU temperature helper (asks for administrator rights once).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-sensors.ps1" %*
