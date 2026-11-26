$ErrorActionPreference = 'Stop'
# La salida del binario es UTF-8. Si la consola usa otra pagina de codigos (por
# ejemplo ibm850), PowerShell la decodifica mal y corrompe todo lo no ASCII. Se
# fija solo para este proceso: no cambia la configuracion del usuario.
$OutputEncoding = New-Object Text.UTF8Encoding $false
[Console]::OutputEncoding = $OutputEncoding
Push-Location $PSScriptRoot
# Push-Location mueve la ubicacion de PowerShell, pero NO el directorio de
# trabajo del proceso, que es el que usan las APIs .NET y los procesos hijo.
# Sin sincronizarlo, toda ruta relativa se resuelve contra el directorio desde
# el que se lanzo el script en vez de contra esta carpeta.
$cwdPrevio = [Environment]::CurrentDirectory
[Environment]::CurrentDirectory = (Get-Location).Path
try {
    cargo build --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo build falló' }
    New-Item -ItemType Directory -Force demo-output | Out-Null
    $demo = "demo-output/episodio-$([guid]::NewGuid().ToString('N')).sqlite"
    $bin = '.\target\debug\msp.exe'
    $receipts = @(& $bin $demo import examples/episodio.jsonl | ForEach-Object { $_ | ConvertFrom-Json })
    if ($LASTEXITCODE -ne 0 -or $receipts.Count -ne 10 -or @($receipts | Where-Object { -not $_.insertado }).Count -ne 0) { throw 'Importación inesperada' }
    & $bin $demo work --lease-ms 1000 --stop-after-reserve
    if ($LASTEXITCODE -ne 3) { throw 'Falta interrupción controlada' }
    Start-Sleep -Seconds 2
    & $bin $demo work
    if ($LASTEXITCODE -ne 0) { throw 'Procesamiento falló' }
    $repeated = @(& $bin $demo import examples/episodio.jsonl | ForEach-Object { $_ | ConvertFrom-Json })
    if ($LASTEXITCODE -ne 0 -or $repeated.Count -ne 10 -or @($repeated | Where-Object insertado).Count -ne 0) { throw 'Se duplicaron eventos' }
    for ($index = 0; $index -lt 10; $index++) {
        if ($receipts[$index].instancia_id -ne $repeated[$index].instancia_id) { throw 'Cambió un ID' }
    }
    $output = & $bin $demo episode demo/episodio-1
    if ($LASTEXITCODE -ne 0) { throw 'Consulta falló' }
    $episode = $output | ConvertFrom-Json
    if ($episode.eventos.Count -ne 10 -or $episode.eventos[0].trabajos[0].reservas_totales -ne 2) { throw 'Reconstrucción inesperada' }
    foreach ($row in $episode.eventos) {
        if ($row.trabajos[0].estado -ne 'completado') { throw 'Trabajo sin completar' }
    }
    $report = "$demo.episodio.json"
    $output | Set-Content -Encoding utf8 $report
    Write-Output "Demostración verificada: 10 eventos, 10 resultados técnicos, primera reserva recuperada, mismos IDs al reimportar. Base: $demo; episodio: $report"
} finally {
    Pop-Location
    [Environment]::CurrentDirectory = $cwdPrevio
}
