# Installs (or with -Uninstall removes) the Orbit Start sensor helper.
#
# The helper reads the CPU temperature through LibreHardwareMonitorLib and the
# PawnIO driver, which needs administrator rights. It is installed to
#   C:\Program Files\OrbitStart\Sensors        (only administrators can change it)
# and started at sign-in by a scheduled task with highest privileges, so there
# is no prompt after this one-time setup. It has no window and no tray icon.
param([switch]$Uninstall)

$ErrorActionPreference = "Stop"
$taskName = "Orbit Start Sensors"
$target = Join-Path $env:ProgramFiles "OrbitStart\Sensors"

# Ask for administrator rights once.
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) {
    $argList = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`""
    if ($Uninstall) { $argList += " -Uninstall" }
    Start-Process powershell.exe -Verb RunAs -ArgumentList $argList
    exit
}

Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue | ForEach-Object {
    Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
}
Get-Process orbit-sensors -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

if ($Uninstall) {
    Remove-Item $target -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $env:ProgramData "OrbitStart") -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "Orbit Start sensor helper removed." -ForegroundColor Green
    Start-Sleep 3
    exit
}

# Files: next to this script (release download) or in ..\target\sensors (source build).
$source = @("$PSScriptRoot\sensors", "$PSScriptRoot\..\target\sensors") | Where-Object { Test-Path "$_\orbit-sensors.exe" } | Select-Object -First 1
if (-not $source) { Write-Host "orbit-sensors.exe not found. Build it first with build.ps1." -ForegroundColor Red; Start-Sleep 8; exit 1 }

New-Item -ItemType Directory -Force $target | Out-Null
Copy-Item "$source\*" $target -Recurse -Force
New-Item -ItemType Directory -Force (Join-Path $env:ProgramData "OrbitStart") | Out-Null

$user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$action = New-ScheduledTaskAction -Execute (Join-Path $target "orbit-sensors.exe")
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -Priority 7 -StartWhenAvailable
Register-ScheduledTask -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description "Reads the CPU temperature for Orbit Start" | Out-Null
Start-ScheduledTask -TaskName $taskName

Start-Sleep 4
$file = Join-Path $env:ProgramData "OrbitStart\sensors.txt"
if (Test-Path $file) {
    Write-Host "Installed. Current reading:" -ForegroundColor Green
    Get-Content $file
} else {
    Write-Host "Installed, but no reading yet. Is the PawnIO driver installed?" -ForegroundColor Yellow
}
Start-Sleep 6
