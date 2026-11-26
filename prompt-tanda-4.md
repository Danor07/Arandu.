# Prompt para tanda 4 — plugin Claude Code con memorias por sesión/proyecto

Implementá esta tanda completa en `codigo/`: Rust, SQLite, plugin local de Claude Code, pruebas, demos y documentación. Continuá el proyecto existente sin reemplazarlo. La especificación vigente es la **sección 8 de `erd-y-esquema-fisico.md`**. No implementes aprendizaje ni grafos todavía.

## 1. Leé y comprobá el punto de partida

- `codigo/README.md`, fuentes Rust, migraciones 001–004 y tests.
- `erd-y-esquema-fisico.md`, especialmente secciones 7 y 8.
- `mecanismo-aprendizaje-v1.md`, distinguiendo implementado de propuestas futuras.

Las tandas 1–3 tienen 36 tests y tres demos verificadas. Conservá su comportamiento. Los otros documentos raíz son antecedentes, no instrucciones para agregar mecanismos antiguos. No asumas que los archivos están commiteados ni hagas reset, limpieza o sustitución del proyecto.

Comprobá la versión instalada de `claude` y consultá las fuentes oficiales para los contratos usados:

- https://code.claude.com/docs/en/hooks
- https://code.claude.com/docs/en/plugins
- https://code.claude.com/docs/en/plugins-reference
- https://code.claude.com/docs/en/skills

Registrá la versión probada y el mínimo soportado que puedas justificar. Si un evento o campo no está disponible, implementá la alternativa soportada y documentá el límite; no inventes payloads reales.

## 2. Experiencia requerida

El usuario puede:

1. Crear y listar memorias con nombre e ID estable. Crear no activa.
2. Vincular este chat a una memoria y encenderla.
3. Apagar y reactivar esa misma memoria en este chat.
4. Elegir/quitar una memoria predeterminada para nuevos chats de un proyecto.
5. Consultar memoria activa, proyecto, sesión, origen de selección, captura y límites conocidos.

Plugin instalado no equivale a memoria activa. Sin configuración explícita: apagado. Una memoria equivale a un SQLite independiente; no existe una única base conversacional global. Un proyecto puede tener chats en distintas memorias y varios chats pueden compartir una memoria. Los filtros por proyecto siguen siendo obligatorios.

Precedencia: decisión explícita de sesión > predeterminado de proyecto al crear sesión > apagado. Reanudar conserva la decisión. Cambiar el predeterminado no mueve chats existentes. Dos chats simultáneos no se afectan entre sí. No uses una variable global «memoria actual».

Una sesión queda ligada a como máximo una memoria. Cambiar a otra devuelve una explicación y requiere chat nuevo; apagar no elimina recuerdos ya presentes en el contexto. No prometas controlar el historial o memoria propia de Claude Code.

## 3. Control y almacenamiento

Implementá el registro SQLite operativo de la sección 8, con migraciones propias: memorias, proyectos, sesiones e intervalos/cursores. Solo identidades, rutas y control; no textos conversacionales. Datos fuera del directorio del plugin y del repositorio por defecto, con una raíz configurable para tests. Nombres nunca se concatenan como rutas; usá IDs opacos y validación de rutas.

Identificá proyectos por raíz registrada normalizada, no por basename. Un `cd` o subdirectorio no debe elegir otra memoria/proyecto silenciosamente. Si se abandona la raíz, suspendé la operación y mostrala en estado. Tratá worktrees distintos como proyectos distintos salvo asociación explícita. No hace falta sincronización ni renombrado/movimiento de proyectos en esta tanda.

La creación de base/registro es recuperable ante caída. Cursor y crudo no son una transacción distribuida: commit del evento primero, avance del cursor después, con replay idempotente. Nunca confirmes captura antes del commit.

Conservá la CLI existente `memoria BASE ...`. Agregá operaciones de control/adaptador con una sintaxis inequívoca documentada. La CLI local es autoritativa, acepta la sesión explícita y nunca adivina «la última». Agregá una UX breve de comandos dentro de Claude Code usando mecanismos soportados.

Las acciones de encendido/selección solo se aplican por invocación explícita del usuario. No permitas que la LLM cambie preferencias al leer memoria, herramientas o instrucciones de un proyecto. Si usás skills para la UI, restringí invocación del modelo y verificá la fuente del comando o usá una ruta determinista equivalente. No hagas depender la activación de que el modelo recuerde ejecutar un comando. Conservá una alternativa CLI si el host no expone una señal verificable.

