# Comandos para revisar a mano las entregas 6 a 12

Todo se corre desde `codigo/`. `msp` es `target/debug/msp` (en Windows, `target\debug\msp.exe`) después de `cargo build --locked`. Para no tocar tus datos reales, usá una raíz de datos aparte:

```bash
export MSP_RAIZ=/tmp/msp-prueba        # PowerShell: $env:MSP_RAIZ = "$env:TEMP\msp-prueba"
```

Cada bloque dice **qué tenés que ver**. Los marcados «corrido a mano» los ejecuté tal cual con el binario; los demás están cubiertos por las pruebas automáticas, no por una corrida manual mía.

## Comprobaciones generales

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets            # 312 pruebas; una imprime «No space left on device» a propósito
```

## Entrega 6: competencia, archivo y alerta

```bash
# Comparación de las cinco familias de reglas en el mundo base (unos 15 segundos).
cargo test --locked --test competencia simulacion -- --nocapture
# Ver: «produccion» elige k = 2 con costo cerca de 33,25; «pliegue» cerca de 33,53;
# «horquilla» cerca de 36,93; «racha» cerca de 57,64; y "gana": false en las cuatro.

# Los doce mundos del ataque (unos 2 minutos, compilación aparte).
cargo test --release --locked --test competencia mundos -- --ignored --nocapture
# Ver: una línea por mundo; «GANA» marca dónde una familia le gana a la regla simple.

# Tasas de la alerta temprana sobre series sintéticas.
cargo test --locked --test competencia operacion -- --nocapture
# Ver: consultas con alerta 0,6 % (p = 0,95) y 2,9 % (p = 0,9); detección 98,6 %.

# La regla y el archivo sobre una memoria (necesita episodios ya procesados y con mínima).
msp --control vigente-consultar --memoria ID --proyecto PROYECTO --objetivo "texto del objetivo"
msp --control objetivos-listar  --memoria ID --proyecto PROYECTO
# Ver en la salida: vigente.camino.neto, vigente.en_falla, vigente.alerta.nivel,
# caminos[].estado y sucesor, archivo.historial y archivo.vinculos_migrados.
```

## Entrega 7: SZZ

```bash
# Propone vínculos; no resta nada. --arreglo acepta un hash o HEAD.
msp --control szz-rastrear  --memoria ID --proyecto PROYECTO --repo RUTA_DEL_REPO --arreglo HEAD
msp --control szz-listar    --memoria ID --proyecto PROYECTO
# Sólo esto resta evidencia (una vez por vínculo, no se puede revertir):
msp --control szz-confirmar --memoria ID --proyecto PROYECTO --vinculo N
msp --control szz-descartar --memoria ID --proyecto PROYECTO --vinculo N
# Ver después en vigente-consultar: evidencia_tipo.fallos_tardios y un registro
# «fallo_tardio_szz» en observaciones.

# Tiene que fallar sin escribir nada:
msp --control szz-rastrear --memoria ID --proyecto PROYECTO --repo RUTA_DEL_REPO --arreglo -h
```

Para que haya algo que vincular, el commit señalado tiene que haberlo hecho un agente con la memoria encendida: el hash se busca en la salida capturada de un `git commit`.

## Entregas 8 a 10: adaptadores (corrido a mano, sólo contra el binario)

```bash
msp --control plataformas          # qué declara cada adaptador y su cobertura

# Un evento de cada host por stdin. Sin memoria encendida no persiste nada.
echo '{"hook_event_name":"SessionStart","session_id":"abc","cwd":"'"$PWD"'"}' | msp --hook --plataforma codex
echo '{"hook_event_name":"SessionStart","session_id":"abc","cwd":"'"$PWD"'"}' | msp --hook --plataforma local

# Tiene que fallar: opción mal escrita (antes caía en silencio a Claude Code).
echo '{}' | msp --hook --plataform codex
```

Archivos de instalación, sin probar contra los hosts reales: `codex/hooks.json` (va a `~/.codex/hooks.json`), `opencode-plugin/msp.js` (va a `.opencode/plugins/`), `local/envoltorio_ollama.py MODELO`. En estas tres plataformas la sesión se nombra con prefijo en la CLI: `--sesion codex:abc`.

## Entregas 11 y 12: canales, proyectos y roles (corrido a mano)

```bash
A=/tmp/proy/a; B=/tmp/proy/b; mkdir -p $A $B $A/dentro

