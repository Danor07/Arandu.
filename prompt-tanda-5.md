# Tanda 5 — plan y prompt: tareas medibles y evaluación de memoria acumulada

## Plan acordado

**Objetivo:** preparar el plugin para instrumentar un proyecto real grande, conservar evidencia por tarea y comparar utilidad de memoria inicial/acumulada. No implementar aprendizaje de soluciones generales todavía.

| Paso | Entrega | Cómo se acepta |
|---|---|---|
| 1. Tareas y criterios | Identidad de tarea independiente de turnos/chats, definición versionada y selección explícita por sesión | Una tarea reúne eventos de varios chats sin cambiar sus identidades o inventar correlación de intentos. |
| 2. Resultados e informes | Feedback externo por criterio; informe JSON/Markdown con métricas y cobertura | Cada resultado y cifra tiene fuente; cerrar no equivale a éxito; desconocido no es cero. |
| 3. Comparación sin recuperación | Modo optativo solo registro | Captura igual que el modo activo, pero no busca ni entrega recuerdos; apagado continúa sin capturar. |
| 4. Instantáneas y corridas | Snapshot consistente, manifiesto y copias aisladas por caso/condición | El original no cambia y una prueba no alimenta la siguiente ni la memoria de producción. |
| 5. Validación | Regresiones, demo offline, guía/prueba nativa y tabla comparativa descriptiva | Se distingue prueba mecánica de evidencia de beneficio; no se fabrican resultados del proyecto real. |

El otro proyecto no es necesario para implementar esta tanda. Antes de usarla, su responsable deberá aportar tareas, criterios verificables y estados iniciales del código. La plantilla del final sirve para cargar esos datos.

---

## Prompt de implementación

Implementá la tanda 5 completa en `codigo/`, incluyendo migraciones nuevas, Rust, controles del plugin, pruebas, demo y documentación. Seguí la **sección 9 de `erd-y-esquema-fisico.md`**. Completá la instrumentación; no la sustituyas por un plan ni ejecutes el hand-off de otro proyecto.

### 1. Contexto y límites

Leé `codigo/README.md`, `erd-y-esquema-fisico.md` (secciones 7–9), `mecanismo-aprendizaje-v1.md`, las fuentes y tests actuales. La tanda 4 tiene 49 pruebas documentadas y cuatro demos; verificá el checkout real. Su prueba nativa fue reportada por el usuario, no es una prueba de aprendizaje.

Conservá las migraciones de contenido 001–005 y la migración 001 del registro de control. Agregá versiones nuevas donde hagan falta. No reescribas crudo, paquetes, asignaciones, entregas, evaluaciones o identidades antiguas. No hagas commits ni publicación. No presupongas un repositorio limpio ni borres artefactos anteriores.

Fuera de alcance: embeddings, grafos aprendidos, árboles, agrupación semántica, promoción/refuerzo, asociación retrospectiva, juez LLM, sincronización, dashboards y ejecución automática de campañas remotas. Esta tanda mide reutilización de antecedentes con el algoritmo actual.

### 2. Verificación breve de observabilidad

Revisá qué campos usa realmente el adaptador de la versión probada. El código actual depende de `prompt_id` para correlacionar intentos y usa hash de payload como fallback de identidad. Probá eventos legítimos repetidos sin ID, replay exacto y cambios de payload de una misma operación.

Si hay IDs de fuente fiables, usalos con su contrato. Si falta una distinción observable, conservá una advertencia de cobertura en el informe: no inventes un ID temporal para fingir exactamente una vez ni declares completo un contador ambiguo. Evitá una reescritura del capturador: corregí solo lo necesario para mediciones honestas.

No amplíes obligatoriamente a lectura de transcript. Conservá cobertura declarada de hooks. Un evento sin enlace a intento puede vincularse a una tarea por selección explícita del usuario, sin afirmar una relación causal ni inventar un turno.

### 3. Tareas explícitas

Implementá el modelo de sección 9: `tarea`, `tarea_evento`, `tarea_vinculo`, `tarea_resultado` (nombres finales documentados). Las descripciones, criterios y resultados viven en la base de memoria; el registro operativo solo contiene selección/revisión por sesión y modo.

