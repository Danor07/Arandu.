-- Arandu (opt-in): variante experimental de secuencias observadas, publicada como
-- migracion 011 para coexistir con las tablas de las migraciones 009 y 010 de main.
-- Autor: Arandu. Sus tablas usan el prefijo arandu_ y no migra datos derivados de
-- la rama experimental.
-- Representación derivada del recorrido observado de un episodio sellado.
-- No afirma un mínimo causal: por omisión conserva todos los pasos y sólo
-- aparta los que una referencia JSONL tipada del mismo episodio descarta.
CREATE TABLE arandu_secuencia_minima (
    id INTEGER PRIMARY KEY,
    clave TEXT NOT NULL UNIQUE CHECK (length(trim(clave)) > 0),
    origen TEXT NOT NULL CHECK (origen IN ('manual_tarea', 'automatico_lista')),
    episodio_id INTEGER NOT NULL CHECK (episodio_id > 0),
    proyecto TEXT NOT NULL CHECK (length(trim(proyecto)) > 0),
    tipo_objetivo_json TEXT NOT NULL CHECK (
        json_valid(tipo_objetivo_json)
        AND json_type(tipo_objetivo_json) = 'array'
        AND json_array_length(tipo_objetivo_json) >= 2),
    tipo_objetivo_sha256 TEXT NOT NULL CHECK (
        length(tipo_objetivo_sha256) = 64
        AND tipo_objetivo_sha256 NOT GLOB '*[^0-9a-f]*'),
    -- Un criterio de tarea y una comprobación técnica no son equivalentes.
    verificacion TEXT NOT NULL CHECK (verificacion IN ('criterio_tarea', 'comprobacion_tecnica')),
    algoritmo TEXT NOT NULL CHECK (algoritmo = 'camino-observado-descartes-v1'),
    normalizador_version TEXT NOT NULL CHECK (length(trim(normalizador_version)) > 0),
    -- Fotografía del resultado al construir. La corrección posterior no la reescribe.
    resultado INTEGER CHECK (resultado IN (0, 1)),
    -- `candidata_*` describe el camino derivado, no una reejecución: el
    -- resultado acredita el episodio fuente, no la variante con descartes.
    estado TEXT NOT NULL CHECK (estado IN (
        'candidata_sin_descartes', 'candidata_con_descartes', 'no_candidata')),
    motivo TEXT NOT NULL CHECK (length(trim(motivo)) > 0),
    evidencia_resultado_instancia_id INTEGER REFERENCES captura_evento(instancia_id),
    -- Identidad de la evidencia para no contar dos veces el mismo éxito en el punto 4.
    evidencia_identidad TEXT NOT NULL CHECK (length(trim(evidencia_identidad)) > 0),
    cantidad_pasos INTEGER NOT NULL CHECK (cantidad_pasos >= 0),
    cantidad_incluidos INTEGER NOT NULL CHECK (cantidad_incluidos >= 0),
    cantidad_descartados INTEGER NOT NULL CHECK (cantidad_descartados >= 0),
    ultima_instancia_id INTEGER NOT NULL CHECK (ultima_instancia_id >= 0),
    ultimo_timestamp INTEGER NOT NULL,
    secuencia_original_sha256 TEXT NOT NULL CHECK (
        length(secuencia_original_sha256) = 64
        AND secuencia_original_sha256 NOT GLOB '*[^0-9a-f]*'),
    evidencia_sha256 TEXT NOT NULL CHECK (
        length(evidencia_sha256) = 64
        AND evidencia_sha256 NOT GLOB '*[^0-9a-f]*'),
    creada_en INTEGER NOT NULL,
    CHECK (cantidad_incluidos + cantidad_descartados = cantidad_pasos),
    CHECK (estado = 'no_candidata' OR resultado = 1),
    CHECK (estado = 'no_candidata' OR cantidad_pasos > 0),
    CHECK (estado <> 'candidata_sin_descartes' OR cantidad_descartados = 0),
    CHECK (estado <> 'candidata_con_descartes' OR cantidad_descartados > 0),
    CHECK ((origen = 'manual_tarea' AND verificacion = 'criterio_tarea')
        OR (origen = 'automatico_lista' AND verificacion = 'comprobacion_tecnica')),
    -- El tipo manual distingue la versión de la definición que fija el criterio.
    CHECK ((origen = 'manual_tarea' AND json_array_length(tipo_objetivo_json) = 5)
        OR (origen = 'automatico_lista' AND json_array_length(tipo_objetivo_json) = 2)),
    CHECK (origen <> 'manual_tarea' OR resultado IS NOT NULL),
    UNIQUE (origen, episodio_id)
);
CREATE INDEX arandu_secuencia_minima_proyecto
    ON arandu_secuencia_minima(proyecto, ultima_instancia_id, id);
