# Capa de memoria/contexto inspirada en predictive coding — resumen para investigación

> **Vigencia (2026-09-15):** este documento conserva antecedentes de investigación. Para implementar rigen el [ERD actualizado](erd-y-esquema-fisico.md) y el [mecanismo v1](mecanismo-aprendizaje-v1.md): Rust + SQLite, recurrencia separada de utilidad, correcciones revisables, feedback externo y condiciones aprendidas. Las propuestas anteriores de fusión por co-ocurrencia o liquidación solo por peso quedan reemplazadas.

## Motivación central
Una LLM reanaliza el input completo en cada llamada salvo que exista caching de infraestructura (KV-cache, prompt caching de Anthropic). Ese caching ya resuelve el ahorro de *cómputo* de reprocesamiento. Lo que NO resuelve es el ahorro de *contenido*: decidir qué entra al contexto en primer lugar. Un humano no re-analiza cada input desde cero porque rellena la mayor parte desde un modelo ya construido — solo procesa la desviación (predictive coding: Rao & Ballard 1997; Friston, free-energy principle). El proyecto apunta a esa segunda capa: una capa de memoria que decide qué presentar en detalle crudo vs. qué reemplazar por un puntero a un esquema ya consolidado.

**Aclaración de alcance:** no se busca que la IA "piense como humano" — se busca aislar y aplicar ese mecanismo puntual de ahorro (predicción top-down + solo el error se propaga), sin comprometerse a replicar biología más allá de lo útil.

## Arquitectura conceptual: borde / centro
Basado en la descomponibilidad casi-completa de Simon (*The Architecture of Complexity*) y en fast-slow dynamical systems / slaving principle (Haken):

- **Borde**: rápido, escribe siempre, barato (O(1) por evento). La novedad se anota después de registrar cada evento y solo prioriza procesamiento.
- **Centro**: lento, se actualiza solo bajo disparador (acumulación de N instancias, o ventana de inactividad — análogo a consolidación durante el sueño). Opera como el modelo generativo top-down.
- **Detección de novedad/cambio de régimen**: dos EMA de distinta velocidad (α_rápido >> α_lento) sobre embeddings; `|borde_t − centro_t|` como señal de novedad. Es un detector de punto de cambio (CUSUM-like), formaliza "una novedad repetida deja de serlo para el centro".

## Marco de niveles de aprendizaje (Bateson)
- **Learning I**: ajustar la respuesta dentro de un esquema de categorización fijo (esto es lo que hace el borde/centro tal como está especificado — inercia temporal, no cambio de esquema).
- **Learning II genuino**: cambiar el *criterio* de categorización mismo (re-clusterizar, mover límites de qué cuenta como "similar").
- **Decisión de diseño explícita**: no importar la fricción/resistencia al cambio de Learning III de Bateson por fidelidad biológica — esa resistencia cumple una función anti-oscilación en el cerebro (evita "personality thrashing"), pero un sistema externo a los pesos del modelo no la necesita igual. Reemplazarla por un mecanismo explícito (ej. exigir evidencia estadística sostenida antes de cambiar el criterio), no simplemente eliminar el freno.

## El vacío real identificado (razón de ser del proyecto)
Ningún proyecto existente revisado cierra el lazo de feedback sobre el *criterio* de categorización — todos (Mnemosyne/Kaliber AI, Mnemosyne npm de 28naem-del, Graphiti/Zep, Engram, OpenMemory, mnemosyne-oss, rand/mnemosyne) hacen decay, dedup, promoción/democión de memorias individuales con umbrales fijos — es Learning I con más capas, no Learning II autoevaluado.

**Riesgo de diseñar el lazo de feedback a priori:** regreso infinito (¿quién evalúa al evaluador?).

**Resolución adoptada — learning-by-using (Rosenberg, *Inside the Black Box*, 1982):** no diseñar el mecanismo de autoevaluación desde el día uno. Dejar que el cuello de botella se revele con uso real sobre corpus propio (investigación económica), y recién ahí diseñar la corrección puntual para el fallo ya identificado empíricamente.

