#Requires -Version 5.1
<#
.SYNOPSIS
    Prepara la prueba manual del plugin en una sesion real de Claude Code, o
    verifica el estado despues de cada paso.
.DESCRIPTION
    Sin parametros: compila e instala el binario del plugin, crea un proyecto
    temporal, registra su raiz e imprime la checklist exacta.
    Con -Verificar -Sesion <id>: muestra estado, intervalos y entregas de ese
    chat, y busca un marcador en la memoria si se pasa -Marcador.
    No lanza Claude Code ni cambia ninguna configuracion global.
.EXAMPLE
    ./prueba-manual.ps1
.EXAMPLE
    ./prueba-manual.ps1 -Verificar -Sesion abc123 -Marcador MARCADORTRAMOAPAGADO
#>
param(
    [switch]$Verificar,
    [string]$Sesion,
    [string]$Marcador,
    [string]$Proyecto,
    [string]$Instalacion
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
# El binario escribe UTF-8; fijarlo evita que una consola con otra pagina de
# codigos corrompa rutas con enie o texto recuperado. Solo para este proceso.
$OutputEncoding = New-Object Text.UTF8Encoding $false
[Console]::OutputEncoding = $OutputEncoding

$plugin = Join-Path $PSScriptRoot 'claude-plugin'
$bin = Join-Path $plugin 'bin/msp.exe'
$marcaProyecto = Join-Path $env:TEMP 'memoria-prueba-manual.txt'

function Invoke-Memoria {
    param([string[]]$Argumentos)
    # El subcomando va primero; las opciones despues.
    if ($Instalacion) { $Argumentos = $Argumentos + @('--instalacion', $Instalacion) }
    $salida = & $bin --control @Argumentos
    if ($LASTEXITCODE -ne 0) { throw "Fallo: memoria --control $($Argumentos -join ' ')" }
    if ($salida) { return ($salida | ConvertFrom-Json) }
    return $null
}

if ($Verificar) {
    if (-not (Test-Path $bin)) { throw "Falta $bin. Ejecutar primero ./prueba-manual.ps1 sin parametros." }
    if (-not $Sesion) { throw "Falta -Sesion. El id sale del campo 'sesion' de `msp:: estado` dentro del chat." }
    if (-not $Proyecto -and (Test-Path $marcaProyecto)) { $Proyecto = (Get-Content -Raw $marcaProyecto).Trim() }

    $estado = Invoke-Memoria @('sesion-estado', '--sesion', $Sesion, '--cwd', $Proyecto)
    Write-Output ''
    Write-Output "=== Chat $Sesion ==="
    if ($estado.origen_seleccion -eq 'sesion_desconocida') {
        Write-Output '  El registro no conoce este chat. Suele ser una de tres cosas:'
        Write-Output '    - el id esta mal copiado: sale del campo "sesion" de `msp:: estado`,'
        Write-Output '      que tiene la forma <instalacion>/<chat>; aca va solo la parte <chat>;'
        Write-Output '    - Claude Code todavia no disparo ningun hook en ese chat;'
        Write-Output '    - el chat corrio con otra instalacion del adaptador (pasar -Instalacion).'
        Write-Output ''
        Write-Output "  Instalacion de este registro: $($estado.instalacion)"
        Write-Output "  Raiz de datos               : $($estado.raiz_datos)"
        Write-Output ''
        return
    }
    Write-Output "  estado            : $($estado.estado)"
    Write-Output "  origen seleccion  : $($estado.origen_seleccion)"
    Write-Output "  memoria           : $($estado.memoria.nombre)  [$($estado.memoria.memoria_id)]"
    Write-Output "  proyecto          : $($estado.proyecto.raiz_normalizada)"
    Write-Output "  revision          : $($estado.revision)   intervalos: $($estado.intervalos_totales)"
    Write-Output "  eventos guardados : $($estado.eventos_asignados)"
    Write-Output "  fuera de raiz     : $($estado.fuera_de_raiz_registrada)"
    Write-Output "  operacion         : $($estado.operacion)"

    $historial = Invoke-Memoria @('sesion-historial', '--sesion', $Sesion)
    Write-Output ''
    Write-Output '--- intervalos de captura ---'
    foreach ($i in $historial.intervalos) {
        Write-Output ("  #{0} {1}  activacion rev {2}  eventos {3}  cursor {4}" -f `
            $i.intervalo_id, $i.estado, $i.revision_activacion, $i.eventos_asignados, $i.cursor_confirmados)
    }
    Write-Output '--- acciones de control ---'
    foreach ($c in $historial.control) { Write-Output "  $($c.accion)" }

    if ($estado.memoria.memoria_id) {
        $entregas = Invoke-Memoria @('entregas', '--memoria', $estado.memoria.memoria_id, '--proyecto', $estado.proyecto.raiz_normalizada)
        $propias = @($entregas.entregas | Where-Object { $_.sesion -like "*/$Sesion" })
        Write-Output ''
        Write-Output "--- contexto ofrecido al host en este chat: $($propias.Count) ---"
        foreach ($e in $propias) {
            Write-Output ("  #{0}  {1} bytes de {2}  |  {3}  |  exposiciones remotas verificables: {4}" -f `
                $e.entrega_id, $e.bytes_salida, $e.presupuesto_integracion_bytes, $e.estado_escritura, $e.exposiciones_tanda3_verificables)
        }
        if ($Marcador) {
            $archivo = Join-Path $estado.raiz_datos "memorias/$($estado.memoria.memoria_id).sqlite"
            $hits = & $bin $archivo search $estado.proyecto.raiz_normalizada $Marcador | ConvertFrom-Json
            Write-Output ''
            Write-Output "--- busqueda de '$Marcador' en la memoria: $($hits.resultados.Count) resultado(s) ---"
            foreach ($h in $hits.resultados) { Write-Output "  $($h.origen)/$($h.rol): $($h.fragmento)" }
        }
    }
    Write-Output ''
    return
}