CREATE INDEX arandu_secuencia_minima_tipo
    ON arandu_secuencia_minima(proyecto, tipo_objetivo_sha256, ultima_instancia_id DESC, id DESC);
CREATE INDEX arandu_secuencia_minima_evidencia
    ON arandu_secuencia_minima(origen, evidencia_identidad);

CREATE TABLE arandu_secuencia_minima_paso (
    reduccion_id INTEGER NOT NULL REFERENCES arandu_secuencia_minima(id),
    orden INTEGER NOT NULL CHECK (orden > 0),
    instancia_id INTEGER NOT NULL REFERENCES captura_evento(instancia_id),
    herramienta TEXT NOT NULL CHECK (length(trim(herramienta)) > 0),
    tipo_destino TEXT NOT NULL CHECK (length(trim(tipo_destino)) > 0),
    -- El fallo se conserva; no se interpreta como un desvío prescindible.
    estado_tecnico TEXT NOT NULL CHECK (estado_tecnico IN ('ok', 'error', 'desconocido')),
    incluido INTEGER NOT NULL CHECK (incluido IN (0, 1)),
    motivo_exclusion TEXT,
    PRIMARY KEY (reduccion_id, orden),
    UNIQUE (reduccion_id, instancia_id),
    CHECK ((incluido = 1 AND motivo_exclusion IS NULL)
        OR (incluido = 0 AND length(trim(motivo_exclusion)) > 0))
);
CREATE INDEX arandu_secuencia_minima_paso_instancia
    ON arandu_secuencia_minima_paso(instancia_id);

-- Se conservan todas las referencias tipadas entre pasos del mismo episodio.
-- `aplicada=0` documenta por qué una referencia no movió el camino candidato.
CREATE TABLE arandu_secuencia_minima_descarte (
    reduccion_id INTEGER NOT NULL,
    instancia_id INTEGER NOT NULL,
    evidencia_instancia_id INTEGER NOT NULL,
    tipo_referencia TEXT NOT NULL CHECK (tipo_referencia IN ('descarta_paso', 'revierte_paso')),
    aplicada INTEGER NOT NULL CHECK (aplicada IN (0, 1)),
    motivo_no_aplicada TEXT,
    orden_descartado INTEGER NOT NULL CHECK (orden_descartado > 0),
    orden_evidencia INTEGER NOT NULL CHECK (orden_evidencia > 0),
    PRIMARY KEY (reduccion_id, instancia_id, evidencia_instancia_id, tipo_referencia),
    FOREIGN KEY (reduccion_id, instancia_id)
        REFERENCES arandu_secuencia_minima_paso(reduccion_id, instancia_id),
    FOREIGN KEY (reduccion_id, evidencia_instancia_id)
        REFERENCES arandu_secuencia_minima_paso(reduccion_id, instancia_id),
    CHECK ((aplicada = 1 AND motivo_no_aplicada IS NULL)
        OR (aplicada = 0 AND length(trim(motivo_no_aplicada)) > 0))
);
CREATE INDEX arandu_secuencia_minima_descarte_evidencia
    ON arandu_secuencia_minima_descarte(reduccion_id, evidencia_instancia_id);

CREATE TABLE arandu_secuencia_minima_sello (
    reduccion_id INTEGER PRIMARY KEY REFERENCES arandu_secuencia_minima(id)
);

