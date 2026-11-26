# Tres mecanismos de disparo/consolidación — qué pregunta responde cada uno

> **Vigencia (2026-09-15):** este documento conserva antecedentes de investigación. Para implementar rigen el [ERD actualizado](erd-y-esquema-fisico.md) y el [mecanismo v1](mecanismo-aprendizaje-v1.md): Rust + SQLite, recurrencia separada de utilidad, correcciones revisables, feedback externo y condiciones aprendidas. Las propuestas anteriores de fusión por co-ocurrencia o liquidación solo por peso quedan reemplazadas.

**Alcance del documento (para no perderlo de vista):** el objetivo es un plugin de memoria persistente para un LLM, no una reconstrucción de tu marco económico en código. Tu marco es una fuente de mecanismos candidatos ya pensados y en algunos casos formalizados — no el target de la implementación. Todo lo que sigue se evalúa por una sola vara: si sirve para decidir qué entra al contexto de una LLM y cuándo se consolida, no por si es fiel a tu marco. Donde un mecanismo de tu marco no encaje, se descarta sin culpa — la idea era "tal vez ayude", no "tiene que encajar".

Antes de escribirlos: no creo que sean tres *opciones* entre las que elegir una. Cada uno responde una pregunta distinta, y la distinción que marcaste vos mismo (deliberado/logueado vs. intuitivo-repetitivo-no-racionalizado) es justo la que separa cuál sirve para qué. Los presento como tres piezas con roles distintos, no como tres candidatos rivales para el mismo lugar del sistema — al final explico por qué, si armás el pipeline mal, terminás con un sistema que solo ve lo que vos ya notás, que es exactamente el problema que señalaste.

## 1. Borde/centro con doble EMA (el tuyo)

**Mecanismo:** dos medias móviles exponenciales de distinta velocidad sobre embeddings (α_rápido ≫ α_lento); la distancia `|borde_t − centro_t|` es la señal de novedad, tipo CUSUM. Cada evento se registra siempre en `instancia`/`observado_en`. La novedad se anota sobre la instancia ya escrita; superar un umbral puede priorizar una revisión de centro, nunca condicionar el registro.

**Pregunta que responde:** "¿este evento se desvía lo suficiente de lo que el sistema ya tiene consolidado como para priorizar su procesamiento posterior al registro?"

**Dónde usarlo:** como anotación de novedad sobre interacción con contenido semántico claro y ya diferenciable — una solución nueva, una decisión de arquitectura, un error corregido. Es bueno para lo que vos mismo ya reconocés como evento en el momento en que pasa.

**El límite estructural, y es el que vos señalaste:** un detector de novedad, por construcción, deja de disparar exactamente cuando el patrón se vuelve automático. Cuanto más consolidado y repetido es un comportamiento (Aprendizaje II en el sentido de Bateson: el marco que opera *sin* deliberación), *menos* señal de sorpresa produce frente a un detector de desviación — porque ya no se desvía de nada, es la línea de base. Este mecanismo no es neutral respecto al problema que planteaste: sistemáticamente va a dejar de ver lo intuitivo-repetitivo a medida que se vuelve más arraigado, que es lo opuesto de "de suma importancia y no lo anoto". No es un defecto de implementación, es lo que hace un detector de cambio por definición: mide cambio, no mide presencia estable.

## 2. Urna de Pólya generalizada con novedad (Tria–Loreto–Servedio–Strogatz 2014, sobre la urna no lineal de Arthur)

**Mecanismo:** cada instancia nueva "elige" un color (categoría) existente con probabilidad proporcional a su peso acumulado (reforzamiento — más se repite un patrón, más pesa), o dispara la aparición de un color nuevo desde el "adyacente-posible" con una tasa que depende de lo ya explorado (esquema tipo Hoppe). La versión de Arthur da la dinámica determinista asociada: $\dot{x} = p(x) - x$, con $p$ en forma de S; los puntos fijos estables (cerca de 0 y 1) son la cristalización de una categoría, el punto fijo inestable del medio es la frontera entre cuencas.

**Pregunta que responde:** no "¿es raro esto?" sino "¿cuánto se repitió esto, acumuló peso suficiente, y cruzó el umbral para volverse una categoría estable?" — cuenta recurrencia y refuerzo, no desviación de una línea de base.

