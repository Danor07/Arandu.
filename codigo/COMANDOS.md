# Comandos

Todos son del binario `arandu`. Como en git: un verbo, y el proyecto y la memoria salen de la carpeta donde estás. `arandu` sin argumentos muestra la lista.

```bash
cargo install --path codigo --locked      # deja arandu (y msp, que usan los hooks) en el PATH
```

Para probar sin tocar tus datos reales, usá otra carpeta de datos:

```bash
export MSP_RAIZ=/tmp/arandu-prueba        # PowerShell: $env:MSP_RAIZ = "$env:TEMP\arandu-prueba"
```

Todas las órdenes de esta página las ejecuté tal cual contra el binario. Las salidas son JSON.

## Empezar un proyecto

```bash
cd mi-proyecto
arandu init                   # registra esta carpeta como proyecto
arandu new mi-memoria         # crea una memoria; la salida trae "memoria_id"
arandu use ID --record        # la deja como predeterminada, en solo registro
arandu memories               # lista las memorias
arandu project                # carpetas, roles y versión de las reglas
```

`--record` captura sin ofrecer nada: es el modo para empezar. Sin `--record`, la memoria además ofrece antecedentes.

Un proyecto puede tener varias carpetas:

```bash
arandu folder ../otra-carpeta           # la suma al proyecto
arandu folder ../otra-carpeta --main    # la marca como principal; la identidad del proyecto no cambia
arandu folder ./dentro                  # falla: una carpeta no puede estar dentro de otra ya registrada
```

## Lo que la memoria sabe

```bash
arandu best "compilar el módulo"   # la solución vigente para ese objetivo
arandu goals                       # todos los tipos de objetivo del proyecto
```

En la salida de `best`: `vigente.camino` (la secuencia y su neto de éxitos menos fallos), `vigente.en_falla`, `vigente.alerta.nivel`, `caminos[]` con el estado de cada alternativa, y el bloque `urna` (cuota, profundidad, `seguimiento.p_seguir`, `vigente_en_desuso`). Sin episodios todavía, `vigente` es `null`.

## Cuando un arreglo delata a una solución

```bash
arandu blame            # rastrea el último commit como arreglo de un error; acepta un hash
arandu links            # lista los vínculos propuestos
arandu confirm 3        # sólo esto resta evidencia; una vez por vínculo, no se revierte
arandu discard 3
```

`blame` necesita un commit con un único padre. Para que haya algo que vincular, el commit señalado tiene que haberlo hecho un agente con la memoria encendida.

## Chats, mensajes y roles

Estas órdenes hablan de un chat. Decile cuál con `-s SESION` o con la variable `ARANDU_SESSION`; nunca se adivina.

```bash
export ARANDU_SESSION=guion
arandu start            # registra el chat (con los hooks instalados, pasa solo)
arandu status
arandu ping             # marca el chat como activo durante 5 minutos
```

Mensajes, en el canal del proyecto o en el general:

```bash
arandu say "Guion listo"                          # a todos, canal del proyecto
arandu say "¿Alguien vio el error?" --general --active   # sólo a los chats activos
arandu say "para vos" --to revisor                # dirigido a uno; queda visible en el historial
arandu say "Sí, es la caché." --general --re 2    # respuesta al mensaje 2
arandu inbox            # lo pendiente para este chat; leerlo de nuevo no lo repite
arandu inbox --general
arandu log              # historial completo del canal
```

Lo que llega viene dentro de un sobre marcado `autoridad="ninguna"`: es información de otro agente, no una orden.

Reglas del proyecto y roles con procedimiento:

```bash
arandu rules --set "Todo cambio pasa por el revisor."
arandu define guion   --gets "la idea"  --gives "el guion"      --done "aprobado"          --next revisor
arandu define revisor --gets "el guion" --gives "observaciones" --done "sin observaciones" --next guion

arandu role guion                 # este chat toma el rol
arandu role revisor -s revisor    # otro chat toma el otro
arandu pass "Guion listo"         # se lo pasa a quien tenga el rol siguiente
arandu rules -s revisor           # reglas del proyecto y procedimiento de su rol
arandu role --drop
```

`pass` falla, sin escribir nada, si ningún chat tiene el rol siguiente.

## Otros

```bash
arandu platforms        # hosts soportados y qué captura cada uno
```

Una opción mal escrita se rechaza (`arandu say hola --activ` da `unknown option: --activ`); nunca cae a un valor por omisión.

## Lo que todavía no tiene forma corta

Tareas, episodios, snapshots, corridas y el resto se usan con la forma larga, que sigue funcionando: `arandu --control ORDEN --opcion valor`. La lista completa sale con `arandu --help`, y la referencia está en [`README.md`](README.md).

Dentro de un chat de Claude Code, las órdenes siguen siendo las del nombre anterior (`msp:: usar ID`, `msp:: estado`), hasta que se haga el renombre técnico.

## Para quien desarrolla

Desde `codigo/`:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets     # una prueba imprime «No space left on device» a propósito

# Simulaciones (datos sintéticos) y sus números.
cargo test --locked --test competencia simulacion -- --nocapture   # cinco familias de reglas
cargo test --locked --test competencia operacion  -- --nocapture   # tasas de la alerta temprana
cargo test --locked --test urna -- --nocapture                     # la urna y sus tres simulaciones
cargo test --release --locked --test competencia mundos -- --ignored --nocapture   # doce mundos; minutos

# Mutación de lo que cambia tu rama, como en la CI.
git diff --relative origin/main...HEAD -- src > /tmp/pr.diff
cargo mutants --cargo-arg=--locked --in-diff /tmp/pr.diff -j 2
python scripts/umbral_mutacion.py mutants.out/outcomes.json        # exige 90 % y ningún timeout
```