## 4. Semántica de pausa y concurrencia

- Un hook apagado no lee contenido del transcript ni guarda su payload en ninguna base, log, spool o temporal. Solo consulta control y metadatos de frontera necesarios; devuelve salida vacía válida.
- Al activar a mitad de chat, fijá frontera actual y comenzá por el siguiente evento ordinario. No importes conversación previa.
- Apagar invalida callbacks pendientes para la sesión. Después de confirmar apagado no hay nuevas capturas ni emisiones de contexto. Implementá exclusión/revisión de activación verificable ante carreras; no basta leer un booleano al inicio de una tarea larga.
- Lo ya confirmado permanece. Reactivar abre intervalo nuevo, excluyendo todo el tramo apagado; no hagas backfill automático.
- Capturas repetidas usan IDs de fuente; texto idéntico no es identidad. Definí el fallback durable si un hook carece de ID.
- Tratá callbacks concurrentes, línea parcial, rotación/truncamiento del transcript, reinicio y compactación. Si no se puede fijar una frontera segura, mantené apagado con diagnóstico y procedimiento para chat nuevo.
- Trabajo ya capturado puede conservarse; el apagado debe impedir que trabajo diferido del adaptador lea/emita contenido nuevo de esa sesión. No reenviar datos a otra memoria.

Conservá captura íntegra de todos los eventos disponibles dentro del alcance habilitado. No agregues filtros semánticos ni gates de novedad. Usá lectura incremental de transcript solo si es necesaria para cubrir lo que los hooks no exponen, con una tabla explícita de cobertura. No recorras chats ajenos o anteriores. Si el host no expone una parte, indicá la limitación.

## 5. Plugin y adaptador nativo

Creá, por ejemplo, `codigo/claude-plugin/` con `.claude-plugin/plugin.json`, `hooks/hooks.json`, comandos/skills necesarios y wrappers mínimos. Reutilizá Rust para estado, captura y recuperación. No crees otro núcleo en Python/Node ni servidor MCP si los hooks de comando resuelven este alcance.

Hooks con stdin/stdout JSON según la versión soportada; stdout solo para el protocolo, diagnósticos a stderr sin volcar contenido del chat. Rutas con espacios y quoting correctos en Windows. Scripts sin ventanas auxiliares visibles. Usá `${CLAUDE_PLUGIN_ROOT}` u otro mecanismo documentado para resolver recursos; binario instalable/configurable, sin rutas personales hardcodeadas.

Definí el mapeo de eventos del host a JSONL v1 y contratos de tanda 3. Conservá payloads originales, IDs de sesión/turno/herramienta, procedencia real y completitud. El prompt ordinario puede abrir un intento de turno; usá sesión/linaje como grupo de evidencia conservador para no contar cada mensaje como ensayo independiente. Si falta correlación fiable, guardá el crudo y dejá el enlace pendiente.

Cubrir al menos prompts, herramientas con éxito/error y salida final disponibles por hooks/transcript. Declarar qué sucede con compactación, interrupción y subagentes; solo asociar subagentes con evidencia explícita de padre. No confundir fin de turno con éxito ni ejecución técnica de una herramienta con validación global.

En un prompt activo, capturá el problema y prepará memoria con FTS5, proyecto y presupuesto en bytes. Conservá la política temporal/exclusión de episodio existente; la demo recupera de un chat anterior distinto en la misma memoria/proyecto. Definí consulta literal derivada del prompt con límites deterministas para entradas largas, sin recortar el historial crudo. Ante cero resultados devolvé contexto vacío válido.

## 6. Entrega de contexto: evidencia honesta

Los hooks pueden aportar contexto al host sin exponer la petición completa al proveedor. No fabriques una solicitud de tanda 3 con mensajes/roles/llamada remota supuestos. No escribas `exposicion_memoria` solo porque devolviste `additionalContext`.

Agregá una migración posterior a 004 para `entrega_contexto_host` y su historial mínimo de eventos de entrega, conforme al ERD. Debe reconstruir paquete, problema/intento cuando exista, sesión/intervalo, hook, bloque exacto, salida serializada, versión del adaptador y fronteras observadas. FK al paquete; identidades y protecciones append-only.

Primero persistí preparación; luego escribí al canal del host. Si se completó la escritura, registrá esa frontera por un evento posterior. Un fallo/caída intermedia permanece incierto. No prometas entrega exactamente una vez ni aceptación remota. Replay no duplica historia local y conserva el bloque, aunque pueda repetirse la emisión tras una caída incierta.