**Por qué este es el que de verdad ataca tu punto:** no necesita que un evento individual *parezca* importante para contarlo. Cada repetición de un patrón — aunque sea aburrida, aunque su score de novedad sea bajo — suma peso al color correspondiente. Cuando ese peso cruza el umbral de $p(x)$, el color se cristaliza en nodo de centro sin que ningún evento puntual haya sido marcado como relevante por nadie, ni por vos ni por un detector de sorpresa. Es justo el mecanismo que puede capturar algo que hacés siempre y nunca racionalizás: la reincidencia misma es la señal, no el contenido de un evento particular.

**La condición que esto impone, y que cambia tu arquitectura:** para que esto funcione, el nivel de borde tiene que loguear *todo*, sin filtrar por "parece importante" — si el filtro de entrada descarta lo rutinario antes de que llegue a la urna, nunca se acumula el conteo que hace falta para cruzar el umbral. Esto invierte el orden que tenías pensado: no es "el gate de novedad decide qué se escribe, y lo escrito eventualmente se consolida" — es "se escribe barato y sin criterio todo lo que pasa, y el conteo/urna decide después qué se vuelve categoría". El costo es que el borde deja de ser barato en el sentido de "selectivo"; sigue siendo barato en cómputo por evento, pero ya no filtra semánticamente.

**Advertencia honesta:** el modelo de Tria-Loreto está validado sobre secuencias de símbolos discretos (tags, ediciones de Wikipedia, palabras, notas musicales) donde "es el mismo color" ya está resuelto de antemano por la naturaleza discreta del dato. Aplicado a embeddings de interacción cruda, el modelo no te saca el problema de clustering/similitud — lo asume resuelto. Te dice *cuándo* algo que ya sabés agrupar cruza a consolidación; no te dice *cómo* agrupar. Ese "cómo" sigue siendo tuyo.

**Extensión relevante para tu caso — Iacopini, Milojević & Latora (2018):** es la versión en red de la misma familia de urnas reforzadas — en vez de una lista plana de colores, el reforzamiento ocurre sobre nodos y aristas de un grafo. Como vas a usar una base de datos de grafo (Kùzu o SQLite+CTEs) para `instancia`/`contexto`, este objeto formal calza mejor con tu esquema que la urna plana original: te da directamente cómo pensar el peso de una arista `observado_en` creciendo por repetición, no solo el peso de un nodo aislado.

## 3. Critical slowing down (Scheffer et al. 2009, lo que ya tenías en tu nota de Alerta temprana)

**Mecanismo:** cerca de una bifurcación tipo fold, el autovalor dominante del sistema tiende a cero — el tiempo de retorno al equilibrio crece. Se manifiesta como varianza creciente (señal más robusta, según tu propia nota) y, más débilmente y con evidencia mixta, autocorrelación lag-1 creciente, en la serie de fluctuaciones antes del colapso/transición.

**Pregunta que responde:** ni "¿es nuevo?" ni "¿se repite?", sino "¿un nodo de centro que ya está consolidado está perdiendo estabilidad y por reorganizarse?" — es un indicador de fragilidad de algo que ya existe, no de formación de algo nuevo.

**Dónde usarlo:** como chequeo de salud periódico sobre nodos de centro ya cristalizados — medir varianza (y con cautela, dado lo que vos mismo documentaste sobre 2008, no confiar solo en autocorrelación) en la serie de instancias que siguen llegando a ese nodo. Un aumento sostenido es la señal de que ese esquema puede estar por romperse — el análogo a Aprendizaje III, la reorganización del marco mismo, no un ajuste dentro de él.

**Su límite es el mismo que el del mecanismo 1, en la otra punta:** mide inestabilidad de algo que *ya* es un atractor. No sirve para la formación inicial de un patrón tácito — para eso hace falta primero que exista el nodo, cosa que hace el mecanismo 2, no este.

**Corrección importante, a partir de tu propio ciclo inter-dominio (núcleo-periferia):** este mecanismo solo detecta *un* modo de muerte de un nodo consolidado — el fold clásico, donde el atractor pierde profundidad gradualmente y hay señal de inestabilidad medible antes del colapso. Tu ciclo describe un segundo modo, la **desintermediación**: una categoría periférica nueva no ataca ni desestabiliza al nodo viejo, simplemente vuelve innecesario el rol que cumplía — "no lo empuja, lo deja colgando". En ese modo no hay varianza ni autocorrelación creciente, porque el nodo viejo no se desestabiliza: sigue tan estable como siempre, solo que deja de recibir tráfico porque una categoría nueva absorbió su función. Critical slowing down es ciego a esto por diseño — mide inestabilidad, y acá no hay ninguna. Si vas a monitorear salud de nodos de centro, hacen falta dos señales, no una: varianza/autocorrelación creciente (fold) y, en paralelo, una caída sostenida en la tasa de instancias entrantes a un nodo relativa a sus vecinos (desintermediación) — que es una métrica mucho más simple, un conteo, no un indicador dinámico.

