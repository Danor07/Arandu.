# Prompt para Codex — tanda 3: ejecución, exposición y evaluación externa

Implementá la tanda 3 de este proyecto en `codigo/`, continuando las tandas 1–2 existentes. Completá código, migración, pruebas, demo y documentación. No te quedes en un plan. No implementes el aprendizaje futuro descrito en los documentos.

## 1. Contexto y lecturas

Leé, en este orden:

1. `codigo/README.md` y `codigo/Cargo.toml`.
2. `erd-y-esquema-fisico.md`, especialmente **sección 7: Extensión de tanda 3**. Es la especificación vigente para este circuito.
3. `mecanismo-aprendizaje-v1.md`: restricciones de captura, evidencia externa y separación entre recurrencia, utilidad y condiciones.
4. Las migraciones 001–003, `codigo/src/`, los tests y `codigo/demo-recuperacion.ps1`.

Los otros documentos raíz son antecedentes históricos. Las conversaciones sobre grafos figuran ahora en los dos documentos normativos; no necesitás acceso al chat para implementar esta tanda.

Estado inicial verificado el 2026-09-16: 14 tests de persistencia y 10 de recuperación pasaron, junto con formato, Clippy y la demo de recuperación. Verificá nuevamente al trabajar sobre tu checkout. Conservá los archivos y cambios existentes; no asumas que están commiteados.

## 2. Objetivo observable

Reconstruir este circuito para un intento concreto:

**Problema → solicitud con o sin memoria → acciones y salida → evaluación externa bajo un criterio definido.**

Responder con evidencia:

- ¿Qué problema se intentó resolver y en qué proyecto/episodio?
- ¿Qué paquete se preparó y qué bloques se registraron como enviados, en qué llamada?
- ¿Hay respuesta enlazada a esa llamada, o la recepción sigue sin confirmarse?
- ¿Qué acciones, pruebas, correcciones y resultados pertenecen explícitamente al intento?
- ¿Quién evaluó qué, con qué criterio y qué evidencia?
- ¿Qué sigue desconocido o pendiente de enlace?

La tanda prueba persistencia y trazabilidad. No prueba que la memoria causó un resultado ni que disminuyó el análisis de una LLM.

## 3. Persistencia y compatibilidad

Creá una migración nueva, por ejemplo `004_ejecucion_evaluacion.sql`. No edites 001, 002 o 003, sus hashes ni las bases de demos anteriores. No levantes los triggers que sellan paquetes o protegen historiales.

Implementá las cuatro entidades de la sección 7 del ERD:

- `ejecucion`: identidad del intento, problema, proyecto y episodio.
- `ejecucion_evento`: enlace posterior explícito de eventos capturados al intento.
- `exposicion_memoria`: paquete íntegro observado en una solicitud enviada, con texto, posición y evidencia exactos.
- `evaluacion_ejecucion`: evaluación externa del intento o de un evento del intento, independiente de que exista memoria.

Concretá el SQL siguiendo sus invariantes. FK/UNIQUE/CHECK para invariantes locales, validación transaccional en Rust para relaciones cruzadas y triggers append-only. Agregá índices solo para las consultas implementadas. No introduzcas un ORM, motor de grafos, servicio externo o framework de plugins.

`evento_feedback` existente exige recuperación y apunta a versiones generales; queda intacto y sin escritor nuevo en esta tanda. No fabriques contextos ni recuperaciones para evaluar un intento sin memoria. No dupliques una evaluación en dos tablas. No escribas `salida_instancia_id` sobre una recuperación sellada: la salida pertenece a la trayectoria enlazada.

El historial capturado sigue siendo la fuente. Los nuevos registros referencian eventos crudos. Reenviar la misma identidad con el mismo contrato devuelve el mismo resultado; identidad repetida con datos distintos es conflicto. Definí y documentá la igualdad para los nuevos comandos; mantené sin cambios la igualdad por bytes de JSONL v1.

## 4. API y CLI mínimas

Implementá operaciones para:

1. Registrar un intento a partir de un problema ya capturado.
2. Enlazar eventos ya capturados al intento, sin editar `evento_uso` anterior.
3. Registrar exposición de un paquete en una llamada explícita.
4. Registrar evaluación externa, incluyendo correcciones append-only.
5. Consultar el intento completo y consultar paquetes sin exposición registrada.

Usá argumentos breves y archivos JSON para contratos extensos; fijá nombres concretos y ejemplos en el README. Podés agrupar operaciones de escritura en un comando de importación de registros si reduce duplicación. Separá claramente captura cruda de interpretación y enlace.