**Condición necesaria para que esto funcione (matiz sobre Rosenberg):** el fallo tiene que ser *atribuible*, no solo perceptible — instrumentar el sistema para loguear qué nodo de `centro` generó cada respuesta, de forma que un fallo notado pueda rastrearse a la categorización específica que lo causó. Sin esto, el fallo es difuso (se le atribuye al modelo, no al criterio de clustering) y el aprendizaje-por-uso no tiene de dónde agarrarse.

## Decisiones técnicas ya tomadas
- **Una sola base de datos de grafo**, no SQL + NoSQL separados. Candidatos: Kùzu (embebido, tipo Cypher, columnar) o SQLite con lista de adyacencia + CTEs recursivas. Local-first.
- **Un solo esquema**, dos tipos de nodo (`instancia` rápida, `contexto` lenta) con dos tipos de arista (`observado_en`, `pertenece_a` — esta última versionada con nueva vigencia en cada reasignación, conservando las versiones anteriores).
- **Ahorro de cómputo (KV-cache / prompt caching) y ahorro de contenido (esta capa) son ejes distintos** — no confundir ambos al medir el éxito del proyecto.

## Prior art a revisar en la sesión de research
- **Mnemosyne (Jonelagadda et al., Kaliber AI, arXiv 2510.08601)** — el más cercano conceptualmente (core summary destilado de subconjunto del grafo ≈ centro; filtros de redundancia ≈ borde). Sin repo público conocido.
- **Graphiti/Zep** — estándar de facto, grafo bi-temporal, referencia obligada de ingeniería madura.
- **Engram** — patrón de gate barato + escalamiento caro solo si es ambiguo.
- **mnemosyne-oss/mnemosyne** — SQLite-backed, local-first, notablemente honesto (retiró una cifra de benchmark no reproducible). Buena base si se decide reusar infraestructura en vez de partir de cero.
- **rand/mnemosyne** — Rust/LibSQL, Claude Code nativo, tiene "hierarchical weight adaptation" — revisar en detalle si esto toca el criterio de categorización o es solo re-ponderación de scores (no confirmado).
- **OpenMemory** — segmentación multi-sector (episódica/semántica/procedural), decay/reinforcement.
- ⚠️ Ojo con el otro "Mnemosyne" (28naem-del/npm mnemosy-ai) — nombre idéntico, proyecto distinto, con señales fuertes de marketing exagerado (tabla comparativa de autoría propia, superlativos sin respaldo independiente). Útil solo para mirar piezas puntuales (decay, dedup), no como referencia de credibilidad.

## Qué buscar/validar en la sesión de Cowork
1. Leer el paper de Kaliber AI completo (no solo abstract) — confirmar si su "core summary" tiene algún mecanismo de actualización de criterio o es prompt/heurística fija.
2. Revisar código de `rand/mnemosyne` — el "hierarchical weight adaptation" es el candidato más prometedor encontrado hasta ahora para algo parecido a Learning II.
3. Decidir Kùzu vs. SQLite+CTEs recursivas con una prueba chica de rendimiento en el propio corpus económico.
4. Diseñar el esquema de logging de atribución (qué nodo de `centro` generó cada respuesta) ANTES de empezar a usar el sistema — es prerrequisito para que learning-by-using funcione.

## Decisiones de persistencia vigentes

- Registro completo en `instancia`/`observado_en`; EMA como anotación, nunca como filtro.
- Todos los candidatos evaluados alimentan la co-ocurrencia de dominios; `fue_inyectado` identifica el subconjunto expuesto al modelo para investigar fallos.
- Asignaciones históricas por vigencia (SCD tipo 2) y ledger append-only para cambios de peso, incluida la liquidación.
- El modelo actualizado está en [ERD y esquema físico](erd-y-esquema-fisico.md).