CREATE TRIGGER arandu_secuencia_minima_episodio BEFORE INSERT ON arandu_secuencia_minima
WHEN NOT EXISTS (
    SELECT 1 FROM episodio e JOIN tarea t ON t.id=e.tarea_id
      JOIN tarea_evento d ON d.id=e.definicion_evento_id
    WHERE NEW.origen='manual_tarea' AND e.id=NEW.episodio_id
      AND t.proyecto=NEW.proyecto
      AND e.normalizador_version=NEW.normalizador_version
      AND e.secuencia_sha256=NEW.secuencia_original_sha256
      AND e.cantidad_pasos=NEW.cantidad_pasos
      -- Un episodio manual sólo existe con su resultado sellado.
      AND EXISTS (SELECT 1 FROM episodio_resultado er WHERE er.episodio_id=e.id)
      AND NEW.resultado IS (SELECT er.resultado FROM episodio_resultado er WHERE er.episodio_id=e.id)
      AND NEW.evidencia_resultado_instancia_id IS (
          SELECT tr.fuente_instancia_id FROM episodio_resultado er
          JOIN tarea_resultado tr ON tr.id=er.tarea_resultado_id WHERE er.episodio_id=e.id)
      AND json_extract(NEW.tipo_objetivo_json,'$[0]')='manual_tarea'
      AND json_extract(NEW.tipo_objetivo_json,'$[1]')=t.clave
      AND json_extract(NEW.tipo_objetivo_json,'$[2]')=d.definicion_version
      AND json_extract(NEW.tipo_objetivo_json,'$[3]')=e.criterio_id
      AND json_extract(NEW.tipo_objetivo_json,'$[4]')=e.criterio_version
)
AND NOT EXISTS (
    SELECT 1 FROM episodio_automatico e
    WHERE NEW.origen='automatico_lista' AND e.id=NEW.episodio_id
      AND e.proyecto=NEW.proyecto
      AND e.normalizador_version=NEW.normalizador_version
      AND e.secuencia_sha256=NEW.secuencia_original_sha256
      AND e.cantidad_pasos=NEW.cantidad_pasos
      -- Sólo se deriva de un tramo sellado; sin sello no hay episodio cerrado.
      AND EXISTS (SELECT 1 FROM episodio_automatico_sello s WHERE s.episodio_id=e.id)
      AND NEW.resultado IS (SELECT r.resultado FROM episodio_automatico_resultado r WHERE r.episodio_id=e.id)
      AND NEW.evidencia_resultado_instancia_id IS (
          SELECT r.comprobacion_instancia_id FROM episodio_automatico_resultado r WHERE r.episodio_id=e.id)
      AND json_extract(NEW.tipo_objetivo_json,'$[0]')='automatico_lista'
      AND json_extract(NEW.tipo_objetivo_json,'$[1]')=e.objetivo
)
BEGIN SELECT RAISE(ABORT,'secuencia incompatible con el episodio'); END;

CREATE TRIGGER arandu_secuencia_minima_paso_validar BEFORE INSERT ON arandu_secuencia_minima_paso
WHEN EXISTS (SELECT 1 FROM arandu_secuencia_minima_sello WHERE reduccion_id=NEW.reduccion_id)
 OR NOT EXISTS (
    SELECT 1 FROM arandu_secuencia_minima r
    WHERE r.id=NEW.reduccion_id
      AND NEW.orden <= r.cantidad_pasos
      AND (
        (r.origen='manual_tarea' AND EXISTS (
            SELECT 1 FROM episodio_paso p
            WHERE p.episodio_id=r.episodio_id AND p.instancia_id=NEW.instancia_id
              AND p.orden=NEW.orden AND p.herramienta=NEW.herramienta
              AND p.tipo_destino=NEW.tipo_destino))
        OR
        (r.origen='automatico_lista' AND EXISTS (
            SELECT 1 FROM episodio_automatico_paso p
            WHERE p.episodio_id=r.episodio_id AND p.instancia_id=NEW.instancia_id
              AND p.orden=NEW.orden AND p.herramienta=NEW.herramienta
              AND p.tipo_destino=NEW.tipo_destino))
      )
 )
 OR NEW.orden <> (SELECT count(*)+1 FROM arandu_secuencia_minima_paso WHERE reduccion_id=NEW.reduccion_id)
BEGIN SELECT RAISE(ABORT,'paso de secuencia ajeno, fuera de orden o sellado'); END;

CREATE TRIGGER arandu_secuencia_minima_descarte_validar BEFORE INSERT ON arandu_secuencia_minima_descarte
WHEN EXISTS (SELECT 1 FROM arandu_secuencia_minima_sello WHERE reduccion_id=NEW.reduccion_id)
 OR NOT EXISTS (
    SELECT 1 FROM arandu_secuencia_minima_paso p
    WHERE p.reduccion_id=NEW.reduccion_id AND p.instancia_id=NEW.instancia_id
      AND p.orden=NEW.orden_descartado)
 OR NOT EXISTS (
    SELECT 1 FROM arandu_secuencia_minima_paso e
    WHERE e.reduccion_id=NEW.reduccion_id AND e.instancia_id=NEW.evidencia_instancia_id
      AND e.orden=NEW.orden_evidencia)
 OR (NEW.aplicada=1 AND NOT EXISTS (
    SELECT 1 FROM arandu_secuencia_minima_paso p, arandu_secuencia_minima_paso e
    WHERE p.reduccion_id=NEW.reduccion_id AND p.instancia_id=NEW.instancia_id
      AND p.incluido=0 AND e.reduccion_id=NEW.reduccion_id
      AND e.instancia_id=NEW.evidencia_instancia_id AND e.incluido=1
      AND NEW.orden_evidencia>NEW.orden_descartado))