Operaciones mínimas mediante CLI y el canal de usuario `memoria::` existente:

- Crear tarea desde JSON con clave, descripción y criterios identificados/versionados.
- Seleccionar/quitar tarea en una sesión; consultar estado.
- Registrar inicio, pausa, reanudación, cierre, ajuste o cambio de requisitos explícitos.
- Revisar definición/criterios con una nueva versión.
- Enlazar un evento anterior explícitamente, con motivo/evidencia, sin modificar su crudo.
- Registrar evaluación externa y correcciones históricas.
- Generar informe de tarea/proyecto con corte de datos.

Elegí nombres breves y coherentes y documentá comandos completos. Una selección de tarea no habilita captura ni cambia la memoria de la sesión. Solo una tarea activa por sesión, varias sesiones pueden seleccionar la misma tarea de la misma memoria/proyecto. No agregar una tarea global para todos los chats.

Congelá tarea/definición/revisión cuando se admite el callback. No consultes simplemente «tarea actual» al finalizar procesamiento. Si llega una salida tardía sin correlación segura, preservá crudo y pendiente; no la asignes silenciosamente a la nueva tarea. Mantené trazabilidad de la selección explícita y del enlace manual.

No alteres `grupo_evidencia` ni las restricciones de ejecución de tanda 3. Una tarea puede agrupar varias ejecuciones y sesiones; ninguna de ellas se transforma por eso en múltiples ensayos independientes. Si falla el enlace, el crudo ya capturado permanece y puede repararse idempotentemente.

### 4. Criterios y evaluación

Un criterio tiene ID, versión, descripción/contrato y carácter obligatorio u opcional. Sus revisiones no cambian resultados previos. Cerrar la tarea es estado operativo, no confirmación de calidad.

Reutilizá validación externa de tanda 3: fuente explícita de usuario o verificador determinista, evidencia original, alcance/objetivo y resultado 0/1/NULL. El modelo no interpreta libremente mensajes como éxito. Para evaluar una tarea sin ejecución correlacionada, usá alcance de tarea y evidencia dedicada; no fabriques una recuperación ni ejecución.

Si existe una `evaluacion_ejecucion`, referenciala como la misma observación; no copies su voto y lo sumes otra vez. Validá tarea/proyecto/objetivo/definición y guardá correcciones append-only. Mostrá conflictos independientes y falta de evidencia. Una corrección NULL no crea éxito/fallo nuevo: distinguí su resultado del antecedente conocido si se muestra. No escondas esa distinción detrás de un estado global.

Reutilizá lógica común sin duplicar un segundo evaluador. Ningún resultado modifica ranking, pesos o reglas. Un error de herramienta no se convierte automáticamente en fracaso de tarea; otro mensaje no es automáticamente una corrección humana.

### 5. Informe verificable

JSON como salida estructurada y Markdown legible, generados desde una instantánea consistente. Incluí tarea/definición, criterios, estados, sesiones, intentos conocidos, enlaces pendientes, configuración/versiones y corte. Cada métrica incluye unidad, alcance, método y referencias de evidencia.

Mínimos:

- Resultados por criterio: conocidos, desconocidos y conflictos; resumen «todos los obligatorios verificados» solo con evidencia suficiente.
- Llamadas de herramientas observadas, deduplicadas por operación, con resultados técnicos separados. No llamarlas llamadas al modelo. Donde falte identidad, declarar cobertura parcial.
- Ajustes/correcciones/cambios de requisitos explícitos, sin clasificarlos mediante una LLM.
- Paquetes preparados, entregas lógicas completadas/inciertas y bytes exactos; no confundir escritura al host con aplicación causal o recepción remota.
- Ventanas de trabajo explícitas y duración por unión de intervalos solapados. Pausas/reanudaciones y intervalos abiertos visibles. No denominarla tiempo de razonamiento o tiempo humano activo.
- Tokens solo si hay medición fuente fiable con alcance. En otro caso `null` con motivo. No estimarlos a partir de bytes y no sumar contadores acumulativos como incrementos.

No inventes un score único de aprendizaje/productividad. No agregues métricas de tareas incomparables como si midieran lo mismo. La ausencia de observaciones no demuestra que no ocurrió actividad fuera de cobertura.

