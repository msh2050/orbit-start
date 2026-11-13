# Builds orbit-sensors.exe with the C# compiler that ships with Windows (.NET Framework 4.x).
# Needs the LibreHardwareMonitor files (0.9.6, .NET Framework build) in -Lhm.
param([string]$Lhm = "C:\Tools\LibreHardwareMonitor", [string]$Out = "$PSScriptRoot\..\target\sensors")
$ErrorActionPreference = "Stop"
$csc = "$env:WINDIR\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
New-Item -ItemType Directory -Force $Out | Out-Null
& $csc /nologo /optimize+ /target:winexe /platform:x64 "/out:$Out\orbit-sensors.exe" "/r:$Lhm\LibreHardwareMonitorLib.dll" /r:System.Core.dll "$PSScriptRoot\OrbitSensors.cs"
if ($LASTEXITCODE -ne 0) { throw "compile failed" }
# Runtime files: the library and what it depends on. The GUI parts of LibreHardwareMonitor are left out.
$skip = 'LibreHardwareMonitor.exe|Aga\.Controls|OxyPlot|TaskScheduler|\.pdb$|\.xml$'
Get-ChildItem $Lhm -File | Where-Object { $_.Name -notmatch $skip } | ForEach-Object {
    $name = if ($_.Name -eq 'LibreHardwareMonitor.exe.config') { 'orbit-sensors.exe.config' } else { $_.Name }
    Copy-Item $_.FullName (Join-Path $Out $name) -Force
}
Get-ChildItem $Out | Select-Object Name, Length
