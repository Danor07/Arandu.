# Mecanismo v1 — memoria por casos y condiciones aprendidas con resultados

Fecha: 2026-09-16. Propuesta de implementación en Rust + SQLite. Complementa el [ERD](erd-y-esquema-fisico.md). No afirma que el algoritmo ya esté implementado ni que haya mostrado Aprendizaje II.

**Estado de implementación:** tandas 1–5 entregadas: captura durable, búsqueda FTS5, paquetes sellados, trayectoria/evaluación externa y plugin Claude Code por sesión/proyecto. El encoder, el presupuesto en tokens, los contextos aprendidos y los árboles siguientes son propuestas posteriores. La sección 9 del [ERD](erd-y-esquema-fisico.md) y el [plan y prompt de tanda 5](prompt-tanda-5.md) definen ese alcance, ya entregado: tareas explícitas, evidencia de resultados, informes con cobertura declarada, instantáneas consistentes y comparaciones aisladas. **Todavía no se implementa aprendizaje de reglas**: la tanda 5 instrumenta para poder medir, y el ensayo sobre un proyecto real sucede después.

El registro completo se aplica a los intervalos que el usuario habilita. Crear/instalar no activa captura. Apagar detiene captura y recuperación del plugin; reactivar no importa lo sucedido durante la pausa. Los comandos de control son deterministas y no decisiones de la LLM. Cada sesión conserva una única memoria elegida; para cambiar de memoria se inicia otro chat, porque apagar no borra contexto ya cargado. La pausa no altera los eventos previamente conservados ni el historial propio del host.

La tanda 5 agrega un modo optativo **solo registro** para medir sin ofrecer antecedentes; no reemplaza ni altera el significado de apagado. La tarea del usuario puede abarcar varios turnos y chats y no se cuenta como varios éxitos por tener más mensajes. La comparación entre memoria inicial y acumulada mide reutilización bajo casos/criterios comparables; un antes/después del proyecto entero no identifica por sí solo un efecto de memoria. Las evaluaciones no alimentan los snapshots ni el corpus de producción. Tokens, duración y llamadas se informan solo con alcance y evidencia disponibles, sin estimar razonamiento interno.

## 1. Qué aprende y quién evalúa

El mecanismo es **aprendizaje por casos con estimación de utilidad condicionada y árboles pequeños de decisión**. Un encoder fijo aproxima semejanza; un motor numérico aprende en qué condiciones una solución observada tuvo buenos resultados. No se consulta a una LLM para decidir importancia, éxito, consolidación, revisión o descarte.

Hay tres señales distintas:

- Recurrencia: cuántos episodios externos independientes exhiben el patrón. Detecta regularidad, incluso cuando nadie la verbaliza como importante.
- Utilidad: éxitos y fallos atribuibles a una versión aplicada bajo un criterio externo definido.
- Aplicabilidad: condiciones del problema que predicen diferencias en utilidad y restringen la recuperación futura.

Se registra todo aunque falten representación o feedback. Sin feedback siguen funcionando memoria episódica, semejanza y conteos; el sistema no inventa utilidad. En investigación abierta sin criterios verificables, el progreso de este componente será más lento y dependerá de confirmaciones/correcciones externas.

Gazzaniga describe un intérprete que construye explicaciones con acceso incompleto a los procesos que produjeron la conducta. La lección adoptada es separar narración de evidencia. No se deduce que usar lenguaje sea siempre deliberativo ni que un algoritmo no verbal sea equivalente a intuición humana. Aquí «intuitivo» significa operacionalmente que la adaptación ocurre sin una evaluación verbal de importancia por evento.

### Learning by using como disciplina de desarrollo

El principio específico de Rosenberg que adoptamos es aprender por uso: la aplicación real revela capacidades, ajustes necesarios y límites de una solución. En programación podemos conservar el circuito **problema → solución aplicada → resultado y ajustes → reutilización en otro caso**. Este es el primer hito de la v1. Los árboles y umbrales siguientes son hipótesis iniciales; no se supone que el mecanismo correcto esté resuelto antes de observar ese circuito funcionando.

