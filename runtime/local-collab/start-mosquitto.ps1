# Mosquitto 启动器（由 plant-collab-monitor/scripts/local-remote-collab-setup.ps1 生成）· 长驻进程，在自己的终端里跑
$ErrorActionPreference = 'Stop'
$candidates = @('C:\Program Files\mosquitto\mosquitto.exe', 'C:\Program Files\mosquitto\mosquitto.exe')
$exe = $null
foreach ($c in $candidates) { if ((Get-Command $c -ErrorAction SilentlyContinue) -or (Test-Path $c)) { $exe = $c; break } }
if (-not $exe) { Write-Error '找不到 mosquitto.exe：winget install --id EclipseFoundation.Mosquitto -e'; exit 2 }
Write-Host "[mosquitto] $exe -c D:\work\plant-code\plant-model-gen\runtime\local-collab\mosquitto.conf -v"
& $exe -c 'D:\work\plant-code\plant-model-gen\runtime\local-collab\mosquitto.conf' -v