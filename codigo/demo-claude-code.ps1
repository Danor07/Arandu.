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

    # REPLAY DE PROTOCOLO, NO UNA SESIÓN REAL DE CLAUDE CODE. Los payloads
    # reproducen los campos documentados de los hooks de la versión 2.1.270; no
    # provienen de ningún chat personal y no se abre ninguna sesión remota.
    New-Item -ItemType Directory -Force demo-output | Out-Null
    $run = "demo-output/claude-code-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Force $run | Out-Null
    # Raíz de datos temporal y propia de esta ejecución: no toca bases previas
    # ni la configuración personal del usuario.
    $root = (Resolve-Path $run).Path
    $datos = Join-Path $root 'datos de memoria'
    $proyecto = Join-Path $root 'mi proyecto ñ'
    New-Item -ItemType Directory -Force $datos | Out-Null
    New-Item -ItemType Directory -Force (Join-Path $proyecto 'sub carpeta') | Out-Null
    $env:MSP_RAIZ = $datos
    $bin = '.\target\debug\msp.exe'
    $binAbs = (Resolve-Path $bin).Path
    $inst = 'demo-instalacion'
    $paso = 0
    $utf8 = New-Object Text.UTF8Encoding $false

    function Control {
        param([string[]]$Argumentos)
        $salida = & $bin --control @Argumentos
        if ($LASTEXITCODE -ne 0) { throw "Control falló: $($Argumentos -join ' ')" }
        if ($salida) { return ($salida | ConvertFrom-Json) }
        return $null
    }

    # Reproduce un hook: payload documentado por stdin, contexto por stdout,
    # informe a un archivo. stdout queda reservado al protocolo del host.
    function Hook {
        param([hashtable]$Payload, [switch]$Emitir, [switch]$SinConfirmar)
        $script:paso++
        $n = '{0:d2}' -f $script:paso
        $json = ($Payload | ConvertTo-Json -Depth 20 -Compress)
        $entrada = Join-Path $run "hook-$n.json"
        $informe = Join-Path $run "informe-$n.json"
        $contexto = Join-Path $run "contexto-$n.txt"
        [IO.File]::WriteAllText($entrada, $json, $utf8)
        $extra = if ($SinConfirmar) { ' --sin-confirmar' } elseif ($Emitir) { '' } else { ' --sin-emitir' }
        # Redirección por cmd: copia los bytes exactos de stdin y stdout, sin que
        # PowerShell reinterprete la codificación. Todas las rutas van entre
        # comillas porque contienen espacios y acentos.
        $linea = '"{0}" --hook --instalacion {1} --informe "{2}"{3} < "{4}" > "{5}"' -f
            $binAbs, $inst, $informe, $extra, $entrada, $contexto
        & cmd /c $linea
        if ($LASTEXITCODE -ne 0) { throw "Hook falló: $($Payload.hook_event_name)" }
        $texto = if (Test-Path $contexto) { $utf8.GetString([IO.File]::ReadAllBytes($contexto)) } else { '' }
        return [pscustomobject]@{
            informe  = (Get-Content -Raw $informe | ConvertFrom-Json)
            contexto = $texto
            bytes    = [Text.Encoding]::UTF8.GetByteCount($texto)
        }
    }

    function Prompt {
        param($sesion, $id, $texto, $cwd = $proyecto)
        @{ hook_event_name = 'UserPromptSubmit'; session_id = $sesion; prompt_id = $id
           cwd = $cwd; transcript_path = (Join-Path $cwd 'transcript.jsonl')
           permission_mode = 'default'; prompt = $texto }
    }
    function Fin {
        param($sesion, $id, $texto)
        @{ hook_event_name = 'Stop'; session_id = $sesion; prompt_id = $id
           cwd = $proyecto; transcript_path = (Join-Path $proyecto 'transcript.jsonl')
           permission_mode = 'default'; last_assistant_message = $texto }
    }

    # 1. Instalado pero apagado: un chat nuevo sin predeterminado no captura nada.
    Control @('proyecto-registrar', '--raiz', $proyecto) | Out-Null
    $inicio = Hook @{ hook_event_name = 'SessionStart'; session_id = 'chat-1'; cwd = $proyecto
                      transcript_path = (Join-Path $proyecto 'transcript.jsonl'); trigger = 'startup' }
    if ($inicio.informe.estado.estado -ne 'apagada') { throw 'Un chat nuevo sin predeterminado debería nacer apagado' }
    $mudo = Hook (Prompt 'chat-1' 'p0' 'Este prompt no debe persistirse en ningún lado.')
    if ($mudo.informe.estado -ne 'apagada' -or $mudo.informe.capturado -ne $false -or $mudo.bytes -ne 0) { throw 'Un hook apagado capturó o emitió algo' }

    # 2. Crear no activa; encender es una decisión explícita de esta sesión.
    $memoriaA = Control @('memoria-crear', '--nombre', 'Memoria A de la demo')
    $memoriaB = Control @('memoria-crear', '--nombre', 'Memoria B de la demo')
    if ($memoriaA.activada -ne $false) { throw 'Crear no debe activar' }
    $repetida = Control @('memoria-crear', '--nombre', 'Memoria A de la demo')
    if ($repetida.memoria_id -ne $memoriaA.memoria_id) { throw 'Crear no fue idempotente' }
    $encendida = Control @('sesion-usar', '--instalacion', $inst, '--sesion', 'chat-1', '--cwd', $proyecto, '--memoria', $memoriaA.memoria_id)
    if ($encendida.estado -ne 'encendida' -or $encendida.origen_seleccion -ne 'sesion_explicita') { throw 'No se encendió por decisión de sesión' }
    $proyectoClave = $encendida.proyecto.raiz_normalizada

    # 3. Captura del turno completo: prompt, herramienta y mensaje final.
    $antecedente = Hook (Prompt 'chat-1' 'p1' 'La migración de esquema necesita un índice compuesto por proyecto y timestamp.')
    if ($antecedente.informe.capturado -ne $true -or $antecedente.bytes -ne 0) { throw 'El primer prompt no se capturó, o inventó antecedentes' }
    Hook @{ hook_event_name = 'PostToolUse'; session_id = 'chat-1'; prompt_id = 'p1'
            tool_use_id = 'toolu_demo_1'; tool_name = 'Bash'; cwd = $proyecto
            transcript_path = (Join-Path $proyecto 'transcript.jsonl'); permission_mode = 'default'
            tool_input = @{ command = 'sqlite3 base .schema' }; tool_response = 'CREATE INDEX idx_proyecto_tiempo ...' } | Out-Null
    Hook @{ hook_event_name = 'PostToolUseFailure'; session_id = 'chat-1'; prompt_id = 'p1'
            tool_use_id = 'toolu_demo_2'; tool_name = 'Bash'; cwd = $proyecto
            transcript_path = (Join-Path $proyecto 'transcript.jsonl'); permission_mode = 'default'
            tool_input = @{ command = 'sqlite3 base .fail' }; error = 'exit status 1' } | Out-Null
    $cierre = Hook (Fin 'chat-1' 'p1' 'Se agregó un índice compuesto sobre proyecto y timestamp en la migración.')
    if ($cierre.informe.enlace.recibo.entidad -ne 'ejecucion_evento') { throw 'El fin de turno no se enlazó al intento' }

    # 4. Replay exacto del mismo payload: misma identidad, sin duplicar.
    $replay = Hook (Prompt 'chat-1' 'p1' 'La migración de esquema necesita un índice compuesto por proyecto y timestamp.')
    if ($replay.informe.insertado -ne $false -or $replay.informe.instancia_id -ne $antecedente.informe.instancia_id) { throw 'El replay duplicó el evento' }

    # 5. Otro chat de la misma memoria y proyecto recupera ese antecedente.
    Control @('sesion-usar', '--instalacion', $inst, '--sesion', 'chat-2', '--cwd', (Join-Path $proyecto 'sub carpeta'), '--memoria', $memoriaA.memoria_id) | Out-Null
    $recuerdo = Hook (Prompt 'chat-2' 'q1' '¿Recordás el índice compuesto de la migración?') -Emitir
    if ($recuerdo.bytes -eq 0) { throw 'El chat nuevo no recuperó el antecedente' }
    if ($recuerdo.contexto -notmatch 'indice compuesto' -and $recuerdo.contexto -notmatch 'índice compuesto') { throw 'El bloque no contiene el antecedente' }
    if ($recuerdo.contexto -notmatch 'autoridad="ninguna"') { throw 'Falta el sobre que marca los datos como históricos sin autoridad' }
    if ($recuerdo.bytes -gt $recuerdo.informe.entrega.presupuesto_integracion_bytes) { throw 'Se superó el presupuesto de integración' }
    if ($recuerdo.informe.entrega.bytes_salida -ne ($recuerdo.informe.entrega.paquete_bytes + $recuerdo.informe.entrega.sobre_bytes)) { throw 'El presupuesto no incluye el sobre' }

    # 6. Un chat de la memoria B, en el mismo proyecto, no ve nada de A.
    Control @('sesion-usar', '--instalacion', $inst, '--sesion', 'chat-3', '--cwd', $proyecto, '--memoria', $memoriaB.memoria_id) | Out-Null
    $aislado = Hook (Prompt 'chat-3' 'r1' '¿Recordás el índice compuesto de la migración?')
    if ($aislado.bytes -ne 0) { throw 'La memoria B vio datos de la memoria A' }

    # 7. Una sesión no cambia de memoria: hace falta un chat nuevo.
    $rechazo = Join-Path $run 'cambio-de-memoria-rechazado.txt'
    & cmd /c ('"{0}" --control sesion-usar --instalacion {1} --sesion chat-2 --cwd "{2}" --memoria {3} 2> "{4}"' -f
        $binAbs, $inst, $proyecto, $memoriaB.memoria_id, $rechazo) | Out-Null
    if ($LASTEXITCODE -eq 0) { throw 'Se permitió cambiar la memoria de un chat existente' }
    if ((Get-Content -Raw $rechazo) -notmatch 'chat nuevo') { throw 'El rechazo no explica que hace falta un chat nuevo' }

    # 8. Pausa y reactivación: el tramo apagado no se importa después.
    Control @('sesion-apagar', '--instalacion', $inst, '--sesion', 'chat-1') | Out-Null
    $tramo = Hook (Prompt 'chat-1' 'pOFF' 'MARCADORTRAMOAPAGADO: esto queda fuera de la memoria.')
    if ($tramo.informe.capturado -ne $false) { throw 'Se capturó durante el tramo apagado' }
    $reactivada = Control @('sesion-usar', '--instalacion', $inst, '--sesion', 'chat-1', '--cwd', $proyecto, '--memoria', $memoriaA.memoria_id)
    if ($reactivada.intervalos_totales -ne 2) { throw 'La reactivación no abrió otro intervalo' }
    Hook (Prompt 'chat-1' 'p2' 'Seguimos después de reactivar la memoria.') | Out-Null
    $buscaMarcador = & $bin (Join-Path $datos "memorias/$($memoriaA.memoria_id).sqlite") search $proyectoClave 'MARCADORTRAMOAPAGADO' | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or $buscaMarcador.resultados.Count -ne 0) { throw 'El tramo apagado se importó al reactivar' }

    # 9. Predeterminado de proyecto: solo se aplica a chats nuevos.
    Control @('proyecto-predeterminado', '--raiz', $proyecto, '--memoria', $memoriaB.memoria_id) | Out-Null
    $viejo = Control @('sesion-estado', '--instalacion', $inst, '--sesion', 'chat-2', '--cwd', $proyecto)
    if ($viejo.memoria.memoria_id -ne $memoriaA.memoria_id) { throw 'El predeterminado movió un chat existente' }
    $nuevo = Hook @{ hook_event_name = 'SessionStart'; session_id = 'chat-4'; cwd = $proyecto
                     transcript_path = (Join-Path $proyecto 'transcript.jsonl'); trigger = 'startup' }
    if ($nuevo.informe.estado.origen_seleccion -ne 'predeterminado_proyecto' -or $nuevo.informe.estado.memoria.memoria_id -ne $memoriaB.memoria_id) { throw 'El predeterminado no se aplicó al chat nuevo' }
    # Reanudar conserva la decisión de esa sesión.
    Control @('sesion-apagar', '--instalacion', $inst, '--sesion', 'chat-4') | Out-Null
    $reanudado = Hook @{ hook_event_name = 'SessionStart'; session_id = 'chat-4'; cwd = $proyecto
                         transcript_path = (Join-Path $proyecto 'transcript.jsonl'); trigger = 'resume' }
    if ($reanudado.informe.estado.estado -ne 'apagada' -or $reanudado.informe.estado.sesion_creada_ahora -ne $false) { throw 'Reanudar volvió a resolver la selección' }

    # 10. Dos chats simultáneos, independientes entre sí.
    Hook (Prompt 'chat-2' 'q2' 'Sigo en el chat dos sin afectar al chat tres.') | Out-Null
    Hook (Prompt 'chat-3' 'r2' 'Sigo en el chat tres sin afectar al chat dos.') | Out-Null

    # 11. Control tipeado por el usuario dentro del chat; el mismo texto emitido
    #     por el modelo se guarda literal pero no ejecuta nada.
    $orden = Hook (Prompt 'chat-3' 'r3' 'msp:: estado')
    if ($orden.informe.accion -ne 'control_por_prompt_del_usuario' -or $orden.informe.capturado -ne $false) { throw 'La orden de control no se reconoció' }
    Hook (Fin 'chat-3' 'r3' 'msp:: apagar') | Out-Null
    $sigue = Control @('sesion-estado', '--instalacion', $inst, '--sesion', 'chat-3', '--cwd', $proyecto)
    if ($sigue.estado -ne 'encendida') { throw 'Un mensaje del modelo apagó la memoria' }

    # 12. Un cwd fuera de la raíz registrada suspende la operación.
    $fuera = Hook (Prompt 'chat-2' 'q3' 'Esto ocurre fuera de la raíz.' $datos)
    if ($fuera.informe.estado -ne 'apagada') { throw 'No se suspendió al salir de la raíz registrada' }

    # 12b. Caída entre el commit de preparación y la confirmación de escritura:
    #      la entrega queda registrada como incierta, nunca como exitosa.
    Control @('sesion-usar', '--instalacion', $inst, '--sesion', 'chat-5', '--cwd', $proyecto, '--memoria', $memoriaA.memoria_id) | Out-Null
    $incierta = Hook (Prompt 'chat-5' 'w1' 'Otra vez el índice compuesto de la migración.') -SinConfirmar
    if ($incierta.informe.entrega.estado -ne 'preparada') { throw 'No se preparó la entrega incierta' }

    # 13. Evidencia honesta: preparación, escritura y ausencia de exposición remota.
    $entregas = Control @('entregas', '--memoria', $memoriaA.memoria_id, '--proyecto', $proyectoClave)
    $entrega = $entregas.entregas | Where-Object { $_.sesion -eq "$inst/chat-2" } | Select-Object -First 1
    if (-not $entrega) { throw 'No se registró la entrega de contexto' }
    if ($entrega.estado_escritura -notmatch 'escritura_completada') { throw 'No se registró la frontera de escritura' }
    if ($entrega.exposiciones_tanda3_verificables -ne 0) { throw 'Se fabricó una exposición remota' }
    if ($entrega.fronteras_observadas.exposicion_remota_verificable -ne $false) { throw 'Se afirmó una exposición remota inexistente' }
    if ($entrega.salida_serializada -cne $recuerdo.contexto) { throw 'El bloque registrado no coincide con lo escrito al host' }
    $sinConfirmar = @($entregas.entregas | Where-Object { $_.estado_escritura -like 'incierta*' })
    if ($sinConfirmar.Count -lt 1) { throw 'Falta el caso de escritura incierta' }
    $historial = Control @('sesion-historial', '--instalacion', $inst, '--sesion', 'chat-1')
    if ($historial.intervalos.Count -ne 2 -or $historial.intervalos[0].estado -ne 'cerrado') { throw 'El historial de intervalos es incorrecto' }
    if (-not ($historial.control.accion -contains 'callback_rechazado')) { throw 'No se registró el rechazo de callbacks durante la pausa' }

    # 14. Integridad de las bases del registro y de cada memoria.
    foreach ($archivo in @((Join-Path $datos 'registro.sqlite'),
                           (Join-Path $datos "memorias/$($memoriaA.memoria_id).sqlite"),
                           (Join-Path $datos "memorias/$($memoriaB.memoria_id).sqlite"))) {
        if (-not (Test-Path $archivo)) { throw "Falta la base $archivo" }
    }
    $listado = Control @('memoria-listar')
    if ($listado.memorias.Count -ne 2) { throw 'El listado de memorias es incorrecto' }
    $entregas | ConvertTo-Json -Depth 30 | Set-Content -Encoding utf8 (Join-Path $run 'entregas.json')
    $historial | ConvertTo-Json -Depth 30 | Set-Content -Encoding utf8 (Join-Path $run 'historial-chat-1.json')
    $listado | ConvertTo-Json -Depth 30 | Set-Content -Encoding utf8 (Join-Path $run 'memorias.json')

    Write-Output "REPLAY DE PROTOCOLO (no es una sesión real de Claude Code). $($script:paso) hooks reproducidos con payloads documentados de la versión 2.1.270."
    Write-Output "Apagado sin captura ni emisión; crear no activa; chat-2 recuperó $($recuerdo.bytes) bytes de $($recuerdo.informe.entrega.presupuesto_integracion_bytes) incluyendo el sobre; memoria B aisló el mismo proyecto; cambio de memoria rechazado; tramo apagado no importado; predeterminado solo en chats nuevos; reanudar conservó la decisión; cwd fuera de raíz suspendido; $($sinConfirmar.Count) entrega(s) en estado incierto y 0 exposiciones remotas."
    Write-Output "Artefactos en $run (payloads, informes, contextos, entregas e historial). Raíz de datos temporal: $datos"
}
finally {
    Remove-Item Env:MSP_RAIZ -ErrorAction SilentlyContinue
    Pop-Location
    [Environment]::CurrentDirectory = $cwdPrevio
}
