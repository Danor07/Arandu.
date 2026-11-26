$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
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
    if ($LASTEXITCODE -ne 0) { throw 'Falló cargo build' }
    New-Item -ItemType Directory -Force demo-output | Out-Null
    $db = "demo-output/recuperacion-$([guid]::NewGuid().ToString('N')).sqlite"
    $bin = '.\target\debug\msp.exe'
    $receipts = @(& $bin $db import examples/recuperacion.jsonl | ForEach-Object { $_ | ConvertFrom-Json })
    if ($LASTEXITCODE -ne 0 -or $receipts.Count -ne 8) { throw 'Importación inesperada' }
    $problem = ($receipts | Where-Object clave_evento -EQ 'demo/B/1').instancia_id
    $search = & $bin $db search calculadora 'suma enteros' --role correccion | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or $search.resultados.Count -ne 1 -or $search.resultados[0].clave_evento -ne 'demo/A/3') { throw 'A no fue encontrado o se mezclaron proyectos' }
    if ($search.sin_proyecto_en_base -ne 1) { throw 'Falta informar el evento sin proyecto' }
    $detail = & $bin $db detail calculadora $search.resultados[0].instancia_id --before 2 --after 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or ($detail.eventos.secuencia -join ',') -ne '10,20,30,40') { throw 'Secuencia incorrecta' }
    $none = & $bin $db search calculadora 'terminoausenteqzx' | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or $none.resultados.Count -ne 0) { throw 'Se esperaban cero coincidencias' }
    $raw = & $bin $db prepare calculadora $problem paquete-B 'suma enteros' --budget-bytes 700
    if ($LASTEXITCODE -ne 0) { throw 'Preparación fallida' }
    $packet = $raw | ConvertFrom-Json
    if ($packet.seleccion.Count -eq 0 -or $packet.bytes_utf8 -gt 700 -or $packet.bytes_utf8 -ne [Text.Encoding]::UTF8.GetByteCount($packet.texto_memoria)) { throw 'Presupuesto incorrecto' }
    foreach ($item in $packet.seleccion) {
        if ($item.candidato.grupo_evidencia -ne 'episodio-A') { throw 'Se incluyó B, otro proyecto o datos no elegibles' }
    }
    if (($packet.seleccion.texto_entregado -join '') -cne $packet.texto_memoria) { throw 'Texto/orden no reconstruibles' }
    # Cada invocación es un proceso nuevo: conexión cerrada y reabierta.
    $saved = & $bin $db packet calculadora paquete-B
    if ($LASTEXITCODE -ne 0 -or $saved -cne $raw) { throw 'Cambió el paquete al reabrir' }
    $repeated = & $bin $db prepare calculadora $problem paquete-B 'suma enteros' --budget-bytes 700
    if ($LASTEXITCODE -ne 0 -or $repeated -cne $raw) { throw 'Reintento no idempotente' }
    & $bin $db reindex | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Reconstrucción de índice fallida' }
    $afterRebuild = & $bin $db packet calculadora paquete-B
    if ($LASTEXITCODE -ne 0 -or $afterRebuild -cne $raw) { throw 'El índice alteró la entrega histórica' }
    $raw | Set-Content -Encoding utf8 "$db.paquete.json"
    $detail | ConvertTo-Json -Depth 30 | Set-Content -Encoding utf8 "$db.detalle.json"
    Write-Output "Trazabilidad verificada: B encuentra A; secuencia 10,20,30,40; $($packet.seleccion.Count) eventos y $($packet.bytes_utf8)/700 bytes UTF-8. Paquete idéntico tras reabrir, reintentar y reconstruir el índice. No se evaluó ninguna respuesta de una LLM. Base: $db"
} finally { Pop-Location
    [Environment]::CurrentDirectory = $cwdPrevio }
