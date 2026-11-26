# Adaptador de demostración: no llama a proveedores ni usa una LLM.
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
# utf8NoBOM solo existe en PowerShell 7; escribir por .NET produce los mismos
# bytes (UTF-8 sin BOM) y permite ejecutar la demo tambien en Windows PowerShell.
$utf8 = New-Object Text.UTF8Encoding $false
function Write-Utf8 { param($Path, $Text) [IO.File]::WriteAllText($Path, (($Text | Out-String -Stream) -join "`n"), $utf8) }
try {
    cargo build --offline --locked
    if ($LASTEXITCODE -ne 0) { throw 'Falló cargo build' }
    $run = "demo-output/ejecucion-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Force $run | Out-Null
    $db = "$run/memoria.sqlite"
    $bin = '.\target\debug\msp.exe'
    function Invoke-Core([string[]]$Arguments) {
        $output = & $bin $db @Arguments
        if ($LASTEXITCODE -ne 0) { throw "Falló memoria: $Arguments" }
        return ($output -join "`n")
    }
    $script:records = [Collections.Generic.List[string]]::new()
    $script:events = [Collections.Generic.List[string]]::new()
    function Write-Record($Contract) {
        $path = "$run/record-$($script:records.Count).json"
        Write-Utf8 $path ($Contract | ConvertTo-Json -Depth 40 -Compress)
        $script:records.Add($path)
        Invoke-Core @('record',$path) | Out-Null
    }
    function Capture-Event($Event) {
        $path = "$run/event-$($script:events.Count).jsonl"
        Write-Utf8 $path ($Event | ConvertTo-Json -Depth 40 -Compress)
        $script:events.Add($path)
        Invoke-Core @('import',$path) | Out-Null
    }
    function New-Event([string]$Key,[int]$Sequence,[string]$Origin,[string]$Content) {
        return @{version=1;clave_evento=$Key;grupo_evidencia='t3/B';secuencia=$Sequence;timestamp=$Sequence;origen=$Origin;rol='otro';contenido=$Content;metadatos=@{proyecto='tanda3';procedencia='simulada'};referencias=@()}
    }
    function Link-Event([string]$Attempt,[string]$Key) { Write-Record @{operacion='enlazar';intento=$Attempt;evento=$Key} }
    $receipts = (Invoke-Core @('import','examples/ejecucion.jsonl')).Split("`n") | ConvertFrom-Json
    $problem = ($receipts | Where-Object clave_evento -EQ 't3/B/problema').instancia_id
    $packet = Invoke-Core @('prepare','tanda3',"$problem",'memoria-B','suma enteros','--budget-bytes','1200') | ConvertFrom-Json
    if ($packet.bytes_utf8 -le 0 -or $packet.bytes_utf8 -gt 1200) { throw 'Paquete/presupuesto incorrecto' }
    Invoke-Core @('prepare','tanda3',"$problem",'preparado-sin-envio','suma enteros') | Out-Null
    foreach ($attempt in 'con-memoria','sin-memoria','sin-respuesta') {
        Write-Record @{operacion='intento';intento=$attempt;proyecto='tanda3';problema='t3/B/problema'}
    }
    # El sumidero es un arreglo local. Su aceptación es la frontera observada
    # simulada; no se presenta como transporte ni recepción de un proveedor.
    $sink = [Collections.Generic.List[object]]::new()
    $seq = 200
    foreach ($attempt in 'con-memoria','sin-memoria','sin-respuesta') {
        $key = "t3/B/$attempt/solicitud"
        $messages = @(@{rol='user';contenido='Calculá 2 + 3.'})
        $blocks = @()
        if ($attempt -ne 'sin-memoria') {
            $messages += @{rol='system';contenido=$packet.texto_memoria}
            $blocks = @(@{paquete='memoria-B';posicion=1;texto=$packet.texto_memoria})
        }
        $snapshot = @{version=1;llamada_id="call/$attempt";procedencia='simulada';frontera='preparada';mensajes=$messages;bloques=$blocks}
        $sink.Add(($messages | ConvertTo-Json -Depth 20 -Compress))
        $snapshot.frontera = 'envio_observado'
        $snapshot.evidencia_envio = "sumidero simulado local aceptó elemento $($sink.Count - 1); no transporte remoto"
        $event = New-Event $key $seq 'herramienta' 'Solicitud observada en adaptador simulado'
        $event.metadatos.solicitud = $snapshot
        Capture-Event $event
        Link-Event $attempt $key
        if ($blocks.Count -gt 0) { Write-Record @{operacion='exponer';clave="exposicion/$attempt";intento=$attempt;solicitud=$key;paquete='memoria-B';posicion=1} }
        if ($attempt -ne 'sin-respuesta') {
            $responseKey = "t3/B/$attempt/respuesta"
            $response = New-Event $responseKey ($seq+1) 'modelo' '5'
            $response.metadatos.respuesta = @{version=1;llamada_id="call/$attempt";procedencia='simulada';solicitud=$key}
            $response.referencias = @(@{tipo='responde_a';clave_evento=$key})
            Capture-Event $response
            Link-Event $attempt $responseKey
            # Verificador REAL ejecutado aquí. Evalúa únicamente igualdad numérica
            # con 2+3; no atribuye el resultado a la memoria ni verifica generalidad.
            $parsed = 0
            $isInteger = [int]::TryParse($response.contenido,[ref]$parsed)
            $expected = 2 + 3
            $observed = @{entero_valido=$isInteger;observado=$parsed;esperado=$expected;cumple=($isInteger -and $parsed -eq $expected)}
            $testKey = "t3/B/$attempt/verificador"
            $test = New-Event $testKey ($seq+2) 'herramienta' ($observed | ConvertTo-Json -Compress)
            $test.referencias = @(@{tipo='evalua';clave_evento=$responseKey})
            $test.uso = @{intento_clave=$attempt;tipo='prueba';metricas=@{}}
            $test.metadatos.evaluacion = @{version=1;fuente='verificador_externo';procedencia='simulada';criterio='suma-2-3';criterio_version='1';alcance='evento';objetivo=$responseKey;resultado=[int]$observed.cumple;nota='Verificador local real sobre respuesta simulada';anterior=$null;detalle=@{nombre='igualdad-entera';version='1';entrada=@{objetivo=$responseKey;contenido=$response.contenido};salida=$observed;regla=@{campo='/cumple';si_true=1;si_false=0};ejecucion='real_local'}}
            Capture-Event $test
            Write-Record @{operacion='evaluar';clave="evaluacion/$attempt";intento=$attempt;evidencia=$testKey}
        }
        $seq += 10
    }
    Write-Record @{operacion='intento';intento='turno';proyecto='tanda3';problema='t3/turno/problema'}
    foreach ($suffix in 'abre','cancela','cierra') { Link-Event 'turno' "t3/turno/$suffix" }
    $before = @{}
    foreach ($attempt in 'con-memoria','sin-memoria','sin-respuesta','turno') {
        $before[$attempt] = Invoke-Core @('attempt','tanda3',$attempt)
    }
    $with = $before['con-memoria'] | ConvertFrom-Json
    $without = $before['sin-memoria'] | ConvertFrom-Json
    $unknown = $before['sin-respuesta'] | ConvertFrom-Json
    $actors = $before['turno'] | ConvertFrom-Json
    if ($with.exposiciones.Count -ne 1 -or $with.exposiciones[0].texto_memoria -cne $packet.texto_memoria -or $with.llamadas[0].estado_respuesta -ne 'respuesta_enlazada' -or $with.evaluaciones[0].resultado -ne 1) { throw 'Intento con memoria no reconstruido' }
    if ($without.exposiciones.Count -ne 0 -or $without.evaluaciones.Count -ne 1 -or $without.evaluaciones[0].resultado -ne 1) { throw 'Intento sin memoria inválido' }
    if ($unknown.exposiciones.Count -ne 1 -or $unknown.llamadas[0].estado_respuesta -ne 'desconocida' -or $unknown.estado_evaluacion -ne 'desconocido') { throw 'Se inventó respuesta o éxito' }
    if (($actors.eventos.secuencia -join ',') -ne '10,20,30' -or $actors.eventos[1].evento.metadatos.actor -ne 'Carla' -or $actors.eventos[1].evento.metadatos.accion -ne 'cancela' -or $actors.eventos[2].evento.metadatos.actor -ne 'Juan' -or $actors.eventos[2].evento.referencias[0].clave_evento -ne 't3/turno/abre') { throw 'Autoría/relación alterada' }
    $exposures = Invoke-Core @('exposures','tanda3','--call','call/con-memoria','--packet','memoria-B') | ConvertFrom-Json
    if ($exposures.exposiciones.Count -ne 1 -or $exposures.exposiciones[0].texto_memoria -cne $packet.texto_memoria) { throw 'Consulta por llamada/paquete incorrecta' }
    $unexposed = Invoke-Core @('unexposed','tanda3')
    $unused = $unexposed | ConvertFrom-Json
    if ($unused.paquetes.Count -ne 1 -or $unused.paquetes[0].clave_evento -ne 'preparado-sin-envio') { throw 'Preparación confundida con exposición' }
    # Cada comando ya corre en otro proceso. Reimportar todos los bytes y contratos.
    $reimport = (Invoke-Core @('import','examples/ejecucion.jsonl')).Split("`n") | ConvertFrom-Json
    if (@($reimport | Where-Object insertado).Count -ne 0) { throw 'Fixture duplicado' }
    foreach ($path in $script:events) {
        $ack = Invoke-Core @('import',$path) | ConvertFrom-Json
        if ($ack.insertado) { throw 'Evento duplicado' }
    }
    foreach ($path in $script:records) { Invoke-Core @('record',$path) | Out-Null }
    foreach ($attempt in $before.Keys) {
        $after = Invoke-Core @('attempt','tanda3',$attempt)
        if ($after -cne $before[$attempt]) { throw "Reconstrucción cambió: $attempt" }
        Write-Utf8 "$run/$attempt.json" $after
    }
    Write-Utf8 "$run/sin-exposicion.json" $unexposed
    Write-Utf8 "$run/paquete.json" ($packet | ConvertTo-Json -Depth 40)
    Write-Output "Tanda 3 verificada: memoria exacta ($($packet.bytes_utf8) bytes), intento sin memoria, paquete sin exposición, envío sin respuesta/evaluación, autoría Carla/Juan y replay sin duplicados. Solicitudes/respuestas simuladas; dos verificaciones locales reales de suma-2-3. No demuestra aprendizaje, uso causal ni mejora de una LLM. Salida: $run"
} finally { Pop-Location
    [Environment]::CurrentDirectory = $cwdPrevio }