Una referencia ausente o incompatible hace fallar esa operación de enlace con un error claro y sin filas parciales. El evento crudo válido previamente capturado permanece. Su sobre conserva las referencias pendientes. Al llegar el destino, reintentar el enlace debe funcionar sin duplicación. No hace falta un nuevo worker ni reintentos automáticos para esta tanda.

Validá proyecto/episodio/problema, pertenencia de objetivos al intento y consistencia con `evento_uso.intento_clave` cuando exista. Sin proyecto explícito, conservá el crudo y rechazá el enlace hasta tener información suficiente; no heredes proyecto por parecido o por nombre de clave. No exijas que todos los eventos del intento tengan `uso`: solicitudes y respuestas también pueden enlazarse.

Consultas ordenadas por secuencia declarada de origen, con identificadores, procedencia, contenido original accesible y referencias resueltas/pendientes. Diferenciá fecha del evento y fecha de registro/enlace. No derives causalidad del orden temporal.

## 5. Contrato de exposición

Definí un contrato pequeño y versionado que el adaptador pueda aportar dentro de los eventos crudos existentes. Debe conservar:

- ID de llamada y procedencia `real` o `simulada`.
- Instantánea de la solicitud disponible: mensajes con roles y contenido exacto; proveedor/modelo si se conocen.
- Frontera observada del transporte: solo una marca explícita de envío observado permite registrar exposición; preparación local o transporte incierto no bastan.
- Cada bloque de memoria con identidad del paquete, posición inequívoca dentro de los mensajes y texto exacto. Una representación simple es un mensaje dedicado por bloque, identificado por su índice; documentá la convención elegida.
- Referencias explícitas de respuestas a solicitudes. Una respuesta enlazada confirma que hubo respuesta a la llamada, no que el modelo aplicó una memoria.

La fuente que observa el envío aporta la evidencia; el núcleo registra y valida su consistencia, sin afirmar que autentica al adaptador. No aceptes una afirmación genérica de «usé memoria» como prueba del cuerpo enviado.

En esta tanda solo admití bloques completos e idénticos a `paquete_recuperacion.texto_memoria`. Verificá su posición y texto contra la solicitud cruda, no solo un hash o una coincidencia de substring. Paquete y ejecución deben compartir problema y proyecto. Un paquete puede reutilizarse en llamadas distintas del mismo problema; cada envío tiene identidad propia. Reenviar el registro de una llamada ya observada no produce otra exposición.

Una transformación, resumen o recorte queda conservado en el crudo pero no se registra como exposición del paquete intacto. El paquete vacío no cuenta como contenido de memoria. No modificar retroactivamente selecciones o candidatos de tanda 2. No registrar `search` o `detail` locales como exposición.

Evitá efectos remotos en la demo. No agregues llamadas pagas, API keys, descargas de modelos ni un adaptador específico de proveedor. Implementá un adaptador local simulado para probar el contrato; indicá expresamente el límite de esa prueba. Un envío de estado incierto no autoriza reenviarlo automáticamente.

## 6. Evaluación externa

Admití `usuario_explicito` y `verificador_externo`, resultado 0/1/NULL, criterio identificado y versionado, alcance `intento` o `evento`, evidencia fuente y objetivo cuando corresponda. Validá que la evidencia declarada provenga de usuario o herramienta según el contrato, no de autoevaluación del modelo. La procedencia simulada debe seguir visible y consistente.

Un verificador conserva entrada, salida y la regla que transforma esa salida en resultado. Ejecutá en la demo un verificador determinista local y registrá su salida real, incluso cuando la solicitud al modelo sea simulada. Distinguí esas dos cosas al explicar la demo.

No infieras éxito por silencio, por recibir respuesta, por terminar un proceso o por una frase del modelo. Una prueba solo evalúa su criterio y alcance. No distribuyas premios/castigos entre paquetes expuestos. No implementes q, refuerzos, árboles ni selección aprendida.

Conservá correcciones mediante referencias a evaluaciones anteriores; verificá mismo intento, criterio, alcance y objetivo, sin ciclos ni bifurcaciones. Mostrá las hojas vigentes de esas cadenas sin borrar el historial. Evaluaciones independientes incompatibles del mismo objetivo/criterio se muestran en conflicto, sin «gana la última». Una evaluación NULL no se convierte en éxito/fallo ni invalida por sí sola otra conocida. Un test inicial fallido y otro posterior exitoso, con objetivos diferentes, son trayectoria, no conflicto.