BEGIN SELECT RAISE(ABORT,'descarte sin pasos, sin evidencia incluida o sellado'); END;

CREATE TRIGGER arandu_secuencia_minima_sello_validar BEFORE INSERT ON arandu_secuencia_minima_sello
WHEN NOT EXISTS (
    SELECT 1 FROM arandu_secuencia_minima r
    WHERE r.id=NEW.reduccion_id
      AND r.cantidad_pasos=(SELECT count(*) FROM arandu_secuencia_minima_paso WHERE reduccion_id=r.id)
      AND r.cantidad_incluidos=(SELECT count(*) FROM arandu_secuencia_minima_paso WHERE reduccion_id=r.id AND incluido=1)
      AND r.cantidad_descartados=(SELECT count(*) FROM arandu_secuencia_minima_paso WHERE reduccion_id=r.id AND incluido=0)
      AND r.cantidad_descartados=(SELECT count(DISTINCT instancia_id) FROM arandu_secuencia_minima_descarte
                                  WHERE reduccion_id=r.id AND aplicada=1)
      AND NOT EXISTS (
          SELECT 1 FROM arandu_secuencia_minima_paso p
          WHERE p.reduccion_id=r.id AND p.incluido=0 AND NOT EXISTS (
              SELECT 1 FROM arandu_secuencia_minima_descarte d
              WHERE d.reduccion_id=p.reduccion_id AND d.instancia_id=p.instancia_id AND d.aplicada=1))
      AND NOT EXISTS (
          SELECT 1 FROM arandu_secuencia_minima_descarte d
          JOIN arandu_secuencia_minima_paso pd
            ON pd.reduccion_id=d.reduccion_id AND pd.instancia_id=d.instancia_id
          JOIN arandu_secuencia_minima_paso pe
            ON pe.reduccion_id=d.reduccion_id AND pe.instancia_id=d.evidencia_instancia_id
          WHERE d.reduccion_id=r.id AND d.aplicada=1
            AND (pd.incluido<>0 OR pe.incluido<>1 OR d.orden_evidencia<=d.orden_descartado))
      AND (r.estado<>'candidata_sin_descartes' OR r.cantidad_descartados=0)
      AND (r.estado<>'candidata_con_descartes' OR r.cantidad_descartados>0)
      AND r.ultima_instancia_id=COALESCE((
          SELECT max(instancia_id) FROM arandu_secuencia_minima_paso WHERE reduccion_id=r.id),0)
      AND r.ultimo_timestamp=COALESCE((
          SELECT max(i.timestamp) FROM arandu_secuencia_minima_paso p JOIN instancia i ON i.id=p.instancia_id
          WHERE p.reduccion_id=r.id),0)
)
BEGIN SELECT RAISE(ABORT,'secuencia incompleta o inconsistente'); END;

CREATE TRIGGER arandu_secuencia_minima_update BEFORE UPDATE ON arandu_secuencia_minima
BEGIN SELECT RAISE(ABORT,'secuencia inmutable'); END;
CREATE TRIGGER arandu_secuencia_minima_delete BEFORE DELETE ON arandu_secuencia_minima
BEGIN SELECT RAISE(ABORT,'secuencia inmutable'); END;
CREATE TRIGGER arandu_secuencia_minima_paso_update BEFORE UPDATE ON arandu_secuencia_minima_paso
BEGIN SELECT RAISE(ABORT,'paso de secuencia inmutable'); END;
CREATE TRIGGER arandu_secuencia_minima_paso_delete BEFORE DELETE ON arandu_secuencia_minima_paso
BEGIN SELECT RAISE(ABORT,'paso de secuencia inmutable'); END;
CREATE TRIGGER arandu_secuencia_minima_descarte_update BEFORE UPDATE ON arandu_secuencia_minima_descarte
BEGIN SELECT RAISE(ABORT,'descarte de secuencia inmutable'); END;
CREATE TRIGGER arandu_secuencia_minima_descarte_delete BEFORE DELETE ON arandu_secuencia_minima_descarte
BEGIN SELECT RAISE(ABORT,'descarte de secuencia inmutable'); END;
CREATE TRIGGER arandu_secuencia_minima_sello_update BEFORE UPDATE ON arandu_secuencia_minima_sello
BEGIN SELECT RAISE(ABORT,'sello de secuencia inmutable'); END;
CREATE TRIGGER arandu_secuencia_minima_sello_delete BEFORE DELETE ON arandu_secuencia_minima_sello
BEGIN SELECT RAISE(ABORT,'sello de secuencia inmutable'); END;
