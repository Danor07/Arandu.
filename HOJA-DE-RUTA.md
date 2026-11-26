# Hoja de ruta de Arandu

Estado actual de lo que falta para llegar a Arandu: Memory of Dynamic Solutions (antes MSP, memoria de soluciones persistentes; el binario y las órdenes siguen llamándose `msp`). Se tilda al terminar cada entrega (PR fusionado y CI en verde) y se reescribe si cambia el plan.

**Tamaños:** chico, unas horas; mediano, uno o dos días; grande, de tres a cinco días. Son estimaciones del 05/11/2026, no mediciones.

**Regla de cada entrega:** sale con sus pruebas (las de siempre, de propiedad y de mutación) y no edita migraciones publicadas, porque las bases verifican su hash.

**Dónde corre cada prueba (desde el 25/11/2026):** la CI de GitHub corre en cada PR formato, clippy, pruebas (regresión y propiedad) y la mutación de las líneas que cambia el PR, en cuatro fragmentos, todo en Linux (`calidad.yml`). El umbral es el mismo de siempre: 90 % sin timeouts, juzgado por `codigo/scripts/umbral_mutacion.py`. Esa mutación acotada consume minutos de Actions en cada PR; en un PR grande, bastante. La mutación completa y el fuzzing siguen en `profundo.yml`, a mano desde la pestaña Actions, porque en un repo privado la corrida entera consume cientos de minutos.

## Hecho

Las entregas 6 a 12 se fusionaron en `main` con el PR #11 (25/11/2026) y siguen sin tildar porque **la CI de ese PR no quedó en verde**: sus siete trabajos terminaron en falla a los dos o tres segundos y sin registro, es decir, no llegaron a arrancar. Hay que mirar por qué en la pestaña Actions (en un repo privado suele ser el límite de minutos o de gasto) y volver a correrla. Cada una tiene su sección en `codigo/README.md` con lo que hace y sus límites.