Además del éxito/fallo, se guarda la trayectoria: intentos, correcciones, pruebas, duración y recursos disponibles. Una solución que termina bien después de tres ajustes aporta evidencia diferente de una que funciona directamente. Se mantienen dimensiones separadas, sin construir de entrada un puntaje único de calidad. Comparar esfuerzo exige tareas y resultados comparables: menor tiempo no demuestra mejora si cambió la dificultad.

## 2. Representar sin juzgar

### Encoder

Propuesta a medir: **BGE-M3 congelado, salida densa**, multilingüe y de 1024 dimensiones según su ficha oficial. Se probará en el equipo y corpus antes de fijar el artefacto final. El encoder produce vectores; no decide si una solución es verdadera, conveniente ni digna de memoria. Descartar MiniLM como juez no obliga a descartar todos los encoders como representación.

El motor Rust llama a un adaptador local de inferencia con revisión y hash fijados (por ejemplo, un artefacto ONNX validado contra la salida de referencia). No se presupone que cualquier exportación de terceros sea equivalente. No se descargó ningún modelo al escribir esta especificación. Latencia, RAM y calidad de recuperación deben medirse; el diseño no depende de la marca del encoder.

Representación inicial: texto del problema normalizado, segmentación determinista en ventanas de hasta 512 tokens con solapamiento de 64, vector normalizado de cada ventana y media normalizada como vector global. Se conserva texto íntegro; segmentación y tokenizador se versionan. La media puede perder detalles: por eso se preservan además rasgos y se evalúan ejemplos donde una restricción cambia la respuesta. Si no hay vectores disponibles, una comparación léxica determinista en Rust permite recuperación degradada y el registro sigue funcionando; no se mezclan sus scores como si fueran cosenos.

### Rasgos disponibles antes del resultado

Se extraen determinísticamente del evento y de metadatos de la integración: identificador de herramienta/operación cuando existe, tipo de archivo, restricciones declaradas estructuradamente, presencia de términos y componentes del embedding. No se usa respuesta, corrección ni resultado futuro para representar el problema evaluado.

La v1 puede aprender que un rasgo observado importa. No puede inventar un atributo semántico que su entrada no expresa: «idempotente» solo es un rasgo directo si lo aporta una fuente o contrato de herramienta; la presencia de la palabra en texto no prueba la propiedad. Las divisiones en dimensiones del embedding son numéricas y no reciben una explicación semántica inventada por una LLM.

## 3. Ciclo de operación

### A. Capturar

Una transacción inserta `instancia`, observación pendiente y `trabajo_pendiente`; solo después del commit se confirma recepción. El historial crudo conserva mensajes, entradas/salidas de herramientas, errores e interrupciones disponibles, con metadatos de fuente y sin depender de su resumen. `clave_evento` identifica el evento de origen y `grupo_evidencia` la tarea/episodio real. Un reintento de entrega conserva ambas claves; una tarea nueva conserva su identidad propia incluso si el texto se repite. Las respuestas del propio modelo, sus citas y el replay no cuentan como nueva recurrencia externa.

La cola contiene referencias al historial y es mutable. El worker reserva trabajo con token y vencimiento, calcula fuera de la transacción y confirma resultado/finalización juntos si la reserva sigue vigente. Tras una caída, se recuperan reservas vencidas; claves únicas evitan duplicar representaciones o deltas. Un fallo conserva el trabajo para reintento con espera creciente. La fuente retiene entregas no confirmadas o permite relectura desde cursor; si no ofrece ninguna de esas posibilidades, se explicita que no hay garantía de captura sin pérdidas. Consumir la cola nunca borra `instancia`.

`evento_uso` enlaza los eventos crudos a un `intento_clave`, con versión aplicada y recuperación solo cuando se conocen. Registra inicio, prueba, ajuste, resultado o interrupción y medidas con unidades/método. Puede existir sin memoria recuperada: también se observa el primer uso. Datos desconocidos quedan ausentes, no como ceros. Un reintento de entrega no es un intento de solución nuevo. Los ajustes intermedios no cuentan como ensayos independientes; `evento_feedback` conserva la evaluación externa del criterio sin duplicar esas observaciones.