### 6. Modo solo registro

Separá explícitamente tres modos visibles: apagado, solo registro, memoria activa. Podés extender el modelo de control con un flag de recuperación si mantiene clara la semántica; no reescribas el historial previo para simular que ya existía.

Solo registro requiere orden explícita del usuario: conserva captura/instrumentación, no ejecuta búsqueda, no crea paquetes para ese prompt ni escribe contexto. Apagado sigue sin leer/capturar nuevos contenidos. Seleccionar tarea o preparar comparación no enciende una sesión existente.

Preservá revisión/bloqueo al cambiar de modo, aislamiento de sesiones, reanudación y ausencia de backfill. Un chat donde ya se inyectó memoria no sirve como control limpio aunque luego se elija solo registro: la herramienta de comparación exige chat nuevo.

### 7. Snapshots consistentes

Implementá snapshot con Online Backup API de SQLite o `VACUUM INTO`; no copies solo el `.sqlite` mientras WAL está activo. Consulta primaria: https://www.sqlite.org/backup.html. Conservá dependencias actuales cuando sea posible; una feature de `rusqlite` para backup es aceptable si se justifica.

Artefacto completo: base cerrada e íntegra + manifiesto versionado con ID/hash, memoria origen, migraciones, configuración, corte obtenido de la copia y procedencia. Sin sobrescribir destinos. Una caída deja un temporal incompleto identificable, nunca un snapshot final aparentemente válido. Validá integridad y hash antes de preparar corridas. No migres/escribas el original del snapshot al leerlo para evaluar.

Los datos de proyecto necesarios se preservan en el manifiesto; no copies preferencias/sesiones del registro como configuración activa. Memoria inicial y final son capturas explícitas, no nombres que apuntan a una base viva.

### 8. Corridas aisladas y comparación descriptiva

Formato de manifiesto de casos: ID, prompt, criterios/versiones, estado inicial del código, configuración/modelo/herramientas conocidos y exposición previa a la solución. Se fija antes de observar resultados. Parámetros desconocidos se declaran, no se completan por suposición.

Comando de preparación: para cada caso/condición/repetición crea una raíz aislada y una copia de trabajo del snapshot elegido, más manifiesto de corrida con ID nuevo. No cambies `memoria_identidad` sellada: la copia conserva linaje dentro de otro registro aislado y la corrida tiene su propia identidad. No abras sesiones históricas ni hagas replay de trabajo pendiente copiado. Validá que el proyecto explícitamente asignado corresponde a los antecedentes; no mezcles proyectos para obtener resultados.

Cada corrida usa un chat nuevo y una sola sesión; los eventos nuevos se guardan en la copia para medirlos, pero la exclusión de episodio evita recuperarlos durante esa corrida. Siguiente caso/repetición empieza desde otra copia limpia. Filtrá informes por sesión/corte de corrida para no sumar métricas históricas del snapshot. Ninguna corrida muta el snapshot o la memoria de producción ni alimenta las siguientes.

Condiciones:

- `sin_recuperacion`: solo registro, preferentemente desde la misma instantánea inicial para igualar estado de instrumentación.
- `memoria_inicial`: recuperación normal desde el snapshot inicial.
- `memoria_acumulada`: recuperación normal desde el snapshot posterior.

Si la inicial está vacía, declaralo. El índice BM25 puede cambiar con las nuevas capturas locales; no afirmes que el ranking es una constante congelada. Conservá las entradas/configuración para reproducir cada corrida y mantené excluidos los nuevos antecedentes del episodio de evaluación.

Otro comando lee informes/manifiestos y genera tabla por caso/condición/repetición: calidad primero, esfuerzo observado después, cobertura y parámetros que difieren. Falta de corrida es pendiente. Si cambia criterio, estado inicial del código, modelo/configuración conocida o el modo a mitad del chat, marcar no comparable; nunca ocultarlo en un promedio. Una observación no demuestra mejora general ni significación estadística.

