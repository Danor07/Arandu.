$ErrorActionPreference = 'Stop'
# Instala el binario dentro del plugin local. No toca la configuracion global de
# Claude Code ni instala nada en el sistema: el plugin se carga con --plugin-dir.
Push-Location (Join-Path $PSScriptRoot '..')
# Push-Location mueve la ubicacion de PowerShell, pero NO el directorio de
# trabajo del proceso, que es el que usan las APIs .NET y los procesos hijo.
# Sin sincronizarlo, toda ruta relativa se resuelve contra el directorio desde
# el que se lanzo el script en vez de contra esta carpeta.
$cwdPrevio = [Environment]::CurrentDirectory
[Environment]::CurrentDirectory = (Get-Location).Path
try {
    cargo build --locked --release
    if ($LASTEXITCODE -ne 0) { throw 'Fallo cargo build --release' }
    $destino = Join-Path $PSScriptRoot 'bin'
    New-Item -ItemType Directory -Force $destino | Out-Null
    Copy-Item 'target/release/msp.exe' (Join-Path $destino 'msp.exe') -Force
    & (Join-Path $destino 'msp.exe') --help | Select-Object -First 1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'El binario copiado no responde' }
    Write-Output "Binario instalado en $destino. El plugin no activa ninguna memoria por si solo."
    Write-Output "Probarlo sin instalacion global:  claude --plugin-dir `"$PSScriptRoot`""
}
finally { Pop-Location
    [Environment]::CurrentDirectory = $cwdPrevio }
