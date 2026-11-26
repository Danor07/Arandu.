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

    # DATOS SINTETICOS ETIQUETADOS. Los hooks se reproducen con los campos
    # documentados de Claude Code 2.1.270; el verificador si se ejecuta de
    # verdad, pero sobre datos inventados. Nada de esto mide una LLM real.
    New-Item -ItemType Directory -Force demo-output | Out-Null
    $run = "demo-output/medicion-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Force $run | Out-Null
    $root = (Resolve-Path $run).Path
    $datos = Join-Path $root 'datos de medicion'
    $proyecto = Join-Path $root 'proyecto ñ'
    New-Item -ItemType Directory -Force $datos, $proyecto, (Join-Path $root 'snapshots'), (Join-Path $root 'contratos') | Out-Null
    $env:MSP_RAIZ = $datos
    $bin = '.\target\debug\msp.exe'
    $binAbs = (Resolve-Path $bin).Path
    $inst = 'demo-medicion'
    $utf8 = New-Object Text.UTF8Encoding $false
    $paso = 0

    function Control {
        param([string[]]$Argumentos)
        $salida = & $bin --control @Argumentos
        if ($LASTEXITCODE -ne 0) { throw "Control falló: $($Argumentos -join ' ')" }
        if ($salida) { return ($salida | ConvertFrom-Json) }
        return $null
    }
    function Contrato {
        param([string]$Nombre, [hashtable]$Cuerpo)
        $ruta = Join-Path $root "contratos/$Nombre.json"
        [IO.File]::WriteAllText($ruta, ($Cuerpo | ConvertTo-Json -Depth 20 -Compress), $utf8)
        return $ruta
    }
    function Hook {
        param([hashtable]$Payload)
        $script:paso++
        $n = '{0:d2}' -f $script:paso
        $entrada = Join-Path $run "hook-$n.json"
        $informe = Join-Path $run "informe-$n.json"
        $contexto = Join-Path $run "contexto-$n.txt"
        [IO.File]::WriteAllText($entrada, ($Payload | ConvertTo-Json -Depth 20 -Compress), $utf8)
        & cmd /c ('"{0}" --hook --instalacion {1} --informe "{2}" --sin-emitir < "{3}" > "{4}"' -f
            $binAbs, $inst, $informe, $entrada, $contexto)
        if ($LASTEXITCODE -ne 0) { throw "Hook falló: $($Payload.hook_event_name)" }
        $texto = if (Test-Path $contexto) { $utf8.GetString([IO.File]::ReadAllBytes($contexto)) } else { '' }
        return [pscustomobject]@{
            informe = (Get-Content -Raw $informe | ConvertFrom-Json)
            bytes   = [Text.Encoding]::UTF8.GetByteCount($texto)
        }
    }
    function Prompt {
        param($sesion, $id, $texto)
        @{ hook_event_name = 'UserPromptSubmit'; session_id = $sesion; prompt_id = $id
           cwd = $proyecto; transcript_path = (Join-Path $proyecto 't.jsonl')
           permission_mode = 'default'; prompt = $texto }
    }
    function Herramienta {
        param($sesion, $id, $uso, $salida)
        @{ hook_event_name = 'PostToolUse'; session_id = $sesion; prompt_id = $id
           tool_use_id = $uso; tool_name = 'Bash'; cwd = $proyecto
           transcript_path = (Join-Path $proyecto 't.jsonl'); permission_mode = 'default'
           tool_input = @{ command = 'cargo test' }; tool_response = $salida }
    }

    # 1. Memoria dedicada, proyecto y tarea con criterios versionados.
    $memoria = (Control @('memoria-crear', '--nombre', 'Memoria de medicion')).memoria_id
    $proyectoClave = (Control @('proyecto-registrar', '--raiz', $proyecto)).raiz_normalizada
    $criterios = @(
        @{ id = 'c-tests'; version = '1'; descripcion = 'la suite pasa'; obligatorio = $true },
        @{ id = 'c-docs'; version = '1'; descripcion = 'README actualizado'; obligatorio = $false })
    Control @('tarea-registrar', '--memoria', $memoria, '--contrato', (Contrato 'crear' @{
        operacion = 'tarea_crear'; tarea = 'demo/login'; proyecto = $proyectoClave
        definicion = 'demo/login/def/1'; descripcion = 'Arreglar el login (datos sinteticos)'
        criterios = $criterios })) | Out-Null
    Control @('tarea-registrar', '--memoria', $memoria, '--contrato', (Contrato 'inicio' @{
        operacion = 'tarea_marcar'; clave = 'demo/login/inicio'; tarea = 'demo/login'
        tipo = 'inicio'; detalle = @{ nota = 'arranca la ventana' } })) | Out-Null

    # 2. La misma tarea a lo largo de dos chats distintos.
    foreach ($chat in 'chat-1', 'chat-2') {
        Control @('sesion-usar', '--instalacion', $inst, '--sesion', $chat, '--cwd', $proyecto, '--memoria', $memoria) | Out-Null
        Control @('sesion-tarea', '--instalacion', $inst, '--sesion', $chat, '--tarea', 'demo/login', '--definicion-version', '1') | Out-Null
    }
    $primero = Hook (Prompt 'chat-1' 'p1' 'el refresh token de OAuth expira antes de tiempo')
    if ($primero.informe.tarea.estado -ne 'vinculado') { throw 'El prompt no se vinculo a la tarea' }
    Hook (Herramienta 'chat-1' 'p1' 'toolu_1' 'fallan 2 tests de integracion') | Out-Null
    # Replay exacto: no agrega una observacion.
    Hook (Herramienta 'chat-1' 'p1' 'toolu_1' 'fallan 2 tests de integracion') | Out-Null
    $segundo = Hook (Prompt 'chat-2' 'q1' 'seguimos con el refresh token de OAuth')
    if ($segundo.informe.tarea.estado -ne 'vinculado') { throw 'El segundo chat no se vinculo a la misma tarea' }

    # 3. Criterio fallido y despues corregido, con verificador local REAL sobre
    #    datos sinteticos. El resultado sale de la regla, no de una frase.
    $jsonl = Join-Path $root 'contratos/evidencia.jsonl'
    $lineas = @()
    foreach ($caso in @(@{ n = 1; fallidos = 2 }, @{ n = 2; fallidos = 0 })) {
        # Verificador determinista: cuenta fallidos y compara con cero.
        $cumple = ($caso.fallidos -eq 0)
        $lineas += (@{
            version = 1; clave_evento = "demo/ver/$($caso.n)"; grupo_evidencia = 'demo/verificaciones'
            secuencia = $caso.n; origen = 'herramienta'; rol = 'otro'
            timestamp = (1800000000000000 + $caso.n); contenido = "fallidos=$($caso.fallidos)"
            metadatos = @{ proyecto = $proyectoClave; procedencia = 'real'; evaluacion = @{
                version = 1; fuente = 'verificador_externo'; procedencia = 'real'
                criterio = 'c-tests'; criterio_version = '1'; alcance = 'tarea'; objetivo = $null
                resultado = [int]$cumple
                detalle = @{ nombre = 'conteo-de-fallidos'; version = '1'; tarea = 'demo/login'
                    entrada = @{ fallidos = $caso.fallidos }
                    salida = @{ cumple = $cumple }
                    regla = @{ campo = '/cumple'; si_true = 1; si_false = 0 }
                    ejecucion = 'real_local' }
                nota = 'verificador local real sobre datos sinteticos'; anterior = $null } }
        } | ConvertTo-Json -Depth 20 -Compress)
    }
    [IO.File]::WriteAllText($jsonl, ($lineas -join "`n") + "`n", $utf8)
    $base = Join-Path $datos "memorias/$memoria.sqlite"
    & $bin $base import $jsonl | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'No se pudo importar la evidencia' }
    Control @('tarea-registrar', '--memoria', $memoria, '--contrato', (Contrato 'res1' @{
        operacion = 'tarea_resultado'; clave = 'demo/login/res/1'; tarea = 'demo/login'
        criterio = 'c-tests'; criterio_version = '1'; evidencia = 'demo/ver/1'
        alcance = 'tarea'; objetivo = $null; anterior = $null })) | Out-Null
    $intermedio = Control @('tarea-informe', '--memoria', $memoria, '--proyecto', $proyectoClave, '--tarea', 'demo/login')
    if ($intermedio.calidad.fallidos -notcontains 'c-tests/1') { throw 'No se registro el criterio fallido' }
    Control @('tarea-registrar', '--memoria', $memoria, '--contrato', (Contrato 'res2' @{
        operacion = 'tarea_resultado'; clave = 'demo/login/res/2'; tarea = 'demo/login'
        criterio = 'c-tests'; criterio_version = '1'; evidencia = 'demo/ver/2'
        alcance = 'tarea'; objetivo = $null; anterior = 'demo/login/res/1' })) | Out-Null

    # 4. Tarea cerrada SIN evaluacion de todos los criterios.
    Control @('tarea-registrar', '--memoria', $memoria, '--contrato', (Contrato 'cierre' @{
        operacion = 'tarea_marcar'; clave = 'demo/login/cierre'; tarea = 'demo/login'
        tipo = 'cierre'; detalle = @{ nota = 'cerrar no confirma calidad' } })) | Out-Null

    $informe = Control @('tarea-informe', '--memoria', $memoria, '--proyecto', $proyectoClave,
        '--tarea', 'demo/login', '--json', (Join-Path $run 'informe-tarea.json'),
        '--markdown', (Join-Path $run 'informe-tarea.md'))
    $informe = Get-Content -Raw (Join-Path $run 'informe-tarea.json') | ConvertFrom-Json
    if ($informe.sesiones.Count -ne 2) { throw 'La tarea deberia abarcar dos chats' }
    if ($informe.calidad.cumplidos -notcontains 'c-tests/1') { throw 'La correccion no quedo vigente' }
    if ($informe.calidad.desconocidos -notcontains 'c-docs/1') { throw 'c-docs deberia quedar desconocido' }
    if ($informe.calidad.todos_los_obligatorios_verificados -ne $true) { throw 'c-tests es el unico obligatorio y esta cumplido' }
    if ($null -ne $informe.tokens.valor) { throw 'Los tokens deben quedar en null con motivo' }
    if ($informe.herramientas.operaciones.Count -ne 1) { throw 'El replay no debe multiplicar operaciones' }
    if ($informe.herramientas.operaciones[0].observaciones -ne 1) { throw 'El replay sumo una observacion' }
    if ($informe.tiempo.intervalo_abierto_desde) { throw 'La ventana deberia estar cerrada' }

    # 5. Pausa intacta: apagar no captura ni entrega.
    Control @('sesion-apagar', '--instalacion', $inst, '--sesion', 'chat-2') | Out-Null
    $apagado = Hook (Prompt 'chat-2' 'q9' 'MARCADORAPAGADO no deberia guardarse')
    if ($apagado.informe.capturado -ne $false -or $apagado.bytes -ne 0) { throw 'Un hook apagado capturo o emitio' }

    # 6. Snapshot inicial y final, consistentes y validados.
    $inicial = Join-Path $root 'snapshots/inicial.sqlite'
    $snapInicial = Control @('snapshot-crear', '--memoria', $memoria, '--destino', $inicial, '--nota', 'hito inicial')
    Hook (Prompt 'chat-1' 'p9' 'mas contexto sobre OAuth, refresh y expiracion') | Out-Null
    $final = Join-Path $root 'snapshots/final.sqlite'
    $snapFinal = Control @('snapshot-crear', '--memoria', $memoria, '--destino', $final, '--nota', 'hito final')
    if ($snapFinal.corte.instancia_id_maxima -le $snapInicial.corte.instancia_id_maxima) { throw 'El snapshot final deberia incluir mas eventos' }
    foreach ($s in $inicial, $final) {
        if ((Control @('snapshot-validar', '--archivo', $s)).valido -ne $true) { throw "Snapshot invalido: $s" }
    }
    # Redireccion por cmd: en Windows PowerShell el stderr de un comando nativo
    # se convierte en error terminante con ErrorActionPreference = Stop.
    $rechazo = Join-Path $run 'snapshot-rechazado.txt'
    & cmd /c ('"{0}" --control snapshot-crear --memoria {1} --destino "{2}" 2> "{3}"' -f
        $binAbs, $memoria, $inicial, $rechazo) | Out-Null
    if ($LASTEXITCODE -eq 0) { throw 'Un snapshot no debe sobrescribir su destino' }
    if ((Get-Content -Raw $rechazo) -notmatch 'no sobrescribe') { throw 'El rechazo no explica el motivo' }

    # 7. Tres condiciones aisladas, preparadas sin lanzar Claude.
    $casos = Join-Path $root 'contratos/casos.json'
    [IO.File]::WriteAllText($casos, (@{
        version = 1; raiz_trabajo = $proyecto; repeticiones = 1
        condiciones = @(
            @{ nombre = 'sin_recuperacion'; modo = 'solo_registro'; snapshot = '../snapshots/inicial.sqlite' },
            @{ nombre = 'memoria_inicial'; modo = 'memoria_activa'; snapshot = '../snapshots/inicial.sqlite' },
            @{ nombre = 'memoria_acumulada'; modo = 'memoria_activa'; snapshot = '../snapshots/final.sqlite' })
        casos = @(@{ id = 'caso-1'; prompt = 'arregla la expiracion del refresh token'
            criterios = @(@{ id = 'c-tests'; version = '1' })
            estado_inicial_codigo = @{ commit = 'sintetico-abc'; artefacto = $null }
            configuracion = @{ modelo = $null; herramientas = @('Bash') }
            exposicion_previa = $false })
    } | ConvertTo-Json -Depth 20 -Compress), $utf8)
    $corridas = Join-Path $root 'corridas'
    $indice = Control @('corrida-preparar', '--casos', $casos, '--salida', $corridas)
    if ($indice.preparadas -ne 3) { throw 'Deberian prepararse tres condiciones' }
    foreach ($c in $indice.corridas) {
        if ($c.antecedentes_elegibles -ne $true) { throw "Antecedentes no elegibles en $($c.condicion.nombre)" }
        if (-not (Test-Path (Join-Path $c.raiz_datos "memorias/$memoria.sqlite"))) { throw 'Falta la copia de trabajo' }
    }

    # 8. Una corrida produce su informe; las demas quedan pendientes.
    $dir = Join-Path $corridas 'caso-1/memoria_inicial/r1'
    Control @('corrida-informe', '--dir', $dir, '--tarea', 'demo/login') | Out-Null
    $propio = Get-Content -Raw (Join-Path $dir 'informe.json') | ConvertFrom-Json
    if ($propio.eventos_vinculados -ne 0) { throw 'La corrida todavia no produjo eventos propios' }
    if ($propio.desde_instancia_id -le 0) { throw 'El informe de corrida debe excluir la historia de la copia' }
    $tabla = Control @('corrida-tabla', '--salida', $corridas)
    if ($tabla.pendientes -ne 2) { throw 'Dos condiciones deberian quedar pendientes' }
    $noComparables = @($tabla.filas | Where-Object { -not $_.comparable })
    if ($noComparables.Count -ne 3) { throw 'Con el modelo desconocido ninguna fila es comparable todavia' }

    # 9. El snapshot y la memoria de produccion quedaron intactos.
    if ((Control @('snapshot-validar', '--archivo', $inicial)).valido -ne $true) { throw 'La corrida altero el snapshot' }
    $produccion = Control @('tarea-informe', '--memoria', $memoria, '--proyecto', $proyectoClave, '--tarea', 'demo/login')
    if ($produccion.eventos_vinculados -ne $informe.eventos_vinculados + 1) { throw 'La corrida contamino la memoria de produccion' }

    Write-Output "DATOS SINTETICOS (no miden una LLM real). Tarea demo/login en $($informe.sesiones.Count) chats; $($informe.eventos_vinculados) eventos vinculados."
    Write-Output "Criterio c-tests fallido y luego corregido: vigente = $($informe.por_criterio[0].estado); c-docs queda desconocido; tarea cerrada sin evaluarlo."
    Write-Output "Herramientas: $($informe.herramientas.operaciones.Count) operacion(es) por ID, cobertura $($informe.herramientas.cobertura); tokens null porque ningun hook los expone."
    Write-Output "Ventana observada: $($informe.tiempo.ventana_observada_total) us. Snapshots inicial/final validados; 3 condiciones aisladas preparadas y $($tabla.pendientes) pendientes."
    Write-Output "Artefactos en $run (hooks, informes, snapshots, corridas y tabla)."
}
finally {
    Remove-Item Env:MSP_RAIZ -ErrorAction SilentlyContinue
    Pop-Location
    [Environment]::CurrentDirectory = $cwdPrevio
}