Las representaciones se calculan en segundo plano. El doble EMA usa alpha rápido 0.2 y lento 0.02 dentro de cada flujo/proyecto; se inicializa con el primer vector y se anota la distancia entre medias. Un score mayor que el percentil 95 de los últimos 100 scores puede adelantar un ciclo de revisión. Antes de 100 muestras no se usa este disparador. La novedad no cambia conteos de éxito ni filtra escrituras.

### B. Recuperar

1. Delimitar el proyecto y restricciones explícitas de acceso/operación; jamás mezclar proyectos por similitud accidental.
2. Buscar episodios similares por coseno, inicialmente con barrido exacto en Rust. Para cada contexto, usar hasta 5 problemas representativos distintos derivados de sus asignaciones (el más próximo al centroide, luego selección por máxima distancia mínima; desempate por id). Esto preserva algo de variación sin un índice vectorial adicional.
3. Guardar cada contexto puntuado con su similitud. Pasan a detalle los 20 mejores con coseno >= 0.60. Los demás quedan como evaluados y no inyectados. Un contexto sin contenido no se propone como solución, pero sus instancias pueden recuperarse.
4. Aplicar las condiciones y estimaciones de utilidad de la versión actual. Ordenar por `score = max(0, coseno) * q`. Ante empate usar menor longitud, después mayor accesibilidad w y finalmente id. Así la frecuencia no suplanta al resultado.
5. Inyectar hasta 3 soluciones activas que quepan en un presupuesto inicial de 1500 tokens de memoria; guardar orden y texto exactos. Si una solución no cabe, pasar a la siguiente; no recortarla eliminando condiciones. Instancias episódicas pueden ocupar el presupuesto restante y se identifican como casos observados, no soluciones validadas. Si ninguna alcanza la elegibilidad, abstenerse de propuesta general y continuar con los casos disponibles.

Candidatos, inyectados y pertenencia son relaciones diferentes. La co-ocurrencia sirve para inspeccionar el vecindario, no para unir dominios. Un nuevo contexto sin origen conocido nace singleton; una corrección con versión objetivo explícita puede heredar su dominio. No hay fusión automática de dominios en esta v1.

La interfaz de consulta permite recuperación progresiva: referencias compactas → secuencia temporal del episodio → contenido completo solicitado. En la tanda 2, buscar o consultar detalle es inspección local; no prueba exposición. La tanda 3 registra envíos observados de paquetes íntegros; las expansiones enviadas por integraciones posteriores necesitarán evidencia equivalente. Recuperar más detalle no incrementa por sí solo la utilidad ni la recurrencia externa del contexto.

### C. Actualizar utilidad con evidencia externa

Prerrequisito de implementación: `evaluacion_ejecucion` (tanda 3) conserva resultados externos de intentos con o sin memoria. No se convierte automáticamente en `evento_feedback` de una versión ni actualiza q. Esa atribución y el aprendizaje siguiente requieren una etapa posterior, con evidencia de aplicación específica. Preparación, exposición, aplicación y éxito son hechos distintos.

Se acepta confirmación/corrección explícita con objetivo identificado o un verificador externo (por ejemplo, un test con contrato de resultado). Éxito del verificador solo significa éxito bajo ese criterio; no garantiza corrección general. No se interpreta silencio, aceptación aparente o autoevaluación del modelo como éxito.

Éxito/fallo es la primera señal de aprendizaje, no toda la evaluación. Costos, duración, correcciones y pruebas permanecen como evidencia separada en `evento_uso`. No se penaliza automáticamente una solución por ajustes que pudieran responder a nuevos requisitos. Se derivan conteos por operaciones distintas y se evitan sumas de intervalos solapados o de contadores acumulados. Esos datos posteriores no se usan como rasgos previos del mismo intento.

Para cada versión y región de aplicación:

`q = (1 + exitos) / (2 + exitos + fallos)`

Es la media de una estimación Beta-Bernoulli con prior uniforme; los conteos y el criterio evaluado se guardan en el snapshot de regla. Con cero resultados vale 0.5, indicando incertidumbre, no éxito demostrado. No se agregan resultados de versiones textuales distintas.