msp --control proyecto-registrar --raiz $A
msp --control proyecto-carpeta   --raiz $A --carpeta $B          # "agregada": true
msp --control proyecto-carpeta   --raiz $A --carpeta $A/dentro   # falla: se superpone
msp --control proyecto-principal --raiz $A --carpeta $B          # cambia la marca, no la identidad
msp --control proyecto-ver       --raiz $A

msp --control sesion-iniciar --sesion guion   --cwd $A
msp --control sesion-iniciar --sesion revisor --cwd $B           # otra carpeta, mismo proyecto

msp --control proyecto-reglas --raiz $A --texto "Todo cambio pasa por el revisor."
msp --control rol-definir --raiz $A --rol guion   --recibe "la idea"  --entrega "el guion"      --terminado "aprobado"          --pasa-a revisor
msp --control rol-definir --raiz $A --rol revisor --recibe "el guion" --entrega "observaciones" --terminado "sin observaciones" --pasa-a guion

msp --control rol-tomar --sesion guion --rol guion
msp --control rol-pasar --sesion guion --texto "Guion listo"     # falla: nadie tiene el rol revisor
msp --control rol-tomar --sesion revisor --rol revisor
msp --control rol-pasar --sesion guion --texto "Guion listo"     # "destinatarios": 1

msp --control canal-leer      --sesion revisor --canal proyecto  # el pase, dentro del sobre autoridad="ninguna"
msp --control canal-leer      --sesion revisor --canal proyecto  # "mensajes": 0: no se repite
msp --control proyecto-leer   --sesion revisor                   # reglas y procedimiento del rol
msp --control canal-historial --sesion guion --canal proyecto

# Canal general, audiencias y señal de vida.
msp --control canal-latido   --sesion revisor
msp --control canal-publicar --sesion guion --canal general --texto "¿Alguien vio el error de build?" --audiencia activos
msp --control canal-publicar --sesion revisor --canal general --texto "Sí, es la caché." --responde-a 2
msp --control canal-publicar --sesion guion --canal general --texto "para vos" --audiencia uno --para revisor

# Tiene que fallar: opción mal escrita (antes publicaba a todos).
msp --control canal-publicar --sesion guion --canal general --texto hola --audienca uno
```

Topes para probar el rechazo: 4096 bytes por mensaje; 10 mensajes por minuto y 200 por día por remitente; 60 por minuto y 1000 por día por canal.

## La urna (entregas 13 y 14)

```bash
# Las tres simulaciones y sus números (unos 10 segundos).
cargo test --locked --test urna -- --nocapture
# Ver: con gamma 1 todas las corridas quedan en el medio; con gamma 4, 51 % abajo y 47 % arriba;
# sesgo crítico exacto 1,066 contra 0,471 de la forma cúbica; «sin_adopcion» 1.0 en las
# cinco filas del mundo por cuota; y en el mundo por tasa, coexistencia con 0 a 2 pasos
# compartidos y sucesión con 3 o 4.

# La lectura sobre una memoria: el bloque «urna» de la consulta.
msp --control vigente-consultar --memoria ID --proyecto PROYECTO --objetivo "texto del objetivo"
# Ver: urna.caminos[].profundidad y exitos_inducidos, cuota_reciente y cuota_anterior,
# pasos_no_compartidos_con_vigente, urna.seguimiento.p_seguir y urna.vigente_en_desuso.
```

## Mutación

```bash
# Sólo lo que cambia la rama contra main, como en la CI.
git diff --relative origin/main...HEAD -- src > /tmp/pr.diff
cargo mutants --cargo-arg=--locked --in-diff /tmp/pr.diff -j 2
python scripts/umbral_mutacion.py mutants.out/outcomes.json     # exige 90 % y ningún timeout
```

Son 319 mutantes y con la suite completa tarda horas en una máquina de 4 núcleos; en la CI corre partido en cuatro fragmentos.