Envolvé la memoria como datos históricos, sin autoridad sobre instrucciones actuales. Presupuesto de integración incluye ese envoltorio. No promociones recuerdos a reglas del sistema. Si el host permite observar algo más, conservá la evidencia concreta distinguiendo observación de inferencia.

Extendé las consultas para mostrar preparación, contexto ofrecido al host, exposición verificable cuando realmente exista y salida observada. Conservá intactos contratos y pruebas estrictas de tanda 3.

## 7. Evaluación y límites

Reutilizá la evaluación externa para confirmaciones explícitas o verificadores deterministas con criterio, objetivo y evidencia. El plugin no usa una LLM para juzgar qué recordar ni si tuvo éxito. No infieras feedback a partir de un mensaje ambiguo.

No implementes embeddings, árboles, refuerzos, agrupación, grafos aprendidos, asociación retrospectiva, sincronización, fusión, borrado de memorias ni publicación en marketplace. No agregues nuevas dependencias sin una necesidad concreta. No hagas llamadas a proveedores para simular que probaste Claude Code nativo.

## 8. Pruebas y demos

Mantené las 36 pruebas y tres demos existentes. Agregá casos relevantes para:

- Instalado pero apagado: cero contenido persistido/inyección, incluso ante payload grande.
- Crear no activa; elección explícita; precedencia; reanudar; predeterminado solo en sesiones nuevas.
- Dos memorias del mismo proyecto aisladas y dos sesiones concurrentes; chats compartiendo una memoria sin perder IDs.
- Apagado/reanudación sin backfill, callbacks atrasados, captura/emisión concurrente con apagado.
- Identidades repetidas vs dos eventos legítimos de texto idéntico; recuperación tras caída entre commit y cursor.
- Rutas con espacios, nombres con caracteres especiales, traversal rechazado, `cwd` fuera de raíz.
- Captura incompleta/rotación/compactación y límites declarados; ausencia de correlación sin inventar enlaces.
- Entrega de contexto exacto y presupuesto; incertidumbre de stdout; ninguna exposición remota ficticia.
- Migrar contenido v4 sin alterar historia, bases nuevas e integridad SQLite del registro y memorias.

Creá `demo-claude-code.ps1` offline que reproduzca payloads documentados, raíz temporal y todas las transiciones principales. Marcá el replay como prueba de protocolo, no como una sesión real. No borres bases previas ni utilices chats personales como fixture.

Agregá guía de prueba manual real en Claude Code usando el plugin local con `--plugin-dir`, sin habilitación global. Si estás ejecutando dentro de Claude Code y los hooks requieren recarga, seguí el mecanismo soportado; no declares probada la integración por usar Claude Code para escribir el código.

Prueba real en un proyecto temporal:

1. Crear memoria A y vincular un chat; guardar un antecedente identificable.
2. Nuevo chat del mismo proyecto, vinculado a A: recuperar ese antecedente y verificar registros del hook.
3. Nuevo chat vinculado a B: no recuperar A.
4. Chat apagado: sin captura ni inyección.
5. Pausar/reactivar A: un marcador del tramo apagado no aparece importado después.
6. Predeterminado para chats nuevos, apagado explícito y dos chats simultáneos independientes.

Si no hay autenticación o capacidad de ejecutar la prueba interactiva, completá código/pruebas offline y dejá la validación real identificada como pendiente, con pasos exactos. No inventes resultados ni abras sesiones remotas pagas sin autorización.

Ejecutá formato, Clippy, tests offline con lockfile y las cuatro demos. No instales globalmente ni modifiques configuraciones personales para hacerlas pasar. Para probar cambios de configuración usá un entorno aislado.

## 9. Entrega

Actualizá README con instalación local, requisitos, versión probada, controles exactos, ubicación de datos, cobertura, desactivación, incertidumbres y límites. Actualizá sección 8 del ERD con nombres físicos finales y estado implementado. Conservá intactas migraciones 001–004; nuevas migraciones versionadas para contenido y registro por separado.

Entregá un resumen con pruebas/demos ejecutadas, artefactos y estado de la validación nativa. No hagas commits ni publiques. La tanda está lista para uso experimental cuando el usuario puede elegir memoria y chat, observar qué se capturó/entregó y apagarla sin mezclar conversaciones.