El modelo inicial es una sola región. Un contexto provisional puede activarse tras al menos 5 episodios independientes con éxito y q >= 0.80 bajo el criterio objetivo. Son umbrales de arranque, no una garantía estadística. Mientras no llegue allí, sus casos pueden recuperarse como antecedentes. Si se aplica una propuesta provisional de forma explícita o en un banco de pruebas, se registra la versión exacta para obtener feedback; no se realizan acciones externas experimentales automáticamente.

Una falla conocida modifica la estimación incluso si es la primera. No dispara por sí sola archivo ni cambio de pertenencia. Feedback a nivel respuesta sin versión objetivo se guarda, pero no cambia q de ningún contexto.

### D. Aprender condiciones nuevas

Cada 20 resultados nuevos atribuibles a una versión, con al menos 40 episodios de entrenamiento, el motor puede proponer un árbol de profundidad máxima 2:

- Entrena predicción de resultado para esa versión, no una clasificación de «importante/no importante».
- Considera rasgos numéricos/booleanos conocidos antes de actuar. Para numéricos prueba cortes en cuartiles del conjunto de entrenamiento; conserva al menos 10 episodios por hoja.
- Elige cortes que reduzcan error cuadrático de predicción (Brier); desempata por menor complejidad y orden de rasgo. Con rasgos o clases insuficientes mantiene una hoja.
- Congela árbol, probabilidades y manifiesto de entrenamiento en una fila `motivo='propuesta'`. Máximo una propuesta pendiente por versión.
- En las siguientes recuperaciones calcula en sombra la predicción de la propuesta y la regla vigente para la MISMA solución/version y el MISMO problema; almacena ambas antes de ver el resultado. La propuesta no cambia la selección durante esta etapa.
- Valida con dos bloques posteriores y disjuntos de 20 episodios etiquetados cada uno, con al menos 5 por hoja en cada bloque. En ambos exige reducción media de Brier >= 0.02 y que ninguna hoja empeore más de 0.02 respecto a la regla anterior. No reutiliza esos bloques para buscar otro corte.
- Si pasa, agrega una activación del snapshot. Si no pasa, agrega un snapshot `motivo='rechazo'` con métricas y mantiene la regla anterior; una nueva propuesta necesita datos futuros. Si faltan casos por hoja, espera: no toma falta de evidencia como aprobación.

Esto modifica qué condiciones importan para recuperar la solución: una región con q < 0.80 deja de recibir propuesta general, aunque el contexto sea frecuente. Las condiciones validadas se aplican a problemas nuevos sin deliberación verbal del modelo.

La validación en sombra prueba predicción sobre usos observados, no la eficacia de una alternativa que nunca se aplicó. No se asigna recompensa a candidatos descartados. El sesgo de selección limita lo que puede aprenderse; sin cobertura no se promueven regiones no observadas. Un contextual bandit con exploración registrada podría abordar esa limitación después, pero no forma parte de esta v1.

Las reglas de una hoja también se actualizan mediante snapshots cuando llegan resultados nuevos; si solo se recalculan conteos bajo la partición ya validada, no hace falta volver a validar una estructura idéntica. Durante una prueba en sombra se congelan ambos predictores comparados; después se actualizan con los resultados observados. Cada snapshot registra el corte de datos utilizado.

### E. Crear, revisar o especializar soluciones

Una semilla conserva una corrección o solución observada y su fuente. Similitud entre correcciones sugiere cercanía, no demuestra equivalencia de procedimientos. Para agrupación automática conservadora se usa igualdad de texto normalizado de la solución (o el mismo identificador externo de procedimiento) y recurrencia del problema: cada nuevo episodio debe tener coseno >= 0.80 con el representante fijo del grupo, sin encadenar vecinos transitivamente.

Al acumular 5 episodios externos independientes, el grupo puede formar un contexto provisional con texto representante observado. No se exige que provenga de una falla; también se detectan soluciones recurrentes nuevas. La promoción posterior exige resultados según C. Una solución expresada de varias formas puede quedar en grupos distintos: esta limitación es preferible a afirmar equivalencia no comprobada y se medirá en el corpus.