No automatices llamadas a Claude, compras, checkout/reset del proyecto externo ni ejecución de comandos arbitrarios del manifiesto. Entregá instrucciones para ejecutar casos en copias/worktrees autorizados. Si un caso o su solución aparece en el snapshot, marcar exposición previa; no presentarlo como transferencia a un problema nuevo. Conservá casos reservados fuera de la memoria de trabajo.

### 9. Pruebas y demo

Preservá toda la suite y las cuatro demos existentes. Agregá pruebas relevantes para:

- Tarea de varios turnos/chats; mismo proyecto/memoria; rechazo de enlaces incompatibles.
- Criterios versionados, cierre sin éxito, resultado externo sin ejecución ficticia, referencia sin duplicar evaluación, correcciones/NULL/conflictos.
- Revisión de selección y salida tardía; replay, rollback y crudo conservado si falla enlace.
- Dos tareas/sesiones concurrentes y métricas sin duplicación; ventanas solapadas/abiertas; tokens ausentes.
- Solo registro sin búsqueda/paquetes/inyección; apagado intacto; cambio de modo invalida comparabilidad.
- Snapshot con escrituras WAL y error/caída, hash/integridad, sin sobreescritura, original inmutable.
- Copias aisladas, nueva sesión/IDs de corrida, informes que excluyen métricas históricas, evaluación sin contaminación de otra corrida o producción.
- Tabla descriptiva con condición faltante, configuración desigual y exposición previa; no inventar datos.
- Migración v5 y control v1 preservando datos e invariantes, FKs/integridad y CLI en procesos separados.

Creá `demo-medicion.ps1`, con datos sintéticos etiquetados, raíz nueva y verificador determinista local. Mostrar tarea multisesión, criterio fallido/corregido, cierre sin evaluación, informe con campos desconocidos, snapshot inicial/final y preparación de tres condiciones aisladas. Los resultados sintéticos prueban el informe; no se presentan como mejora real de una LLM.

Corré `cargo fmt --all -- --check`, `cargo clippy --offline --locked --all-targets -- -D warnings`, `cargo test --offline --locked --all-targets` y las cinco demos. Documentá prueba nativa breve de selección de tarea y modo solo registro en Claude Code, con un proyecto temporal. Si no podés hacerla, dejala explícitamente pendiente con pasos exactos; completá todo lo demás.

### 10. Entrega y uso previo al proyecto

Actualizá README, sección 9 del ERD y el estado del mecanismo con nombres físicos finales, comandos ejecutables, ejemplos y limitaciones. No cambies las tandas anteriores retroactivamente para esconder incompatibilidades.

Incluí una guía breve para el usuario:

1. Crear memoria dedicada al proyecto.
2. Cargar unas pocas tareas con criterios verificables; no necesita descomponer todo el hand-off antes de empezar.
3. Definir casos reservados y guardar snapshot inicial.
4. Trabajar con tareas seleccionadas; registrar resultados y cambios de requisitos explícitos.
5. Guardar snapshots por hitos y al finalizar.
6. Ejecutar comparaciones en chats/copias nuevos sobre estados iniciales equivalentes.

Resumen final: qué quedó implementado, validación ejecutada vs pendiente, rutas de demo y qué datos debe completar el usuario. Esta tanda está terminada cuando la instrumentación es reproducible; el ensayo real y sus conclusiones suceden después.

## Plantilla conceptual para el proyecto del usuario

Convertí esta plantilla en los archivos JSON de ejemplo compatibles con los comandos finales:

```text
Proyecto: <identidad y raíz>
Memoria de trabajo: <ID>
Tarea: <clave y descripción>
Criterios obligatorios: <ID, versión, qué se verifica y con qué evidencia>
Estado inicial del código: <commit y cambios adicionales o artefacto reproducible>
Configuración: <modelo/versión conocidos, herramientas, instrucciones relevantes>
Caso reservado: <prompt y criterio; guardar fuera de la memoria de trabajo>
Snapshot inicial: <ID/hash>
Snapshot por hito/final: <ID/hash>
Resultados desconocidos: <dejar sin completar; nunca poner cero por defecto>
```

No necesitás acceso al hand-off privado para implementar contratos, pruebas y ejemplos sintéticos. No le pidas al usuario que lo entregue para poder terminar esta tanda.