Las métricas de uso siguen separadas: valor, unidad, alcance y método cuando existan. No inventes ceros ni sumes duraciones superpuestas. Varias pruebas de un intento no son episodios independientes.

## 7. Acciones y grafos: qué conservar y qué postergar

Conservá datos estructurados de actor, acción, objeto, rol vigente, momento y referencias tipadas cuando la fuente los entregue. Aprovechá `metadatos` y `referencias` del JSONL actual; no agregues una ontología universal ni deduzcas hechos de prosa.

Incluí en un fixture pequeño el caso:

- Carla abre un turno.
- Carla realiza una cancelación.
- Juan cierra el turno.

La reconstrucción debe conservar quién ejecutó cada acción y a qué turno corresponde. No atribuir la cancelación a Juan por cerrar ni equiparar autoría con supervisión. No hace falta un motor que conteste preguntas ni una solución general aprendida para demostrarlo.

Quedan fuera de esta tanda: extracción semántica automática, comparación de grafos, generalización de patrones, asociación retrospectiva, migración de pertenencias, promoción de contextos, embeddings, EMA y entrenamiento. La estructura capturada permitirá explorarlos después.

## 8. Demo reproducible

Agregá `codigo/demo-ejecucion.ps1` y fixtures propios, usando una base nueva por ejecución. No cambies los fixtures históricos para ocultar problemas de compatibilidad.

Demostrá:

1. Un antecedente previo y un problema nuevo; preparación de un paquete.
2. Un intento con ese paquete en una solicitud simulada registrada como enviada; respuesta simulada explícitamente enlazada; verificador local y evaluación de su objetivo.
3. Un intento sin memoria con evaluación externa válida, sin recuperación artificial.
4. Otro paquete preparado sin exposición registrada.
5. Un envío observado sin respuesta ni evaluación: estado desconocido, nunca éxito automático.
6. Reapertura y repetición de registros: misma reconstrucción, sin duplicados.
7. El caso de autoría Carla/Juan, conservando las relaciones declaradas.

El script debe verificar los resultados y fallar con código distinto de cero ante incumplimientos. Guardá la reconstrucción JSON en `demo-output/` y emití un resumen legible con sus límites. No afirmes que comparaste rendimiento de una LLM con/sin memoria ni que demostraste aprendizaje.

## 9. Pruebas de aceptación

Además de mantener las 24 pruebas existentes, cubrí con casos significativos:

- Migrar una base creada con 001–003 con eventos y un paquete existente; conservar intactos crudo y paquete.
- Idempotencia y conflicto de identidad tras reabrir para los nuevos escritores.
- Rollback ante fallo inyectado entre escrituras de una operación; crudo previo intacto.
- Rechazo de enlaces entre proyectos, episodios o intentos incompatibles.
- Referencia desordenada: crudo preservado, enlace pendiente visible, reintento correcto tras llegada del destino.
- Exposición válida; rechazo de texto/posición/problema incompatibles, simple preparación, evidencia de envío ausente y procedencia inconsistente.
- Llamadas distintas vs reenvío del mismo registro; varias exposiciones no generan atribución causal.
- Intentos sin memoria, paquetes sin exposición y ejecuciones sin respuesta/feedback.
- Evaluación por criterio y objetivo, resultado NULL, correcciones históricas y conflictos independientes; rechazo de autoevaluación del modelo como evidencia externa.
- Triggers de inmutabilidad, `foreign_key_check` e `integrity_check`.
- CLI en procesos separados y demo reproducible. El fixture Carla/Juan preserva actores y relaciones sin inferencias nuevas.

Corré desde `codigo/`:

```text
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets -- -D warnings
cargo test --offline --locked --all-targets
```

Ejecutá las demos de persistencia, recuperación y ejecución. Preferí las dependencias ya disponibles. Si hubiera una necesidad técnica concreta de otra dependencia, justificá el cambio y respetá las restricciones de red del entorno.

## 10. Entrega

Actualizá `codigo/README.md` con contratos, comandos completos, ejemplo mínimo, demo y limitaciones. Actualizá la sección 7 del ERD con nombres finales, restricciones y referencia a la migración implementada; mantené separados estado implementado y aprendizaje futuro. Si debés ajustar el diseño lógico para resolver una incompatibilidad real, documentá la razón y conservá los invariantes de captura, historial y atribución.

Dejá un resumen final de lo implementado, pruebas y demos ejecutadas, rutas de salida y pendientes reales. No hagas commits, publicación ni integración remota como parte de esta tanda.