# ---------------------------------------------------------------- preparacion
Write-Output 'Compilando e instalando el binario del plugin...'
& (Join-Path $plugin 'instalar.ps1') | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'Fallo la instalacion del binario' }

$proyecto = Join-Path $env:TEMP "memoria-prueba-$([guid]::NewGuid().ToString('N').Substring(0,8))"
New-Item -ItemType Directory -Force $proyecto | Out-Null
Set-Content -Path (Join-Path $proyecto 'LEEME.txt') -Value 'Proyecto temporal para probar el plugin de memoria persistente.'
$registrado = Invoke-Memoria @('proyecto-registrar', '--raiz', $proyecto)
Set-Content -Path $marcaProyecto -Value $registrado.raiz_normalizada -NoNewline

$memorias = Invoke-Memoria @('memoria-listar')

Write-Output ''
Write-Output '================================================================'
Write-Output ' PRUEBA MANUAL EN UNA SESION REAL DE CLAUDE CODE'
Write-Output '================================================================'
Write-Output ''
Write-Output "Proyecto temporal : $proyecto"
Write-Output "Raiz de datos     : $($memorias.raiz_datos)"
Write-Output "Memorias actuales : $($memorias.memorias.Count)"
Write-Output ''
Write-Output '--- 1. Abrir Claude Code con el plugin, SIN instalarlo globalmente ---'
Write-Output ''
Write-Output "    cd `"$proyecto`""
Write-Output "    claude --plugin-dir `"$plugin`""
Write-Output ''
Write-Output '--- 2. Dentro del chat, tipear estas ordenes (el modelo no las ejecuta) ---'
Write-Output ''
Write-Output '    msp:: estado                 -> deberia decir apagada'
Write-Output '    msp:: crear Memoria A'
Write-Output '    msp:: usar <ID de A>         -> el ID sale del paso anterior'
Write-Output '    msp:: estado                 -> encendida / sesion_explicita'
Write-Output ''
Write-Output '    Anotar el campo "sesion" que muestra el estado: lo necesitas abajo.'
Write-Output '    Despues escribir un antecedente reconocible, por ejemplo:'
Write-Output '      El puerto de staging de este proyecto es 8431.'
Write-Output ''
Write-Output '--- 3. Verificar desde afuera, en otra terminal ---'
Write-Output ''
Write-Output "    cd `"$PSScriptRoot`""
Write-Output '    ./prueba-manual.ps1 -Verificar -Sesion <sesion del paso 2>'
Write-Output ''
Write-Output '--- 4. Los seis pasos de la prueba ---'
Write-Output ''
Write-Output '  a) Chat nuevo vinculado a A RECUPERA el antecedente.'
Write-Output '     Cerrar el chat, abrir otro en la misma carpeta, msp:: usar <ID de A>,'
Write-Output '     preguntar cual era el puerto de staging. Debe aparecer un bloque'
Write-Output '     <antecedentes-msp ...> antes de la respuesta.'
Write-Output ''
Write-Output '  b) Chat vinculado a B NO recupera A.'
Write-Output '     msp:: crear Memoria B ; en un chat nuevo msp:: usar <ID de B>'
Write-Output '     ; misma pregunta. Sin bloque, y -Verificar muestra 0 entregas.'
Write-Output ''
Write-Output '  c) Chat apagado: sin captura ni inyeccion.'
Write-Output '     Chat nuevo, ninguna orden, misma pregunta. -Verificar debe mostrar'
Write-Output '     estado apagada y 0 eventos guardados.'
Write-Output ''
Write-Output '  d) Pausar y reactivar A, sin backfill.'
Write-Output '     En el chat de (a): msp:: apagar ; escribir MARCADORTRAMOAPAGADO ;'
Write-Output '     msp:: usar <ID de A> ; seguir conversando. Luego:'
Write-Output '       ./prueba-manual.ps1 -Verificar -Sesion <esa sesion> -Marcador MARCADORTRAMOAPAGADO'
Write-Output '     Debe dar 0 resultados y 2 intervalos, el primero cerrado.'
Write-Output ''
Write-Output '  e) Predeterminado, solo para chats nuevos.'
Write-Output '     msp:: predeterminado <ID de A> ; un chat nuevo nace encendido con'
Write-Output '     origen_seleccion predeterminado_proyecto; los anteriores no cambian.'
Write-Output ''
Write-Output '  f) Dos chats simultaneos e independientes.'
Write-Output '     Abrir dos a la vez, apagar uno con msp:: apagar y comprobar que'
Write-Output '     el otro sigue capturando.'
Write-Output ''
Write-Output '--- 5. Tanda 5: tarea y modo solo registro ---'
Write-Output ''
Write-Output '  g) Seleccion de tarea.'
Write-Output '     Registrar una tarea (ver ejemplos/tarea-crear.json; reemplazar el proyecto'
Write-Output '     por la raiz normalizada que imprime este script):'
Write-Output "       ./claude-plugin/bin/msp.exe --control tarea-registrar --memoria <ID> --contrato <archivo>"
Write-Output '     En el chat: msp:: tareas ; msp:: tarea <clave> ; msp:: estado'
Write-Output '     El estado debe mostrar la tarea y seguir en memoria_activa.'
Write-Output '     Despues de un par de mensajes, tarea-informe debe listarlos como vinculados:'
Write-Output "       ./claude-plugin/bin/msp.exe --control tarea-informe --memoria <ID> --proyecto <raiz> --tarea <clave>"
Write-Output ''
Write-Output '  h) Modo solo registro.'
Write-Output '     En un chat NUEVO: msp:: registrar <ID> ; msp:: estado -> solo_registro.'
Write-Output '     Preguntar algo que en modo activo habria recuperado antecedentes: NO debe'
Write-Output '     aparecer el bloque <antecedentes-msp ...>, y el informe debe'
Write-Output '     mostrar el evento capturado con cero paquetes y cero entregas nuevas.'
Write-Output ''
Write-Output '--- Al terminar ---'
Write-Output ''
Write-Output "    Remove-Item -Recurse -Force `"$proyecto`""
Write-Output "    # Las memorias de prueba quedan en $($memorias.raiz_datos)"
Write-Output '    # y se listan con:  ./claude-plugin/bin/msp.exe --control memoria-listar'
Write-Output ''