Tres caminos compatibles con el historial:

- **Revisión:** si la fuente identifica explícitamente la corrección como reemplazo de esa solución bajo el mismo ámbito, se agrega una versión del mismo contexto y vuelve a provisional; su utilidad se estima de nuevo. Es una señal de la interacción, no una evaluación LLM adicional. Si no se conoce esa relación, no se la infiere del mero hecho de compartir recuperación.
- **Especialización:** una condición nueva validada restringe el ámbito del contexto. Si una solución alternativa acumula recurrencia y éxitos en la región excluida, forma otro contexto del mismo dominio conocido. No hace falta duplicar nodos solo para representar una condición; el árbol ya la representa.
- **Pendiente:** sin evidencia suficiente sobre corrección, equivalencia o utilidad, conservar instancias y contexto provisional. La captura y recuperación episódica siguen operando.

Así puede profundizarse la confianza en una solución, corregirse su texto o diferenciarse su ámbito. El algoritmo no sintetiza por sí solo una explicación verbal general a partir de vectores. Aprende regularidades de selección y reutiliza texto observado; una futura verbalización no controlará la promoción.

#### Dirección posterior: patrones estructurales y asociación retrospectiva

Una solución general puede expresar una lógica de sistema: entidades, acciones, relaciones tipadas y condiciones que permanecen aunque cambien nombres o secuencias. La representación por grafos pequeños permitiría comparar y versionar partes del patrón. Una cadena temporal sola no representa autoría, permisos, dependencias ni causalidad.

La tanda 3 conserva hechos y relaciones que la fuente declare, con procedencia; no aprende esos grafos ni extrae una ontología desde prosa. Una etapa posterior puede contrastar una ejecución resuelta independientemente con patrones existentes y proponer incorporar su problema como ejemplo. Debe distinguir semejanza de equivalencia comprobada, redescubrimiento de aplicación desde memoria y éxito observado de crédito causal. Asociaciones inciertas permanecen pendientes; cambios parciales requieren validar sus dependencias y conservar versiones.

La información posterior puede ayudar a proponer la asociación, pero las señales que activan la solución en problemas futuros deben existir antes de resolverlos. La evidencia retrospectiva no modifica lo que se registró como candidato o expuesto en el pasado. Esta dirección aún no sustituye al algoritmo conservador de agrupación descrito arriba.

### F. Consolidar y perder prioridad

Disparador ordinario: 20 episodios externos nuevos, o 5 minutos sin entrada con trabajo pendiente; el EMA solo puede adelantar el ciclo. Las reglas con feedback escaso permanecen simples. No hay trabajo periódico cuando no hay cambios.

El ciclo aplica fuga exponencial a w con vida media inicial de 30 días, registra deltas y agrega +1 por episodio externo nuevo asociado. Estado de último ciclo, versión de algoritmo, intervalos y claves de cambios permiten reinicio idempotente. Un w < 0.25 excluye propuestas proactivas por desuso; no elimina contenido ni éxitos conocidos. Una consulta explícita pertinente puede recuperar ese contexto como antecedente y nuevo uso externo reactiva su accesibilidad.

No existe migración automática hacia el contexto de mayor peso. Si no se comprobó sustitución, se conserva coexistencia. La operación de liquidación queda disponible para sustituciones explícitas verificadas: destino, dominio y correspondencia de instancias deben estar determinados antes de registrar ledger, nuevas vigencias y archivo en una transacción.

## 4. Qué tendría que mostrar como Aprendizaje II

Una prueba controlada comienza con una regla global que mezcla casos. Se presentan resultados distintos según una condición observable, se aprende la partición y se evalúan episodios nuevos independientes. La comparación mantiene iguales encoder, memoria y presupuesto; cambia solo la posibilidad de aprender condiciones.

Éxitos/fallos ponderados dentro del mismo esquema son Aprendizaje I en nuestra interpretación. Cambiar condiciones de aplicación y transferir esa distinción se acerca al Aprendizaje II. Con encoder fijo y rasgos finitos, el sistema no inventa cualquier marco conceptual ni modifica arbitrariamente su forma de aprender. El resultado debe describirse como cambio de criterio acotado, no como equivalencia demostrada con aprendizaje humano.