## Un cuarto ingrediente, no un mecanismo de disparo: cuándo NO tocar un nodo

Tu ciclo inter-dominio identifica que la barrera real para reorganizar algo no es la *magnitud* del costo sino su *indeterminación*: el núcleo no puede computar cuántos protocolos se desacoplan si suelta un requisito viejo, porque están todos enlazados entre sí. Un costo alto pero conocido se paga; uno indeterminado paraliza porque no se puede evaluar qué se rompe río abajo.

Esto no es un disparador de consolidación — es un criterio de riesgo antes de re-clusterizar. Un nodo de centro con alta densidad de dependencias entrantes (muchos otros nodos de instancia o de centro que lo referencian) es peligroso de reorganizar no porque sea costoso sino porque el efecto cascada es incomputable de antemano. Práctico: usar la densidad de dependencias de un nodo como proxy de riesgo antes de tocarlo. Nodos de baja densidad se re-clusterizan libremente. Nodos de alta densidad se versionan o bifurcan en vez de mutarse in-place, porque no se puede predecir con confianza qué otras cosas dependen de esa categorización exacta.

## Una restricción metodológica que se transfiere directo (no un mecanismo, una disciplina)

Tu decisión de que "el agente es fino" — no reconstruir la trayectoria interna completa de un agente porque ni el propio agente tiene acceso a ella (Nisbett-Wilson), y medir en cambio a escala observable (firma, no cognición individual) — aplica sin cambios al logging de atribución que pediste como prerrequisito. No se puede loguear "qué pasó realmente adentro" de la LLM al categorizar algo — ese estado no es introspectable ni siquiera en principio, ni para la LLM ni para vos mirando afuera. Lo que sí se puede loguear es comportamiento observable: qué instancia se activó, con qué peso, qué nodo de centro la reclamó, qué salida produjo el sistema como consecuencia. El logging de atribución tiene que construirse sobre esa capa observable, no sobre una reconstrucción de "por qué" categorizó así — esa pregunta no tiene respuesta accesible, ni en tu marco ni acá.

Relacionado, una advertencia que ya te aplicás a vos mismo y vale la pena señalar explícitamente para este proyecto: "homónimo real vs. espejismo" — descartaste Kuramoto para las ramas de Perez porque el vocabulario calzaba (sincronización) pero el fenómeno no (logísticas que saturan una vez, no osciladores). La misma pregunta corre para Bateson, Friston o incluso la urna de Tria-Loreto acá: antes de asumir que un mecanismo económico transfiere a una LLM, preguntar si el fenómeno realmente se comporta igual o si solo suena parecido. La urna pasa esa prueba razonablemente bien porque tiene validación empírica en dominios ajenos a la economía (Wikipedia, tags, texto). Critical slowing down, después de la corrección de arriba, la pasa peor — es candidato con un hueco conocido, no una herramienta resuelta.

## Cómo encajan si los combinás

Los tres operan en momentos distintos del ciclo de vida de un patrón, no compiten por el mismo lugar:

- **Formación de lo tácito-repetitivo** (tu punto: lo que hacés sin notar) → mecanismo 2, alimentado por un borde que loguea sin filtrar semánticamente.
- **Priorización de lo novedoso** (lo que ya reconocés como evento al pasar) → mecanismo 1, como anotación sobre contenido ya registrado con semántica clara, en paralelo al conteo del mecanismo 2, no en su lugar.
- **Monitoreo de lo ya consolidado** (¿este esquema está por romperse?) → mecanismo 3, corriendo periódicamente sobre nodos de centro existentes, sin importar si se formaron por 1 o por 2.

Si usás solo el mecanismo 1 como filtro de entrada único, terminás con un sistema que solo ve lo que vos ya notás como importante — el mismo sesgo humano de introspección limitada (Nisbett-Wilson) que le atribuiste a tu propia impronta, pero replicado en la arquitectura del sistema que se supone que iba a compensar eso.
