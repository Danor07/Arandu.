# Arandu: Memory of Dynamic Solutions

Una memoria local para agentes de programación. Registra lo que el agente hace, reconoce qué secuencias de pasos llegaron a un resultado comprobado y, la próxima vez que aparece el mismo objetivo, le ofrece la que sigue funcionando.

*Arandu* es «sabiduría» en guaraní: el conocimiento que viene de la experiencia.

> **Estado: experimental.** Funciona y tiene más de 300 pruebas, pero todavía no se usó de forma sostenida en proyectos reales. Las reglas que deciden qué solución se ofrece están medidas sólo con datos sintéticos. No lo uses donde un antecedente equivocado tenga consecuencias.

## Qué hace

- **Captura sin filtrar.** Guarda en SQLite, en tu máquina, los eventos que el host expone: prompts, llamadas a herramientas, resultados. Nada sale a la red.
- **Episodios.** Cada objetivo con su secuencia de pasos y su resultado (pasó o falló) es un episodio. De la secuencia queda la versión mínima que llegó al resultado.
- **Solución vigente.** Entre las secuencias que resolvieron un mismo tipo de objetivo, compiten por éxitos y fallos recientes. La que pierde se archiva; nunca se borra.
- **Fallos tardíos.** Si un arreglo posterior señala, con `git blame`, al commit que hizo un episodio, podés confirmarlo y eso le resta evidencia.
- **Varios hosts.** Claude Code está probado. Codex, OpenCode y modelos locales tienen adaptador, sin probar contra el host real.
- **Colaboración entre agentes.** Canales de mensajes, proyectos con varias carpetas y roles con procedimiento escrito.

Cada chat nace **apagado**: instalar Arandu no captura nada hasta que vos vinculás una memoria.

## Inicio rápido

Hace falta Rust estable y un compilador de C (SQLite se compila con el proyecto).

```bash
cd codigo
cargo build --release --locked
cargo test --locked --all-targets
```

Se compilan dos binarios del mismo programa: `arandu`, que acepta las órdenes cortas, y `msp`, el nombre anterior, que usan los hooks. Para tenerlos en el `PATH`: `cargo install --path codigo --locked`.

## Órdenes cortas

Como git: un verbo, y el proyecto y la memoria salen de la carpeta donde estás.

```bash
cd mi-proyecto
arandu init                  # registra esta carpeta como proyecto
arandu new mi-memoria        # crea una memoria y muestra su ID
arandu use ID --record       # la deja como predeterminada del proyecto, en solo registro
arandu best "compilar el módulo"   # la solución vigente para ese objetivo
arandu goals                 # todos los tipos de objetivo del proyecto
arandu blame                 # rastrea el último commit como arreglo de un error
```

`arandu` sin argumentos lista todas. Las que hablan de un chat (`status`, `say`, `inbox`, `role`, `pass`, `rules`) necesitan saber cuál: `-s SESION` o la variable `ARANDU_SESSION`. La forma larga (`arandu --control ORDEN --opcion valor`) sigue funcionando y es la referencia completa.

Con Claude Code, en Windows:

```powershell
cd codigo
./claude-plugin/instalar.ps1
claude --plugin-dir "$PWD\claude-plugin"
```

Dentro del chat, las órdenes empiezan con `msp::` y sólo valen si las tipeás vos:

```text
msp:: crear mi-memoria      crea una memoria y muestra su ID
msp:: proyecto              registra la carpeta actual como proyecto
msp:: registrar ID          enciende en «solo registro»: captura, no ofrece nada
msp:: usar ID               enciende la memoria activa: captura y ofrece antecedentes
msp:: estado
msp:: apagar
```

Empezá por `registrar`: junta datos sin cambiar lo que hace el agente.

## Documentación

| Documento | Para qué |
|---|---|
| [`codigo/COMANDOS.md`](codigo/COMANDOS.md) | Comandos para probar cada función a mano, con lo que tenés que ver. |
| [`codigo/README.md`](codigo/README.md) | Referencia técnica completa: contrato de eventos, esquema, reglas y sus límites. Es largo y está ordenado por entrega. |
| [`HOJA-DE-RUTA.md`](HOJA-DE-RUTA.md) | Qué está hecho, qué falta y qué queda por decidir. |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Cómo proponer cambios y qué pruebas se piden. |
| [`SECURITY.md`](SECURITY.md) | Qué protege, qué no, y cómo avisar de un problema. |
| `erd-y-esquema-fisico.md`, `mecanismo-aprendizaje-v1.md` | Diseño normativo del esquema y del mecanismo. |
| Los demás `.md` de la raíz | Antecedentes de investigación; no rigen la implementación. |

## Lo que todavía no es cierto

- **No está demostrado que mejore los resultados.** Falta la comparación con uso real entre memoria encendida y apagada.
- **«Mismo objetivo» es «mismas palabras».** No hay comprensión semántica, y esa regla no se evalúa.
- **La CLI no autentica a quien la invoca.** Cualquier proceso local con acceso a la carpeta de datos puede actuar como otro chat.
- **La memoria puede reforzarse a sí misma:** ofrece una solución, se la sigue, y eso cuenta como evidencia. Se mide (`urna` en `vigente-consultar`), todavía no se corrige.

## Licencia

Apache-2.0. Ver [`LICENSE`](LICENSE) y [`NOTICE`](NOTICE).