- [x] **Renombre a MSP:** crate, binario, plugin, órdenes `msp::`, etiquetas y variables. La carpeta de datos conserva el nombre anterior, `memoria-persistente` (commit 9883c3f).
- [ ] **Renombre a Arandu:** hecho el nombre visible (README, licencia, NOTICE, descripciones). Falta el técnico: crate, binario, órdenes `msp::`, variables `MSP_*` y sobre; rompe hooks instalados y obliga a renombrar la variante `arandu-*` actual, así que va en un PR aparte.
- [ ] **Pruebas endurecidas en CI:** propiedades, fuzzing y mutación con umbral de 90 %, en Linux y Windows ([PR #1](https://github.com/Danor07/msp/pull/1)).

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
- [ ] **6. Competencia con la horquilla, archivado y alerta temprana** (grande). *Implementada, sin fusionar.* Producción usa una regla simple (ventaja de enteros en una ventana). La simulación comparó el umbral de la horquilla como regla de corte, con una ganancia elegida a mano y sobre escenarios sintéticos sin rendimientos crecientes: no mostró que decida mejor, y tampoco pone a prueba la horquilla como descripción del fenómeno.
  - El dominio de comparabilidad es el tipo de objetivo. Compiten los patrones de secuencia, y `h` son los éxitos verificados.
  - La horquilla imperfecta ẋ = h + r·x − x³ se simula contra una regla simple («la nueva gana después de N éxitos sin fallas»). Si la dinámica no decide mejor, se queda la regla simple.
  - Antes de archivar una solución vieja se migran sus vínculos. Archivar, nunca borrar.
  - Alerta temprana cuando una solución empieza a fallar de forma intermitente (sube la varianza y la autocorrelación).
- [ ] **7. SZZ: fallos tardíos** (mediano). *Implementada, sin fusionar.* Un arreglo posterior apunta, con `git blame`, al commit que introdujo el error y al episodio que lo produjo, y le resta evidencia. El vínculo es «probable», no automático.

## Más allá de Claude Code

- [ ] **8. Adaptador para Codex** (mediano). *Implementado, sin fusionar y sin prueba contra el host real.*
- [ ] **9. Adaptador para OpenCode** (mediano). *Implementado, sin fusionar y sin prueba contra el host real.*
- [ ] **10. Adaptador para modelos locales** (mediano). *Implementado, sin fusionar y sin prueba contra Ollama.*

Los tres van sobre el contrato JSONL v1, que ya es independiente de la plataforma.

## La capa de colaboración

- [ ] **11. Slack de agentes** (mediano). *Implementado, sin fusionar.*
  - Un canal general y canales por proyecto, donde la pregunta y la respuesta quedan registradas para todos.
  - Audiencias: todos, los activos o uno. «Activo» se define con una señal de vida periódica.
  - Un mensaje es información, no una orden, y hay tope de mensajes.
- [ ] **12. La capa de proyectos** (grande). *Implementada, sin fusionar.*
  - Un proyecto abarca varias carpetas; la principal es un símbolo, no una atadura.
  - Chats con un rol que se reutilizan (por ejemplo, edición de imágenes, animación, guion, sonido y revisor), cada uno con un procedimiento escrito: qué recibe, qué entrega, cuándo está terminado y a quién se lo pasa.
  - Reglas del proyecto independientes de la plataforma.

## Después de la 12: la urna

Inspirado en el marco teórico, sin intención de reproducirlo: un camino es un atractor que se hunde con las trayectorias que atrae; especialización y costo van juntos; lo normal es que varios caminos convivan.

- [ ] **13. La urna observada** (chico). *Implementada en la rama, sin fusionar.* Lectura en `vigente-consultar`: éxitos independientes contra inducidos por la oferta, cuota y profundidad por camino, pasos no compartidos con la vigente, seguimiento de lo ofrecido y vigente en desuso. No decide nada.
- [ ] **14. Simulaciones de la urna** (chico). *Implementadas en la rama, sin fusionar.* Dónde bifurca la urna pura, el encierro por rendimientos crecientes y sucesión contra coexistencia según el solapamiento de pasos.
- [ ] **15. Medir p(x) con uso real** (depende de la entrega 5). Con `urna.seguimiento` a lo largo del tiempo: cuánto se sigue lo ofrecido y cómo cambia con la cuota. Decide si hay rendimientos crecientes.
- [ ] **16. Que la profundidad independiente gobierne la competencia** (mediano; decidir con 15). Hoy un éxito inducido por la oferta cuenta igual que uno independiente.
- [ ] **17. Vigente condicional en vez de una por tipo** (grande; decidir con 15). Coexistencia: qué camino vale para qué contexto (rol, carpeta, plataforma, pasos de los que ya depende), y archivar sólo dentro del mismo contexto. Hay que contrastar antes la medida de costo de desacople con relevos reales.
- [ ] **18. Fuente de caminos nuevos** (mediano). Hoy nada genera novedad; la simulación muestra que sin eso un camino mejor no entra.
- [ ] **19. El criterio de «lo mismo»** (grande). Las reglas `objetivo-terminos-v1` y `herramienta-destino-v1` deciden qué objetivos y qué soluciones son comparables y nadie las evalúa: es el Aprendizaje II del proyecto.

## Pendientes de decidir

- Si los 167 MB de datos que ya hay en `memoria-persistente` se convierten a episodios o quedan como registro histórico.
- La metodología propia de trabajo con agentes, que tiene que salir del uso de la entrega 12.
- Medir con uso real (entrega 5) la función p(x) de la urna: cuántas veces el host sigue la vigente que se le ofrece y cómo cambia con la cuota de usos de ese camino. De ahí sale si hay rendimientos crecientes y, si los hay, el `r` de la horquilla, que es la forma local de la urna en la bifurcación y no una alternativa a ella. Con eso se repite la comparación de la entrega 6; hoy la ventaja `K` no está calibrada y el umbral de la horquilla, con ganancia elegida a mano, ganó en cuatro de doce mundos sintéticos.
- Probar los adaptadores de Codex, OpenCode y modelos locales contra los hosts reales, y decidir si las órdenes `msp::` se abren en alguno.
- Cómo se autentica quién invoca la CLI: hoy cualquier proceso local puede pasar el `--sesion` de otro chat, y de eso dependen los topes de los canales y quién escribe reglas y roles.
