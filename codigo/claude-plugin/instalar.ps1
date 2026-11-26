$ErrorActionPreference = 'Stop'
# Instala msp y arandu en el PATH de cargo (los hooks llaman a msp por su nombre) y deja
# una copia dentro del plugin para prueba-manual.ps1. No toca la configuracion de Claude Code.
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
    # Los hooks llaman a msp por su nombre: tiene que estar en el PATH.
    cargo install --path . --locked
    if ($LASTEXITCODE -ne 0) { throw 'Fallo cargo install' }
    $destino = Join-Path $PSScriptRoot 'bin'
    New-Item -ItemType Directory -Force $destino | Out-Null
    Copy-Item 'target/release/msp.exe' (Join-Path $destino 'msp.exe') -Force
    & (Join-Path $destino 'msp.exe') --help | Select-Object -First 1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'El binario copiado no responde' }
    Write-Output "msp y arandu instalados con cargo; copia en $destino. El plugin no activa ninguna memoria por si solo."
    Write-Output "Probarlo sin instalacion global:  claude --plugin-dir `"$PSScriptRoot`""
}
finally { Pop-Location
    [Environment]::CurrentDirectory = $cwdPrevio }
