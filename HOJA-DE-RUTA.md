# Hoja de ruta de Arandu

Estado actual de lo que falta para llegar a Arandu: Memory of Dynamic Solutions (antes MSP, memoria de soluciones persistentes; el binario y las órdenes siguen llamándose `msp`). Se tilda al terminar cada entrega (PR fusionado y CI en verde) y se reescribe si cambia el plan.

**Tamaños:** chico, unas horas; mediano, uno o dos días; grande, de tres a cinco días. Son estimaciones del 05/11/2026, no mediciones.

**Regla de cada entrega:** sale con sus pruebas (las de siempre, de propiedad y de mutación) y no edita migraciones publicadas, porque las bases verifican su hash.

**Dónde corre cada prueba (desde el 25/11/2026):** la CI de GitHub corre en cada PR formato, clippy, pruebas (regresión y propiedad) y la mutación de las líneas que cambia el PR, en cuatro fragmentos, todo en Linux (`calidad.yml`). El umbral es el mismo de siempre: 90 % sin timeouts, juzgado por `codigo/scripts/umbral_mutacion.py`. La mutación completa y el fuzzing siguen en `profundo.yml`, a mano desde la pestaña Actions.

**Dónde se desarrolla (desde el 26/11/2026):** en este repositorio, que es público. Nació con un único commit inicial; el historial anterior quedó en un repositorio privado de archivo, y los PR y commits que esta hoja nombra de antes de esa fecha (del #1 al #12, y hashes como 9883c3f) son de ahí.

## Hecho

Las entregas 6 a 14 están fusionadas (PR #11 y #12 del repositorio de archivo). En el #12 pasaron las pruebas en Ubuntu y el control de mutación detectó los 55 mutantes evaluables; ese umbral se juzgó a mano con los reportes de Actions, porque GitHub no inició los trabajos finales por una restricción de facturación de aquel repositorio privado. En este, público, la CI todavía tiene que correr completa por primera vez. Cada una tiene su sección en `codigo/README.md` con lo que hace y sus límites.

- [x] **Renombre a MSP:** crate, binario, plugin, órdenes `msp::`, etiquetas y variables. La carpeta de datos conserva el nombre anterior, `memoria-persistente` (commit 9883c3f).
- [ ] **Renombre a Arandu:** hecho el nombre visible (README, licencia, NOTICE, descripciones). Falta el técnico: crate, binario, órdenes `msp::`, variables `MSP_*` y sobre; rompe hooks instalados y obliga a renombrar la variante `arandu-*` actual, así que va en un PR aparte.
- [ ] **Pruebas endurecidas en CI:** propiedades, fuzzing y mutación con umbral de 90 %, en Linux y Windows (PR #1 del repositorio de archivo).

## La unidad: el episodio

- [ ] **1. Normalizar la secuencia y crear el episodio** (mediano). El episodio es objetivo, secuencia y resultado.
  - Objetivo con comprobación, sobre las tareas que ya existen.
  - Secuencia normalizada: herramienta y tipo de destino, no los argumentos literales. El registro crudo queda aparte.
  - Resultado: pasó o falló, más el costo en tiempo, reintentos y tokens.
  - Tablas nuevas `episodio`, `episodio_paso` y `episodio_resultado`, en una migración nueva.
- [ ] **2. Cortes automáticos del episodio** (mediano). Cada paso de la lista de tareas es un objetivo, las llamadas hasta el paso siguiente son la secuencia, y el cierre con su verificación es el resultado. Depende de lo que exponga cada plataforma.
- [ ] **3. Secuencia mínima y caminos descartados** (mediano). Se guarda la secuencia mínima que llegó al resultado. Los desvíos quedan como «intentado y descartado», que también evita repetirlos.
- [ ] **4. Recuperar la solución vigente por tipo de objetivo** (mediano). Hoy la búsqueda es textual (FTS5 sobre el registro crudo). La pregunta pasa a ser «cuál es la forma vigente de hacer esto». La búsqueda textual queda como respaldo.

## El corazón: la limpieza

- [ ] **5. Juntar uso real** (chico, pero lleva tiempo). Instalar el plugin y usarlo en proyectos reales en modo `solo_registro`, para que la entrega 6 tenga datos de verdad y no sólo ejemplos.
- [x] **6. Competencia con la horquilla, archivado y alerta temprana** (grande). Producción usa una regla simple (ventaja de enteros en una ventana). La simulación comparó el umbral de la horquilla como regla de corte, con una ganancia elegida a mano y sobre escenarios sintéticos sin rendimientos crecientes: no mostró que decida mejor, y tampoco pone a prueba la horquilla como descripción del fenómeno.
  - El dominio de comparabilidad es el tipo de objetivo. Compiten los patrones de secuencia, y `h` son los éxitos verificados.
  - La horquilla imperfecta ẋ = h + r·x − x³ se simula contra una regla simple («la nueva gana después de N éxitos sin fallas»). Si la dinámica no decide mejor, se queda la regla simple.
  - Antes de archivar una solución vieja se migran sus vínculos. Archivar, nunca borrar.
  - Alerta temprana cuando una solución empieza a fallar de forma intermitente (sube la varianza y la autocorrelación).
- [x] **7. SZZ: fallos tardíos** (mediano). Un arreglo posterior apunta, con `git blame`, al commit que introdujo el error y al episodio que lo produjo, y le resta evidencia. El vínculo es «probable», no automático.

## Más allá de Claude Code

- [x] **8. Adaptador para Codex** (mediano). *Sin prueba contra el host real.*
- [x] **9. Adaptador para OpenCode** (mediano). *Sin prueba contra el host real.*
- [x] **10. Adaptador para modelos locales** (mediano). *Sin prueba contra Ollama.*

Los tres van sobre el contrato JSONL v1, que ya es independiente de la plataforma.

## La capa de colaboración

- [x] **11. Slack de agentes** (mediano).
  - Un canal general y canales por proyecto, donde la pregunta y la respuesta quedan registradas para todos.
  - Audiencias: todos, los activos o uno. «Activo» se define con una señal de vida periódica.
  - Un mensaje es información, no una orden, y hay tope de mensajes.
- [x] **12. La capa de proyectos** (grande).
  - Un proyecto abarca varias carpetas; la principal es un símbolo, no una atadura.
  - Chats con un rol que se reutilizan (por ejemplo, edición de imágenes, animación, guion, sonido y revisor), cada uno con un procedimiento escrito: qué recibe, qué entrega, cuándo está terminado y a quién se lo pasa.
  - Reglas del proyecto independientes de la plataforma.

## Después de la 12: la urna

Inspirado en el marco teórico, sin intención de reproducirlo: un camino es un atractor que se hunde con las trayectorias que atrae; especialización y costo van juntos; lo normal es que varios caminos convivan.

- [x] **13. La urna observada** (chico). Lectura en `vigente-consultar`: éxitos independientes contra inducidos por la oferta, cuota y profundidad por camino, pasos no compartidos con la vigente, seguimiento de lo ofrecido y vigente en desuso. No decide nada.
- [x] **14. Simulaciones de la urna** (chico). Dónde bifurca la urna pura, el encierro por rendimientos crecientes y sucesión contra coexistencia según el solapamiento de pasos.
- [ ] **15. Medir p(x) con uso real** (depende de la entrega 5). Con `urna.seguimiento` a lo largo del tiempo: cuánto se sigue lo ofrecido y cómo cambia con la cuota. Decide si hay rendimientos crecientes.
- [ ] **16. Que la profundidad independiente gobierne la competencia** (mediano; decidir con 15). Hoy un éxito inducido por la oferta cuenta igual que uno independiente.
- [ ] **17. Vigente condicional en vez de una por tipo** (grande; decidir con 15). Coexistencia: qué camino vale para qué contexto (rol, carpeta, plataforma, pasos de los que ya depende), y archivar sólo dentro del mismo contexto. Hay que contrastar antes la medida de costo de desacople con relevos reales.
- [ ] **18. Fuente de caminos nuevos** (mediano). Hoy nada genera novedad; la simulación muestra que sin eso un camino mejor no entra.
- [ ] **19. El criterio de «lo mismo»** (grande). Las reglas `objetivo-terminos-v1` y `herramienta-destino-v1` deciden qué objetivos y qué soluciones son comparables y nadie las evalúa: es el Aprendizaje II del proyecto.

## La interfaz

- [ ] **20. Órdenes cortas y renombre técnico** (mediano). Las órdenes de hoy (`msp --control vigente-consultar --memoria ID --proyecto P --objetivo "…"`) son largas y repiten lo que el programa ya puede saber. El modelo es git: un verbo corto, y la memoria y el proyecto se deducen de la carpeta actual. Va junto con el renombre del binario a `arandu`, porque rompen lo mismo (hooks instalados y guiones) y conviene romperlo una sola vez. Las órdenes largas quedan como alias durante una versión.

## Pendientes de decidir

- Si los 167 MB de datos que ya hay en `memoria-persistente` se convierten a episodios o quedan como registro histórico.
- La metodología propia de trabajo con agentes, que tiene que salir del uso de la entrega 12.
- Medir con uso real (entrega 5) la función p(x) de la urna: cuántas veces el host sigue la vigente que se le ofrece y cómo cambia con la cuota de usos de ese camino. De ahí sale si hay rendimientos crecientes y, si los hay, el `r` de la horquilla, que es la forma local de la urna en la bifurcación y no una alternativa a ella. Con eso se repite la comparación de la entrega 6; hoy la ventaja `K` no está calibrada y el umbral de la horquilla, con ganancia elegida a mano, ganó en cuatro de doce mundos sintéticos.
- Probar los adaptadores de Codex, OpenCode y modelos locales contra los hosts reales, y decidir si las órdenes `msp::` se abren en alguno.
- Cómo se autentica quién invoca la CLI: hoy cualquier proceso local puede pasar el `--sesion` de otro chat, y de eso dependen los topes de los canales y quién escribe reglas y roles.