## 5. Contratos de implementación

- `registrar_evento`: clave de origen, grupo de evidencia, rol, contenido, metadatos y tiempo; confirma escritura cruda y alta de trabajo juntas después del commit.
- `registrar_uso`: evento crudo, intento, versión conocida opcional y medidas con unidad; conserva proceso y resultado sin emitir un juicio LLM.
- `recuperar`: problema, proyecto y presupuesto; devuelve casos/versiones y registra configuración, candidatos y predicciones antes del prompt.
- `registrar_resultado`: en tanda 3, clave idempotente, ejecución (con o sin memoria), evidencia externa, criterio, alcance, resultado y objetivo conocido opcional; escribe `evaluacion_ejecucion`. La atribución posterior a una versión general es un contrato separado y nunca se infiere solo de la exposición.
- `consolidar`: aplica deltas y genera snapshots con cortes de evidencia; admite reinicio sin repetir cambios.

Una conexión escritora serializa cambios (SQLite WAL para convivencia con lectores cuando corresponda); transacciones cortas, sin esperar inferencia dentro de ellas. `rusqlite` con SQLite bundled es la propuesta de acceso. Los cálculos vectoriales/árboles se ejecutan en Rust. Un snapshot de configuración guarda todos los números de este documento, hashes de encoder/tokenizador y versión del algoritmo; no se confía en valores hardcodeados sin registro.

CTEs solo para traversals necesarios. Historial y score usan joins/ventanas y cálculos acotados; sin Kramers, servicio de grafos, entrenamiento del encoder ni LLM juez. La carpeta `codigo/` no se modifica como parte de esta actualización de diseño.

## 6. Verificación requerida

Persistencia: commit de evento/observación/trabajo atómico, caída antes/después de confirmar, reserva vencida recuperable, rechazo del worker con token obsoleto, finalización idempotente, supervivencia del historial al consumo de cola, intento de uso sin recuperación previa, medidas ausentes distintas de cero, replay idempotente, historia inmutable, frontera de vigencias, versión exacta inyectada y rechazo de feedback a candidatos descartados. Aprendizaje: registro sin feedback, no reforzar ecos del modelo, no mezclar versiones, una falla fiable afecta q, conflicto de evidencia queda excluido, nada de etiquetas futuras en rasgos, sombra sin filtración temporal y cambio de criterio evaluado en episodios nuevos. Recursos: medir recuperación semántica y costo del encoder local antes de fijarlo.

### Fuentes

- [Gazzaniga, 2000](https://people.psych.ucsb.edu/gazzaniga/PDF/cerebral%20specialization%20and%20interhemispheric%20communication.pdf): intérprete y límites de la explicación verbal.
- [Gershman et al., 2017](https://elifesciences.org/articles/23763): modelo de actualización de recuerdos frente a formación de otros.
- [BGE-M3, ficha oficial](https://huggingface.co/BAAI/bge-m3): capacidades del encoder, no validación para nuestro corpus.
- [Féraud et al., 2016](https://proceedings.mlr.press/v51/feraud16.html): antecedente de árboles con feedback de bandit; nuestra v1 no implementa ese algoritmo ni hereda sus garantías.
- [Bateson, The Logical Categories of Learning and Communication](https://ejcj.orfaleacenter.ucsb.edu/wp-content/uploads/2017/06/1972.-Gregory-Bateson-Steps-to-an-Ecology-of-Mind.pdf).

Los umbrales, el score y el árbol acotado son decisiones de ingeniería propuestas aquí. No son fórmulas extraídas de los trabajos biológicos.

- [Rosenberg: Learning by using](https://www.cambridge.org/core/books/abs/inside-the-black-box/learning-by-using/DD83066A41F317E3875AA41C28163F47).
- [Claude-Mem: almacenamiento y separación de cola/historial](https://docs.claude-mem.ai/architecture/database).
- [Claude-Mem: recuperación progresiva](https://github.com/thedotmack/claude-mem#MCP-Search-Tools).
