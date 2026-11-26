-- Episodio explícito: una instantánea de tarea, criterio, pasos y comprobación.
-- No cambia el crudo ni el significado histórico de grupo_evidencia.
CREATE TABLE episodio (
    id INTEGER PRIMARY KEY,
    clave TEXT NOT NULL UNIQUE CHECK (length(trim(clave)) > 0),
    tarea_id INTEGER NOT NULL REFERENCES tarea(id),
    definicion_evento_id INTEGER NOT NULL REFERENCES tarea_evento(id),
    cierre_evento_id INTEGER NOT NULL REFERENCES tarea_evento(id),
    criterio_id TEXT NOT NULL CHECK (length(trim(criterio_id)) > 0),
    criterio_version TEXT NOT NULL CHECK (length(trim(criterio_version)) > 0),
    normalizador_version TEXT NOT NULL CHECK (normalizador_version = 'herramienta-destino-v1'),
    secuencia_sha256 TEXT NOT NULL CHECK (length(secuencia_sha256) = 64 AND secuencia_sha256 NOT GLOB '*[^0-9a-f]*'),
    cantidad_pasos INTEGER NOT NULL CHECK (cantidad_pasos >= 0),
    creada_en INTEGER NOT NULL
);
CREATE INDEX episodio_tarea ON episodio(tarea_id, id);

CREATE TABLE episodio_paso (
    episodio_id INTEGER NOT NULL REFERENCES episodio(id),
    orden INTEGER NOT NULL CHECK (orden > 0),
    instancia_id INTEGER NOT NULL REFERENCES captura_evento(instancia_id),
    herramienta TEXT NOT NULL CHECK (herramienta IN
        ('Read','Edit','Write','Bash','Grep','Glob','WebFetch','WebSearch','Task','MCP','otro')),
    tipo_destino TEXT NOT NULL CHECK (length(trim(tipo_destino)) > 0),
    PRIMARY KEY (episodio_id, orden),
    UNIQUE (episodio_id, instancia_id)
);

-- La fila de resultado sella el episodio: después no se agregan pasos.
-- NULL no es cero. El hook no expone tokens ni una correlación de reintentos.
CREATE TABLE episodio_resultado (
    episodio_id INTEGER PRIMARY KEY REFERENCES episodio(id),
    tarea_resultado_id INTEGER NOT NULL REFERENCES tarea_resultado(id),
    resultado INTEGER NOT NULL CHECK (resultado IN (0, 1)),
    duracion_microsegundos INTEGER CHECK (duracion_microsegundos >= 0),
    reintentos INTEGER CHECK (reintentos >= 0),
    tokens INTEGER CHECK (tokens >= 0)
);

CREATE TRIGGER episodio_objetivo BEFORE INSERT ON episodio
WHEN NOT EXISTS (
    SELECT 1 FROM tarea_evento d JOIN tarea_evento c ON c.tarea_id=d.tarea_id,
         json_each(d.criterios_json) criterio
    WHERE d.id=NEW.definicion_evento_id AND d.tarea_id=NEW.tarea_id AND d.tipo='definicion'
      AND c.id=NEW.cierre_evento_id AND c.tipo='cierre'
      AND c.definicion_version=d.definicion_version
      AND json_extract(criterio.value,'$.id')=NEW.criterio_id
      AND json_extract(criterio.value,'$.version')=NEW.criterio_version
)
BEGIN SELECT RAISE(ABORT, 'objetivo o cierre incompatible'); END;

CREATE TRIGGER episodio_paso_validar BEFORE INSERT ON episodio_paso
WHEN EXISTS (SELECT 1 FROM episodio_resultado WHERE episodio_id=NEW.episodio_id)
 OR NOT EXISTS (SELECT 1 FROM episodio e JOIN tarea t ON t.id=e.tarea_id
                JOIN tarea_vinculo v ON v.tarea_id=e.tarea_id JOIN instancia i ON i.id=v.instancia_id
                WHERE e.id=NEW.episodio_id AND v.instancia_id=NEW.instancia_id
                  AND i.origen='herramienta' AND json_extract(i.metadatos_json,'$.proyecto')=t.proyecto)
 OR NEW.orden <> (SELECT count(*)+1 FROM episodio_paso WHERE episodio_id=NEW.episodio_id)
 OR NEW.orden > (SELECT cantidad_pasos FROM episodio WHERE id=NEW.episodio_id)
BEGIN SELECT RAISE(ABORT, 'paso ajeno, fuera de orden o episodio sellado'); END;

CREATE TRIGGER episodio_resultado_validar BEFORE INSERT ON episodio_resultado
WHEN NOT EXISTS (
    SELECT 1 FROM episodio e JOIN tarea_evento d ON d.id=e.definicion_evento_id
      JOIN tarea_resultado r ON r.tarea_id=e.tarea_id
    WHERE e.id=NEW.episodio_id AND r.id=NEW.tarea_resultado_id
      AND r.definicion_version=d.definicion_version
      AND r.criterio_id=e.criterio_id AND r.criterio_version=e.criterio_version
      AND r.alcance='tarea' AND r.resultado=NEW.resultado
      AND NOT EXISTS (SELECT 1 FROM tarea_resultado posterior WHERE posterior.anterior_id=r.id)
      AND e.cantidad_pasos=(SELECT count(*) FROM episodio_paso WHERE episodio_id=e.id)
)
BEGIN SELECT RAISE(ABORT, 'resultado incompatible, corregido o pasos incompletos'); END;

CREATE TRIGGER episodio_update BEFORE UPDATE ON episodio
BEGIN SELECT RAISE(ABORT, 'episodio inmutable'); END;
CREATE TRIGGER episodio_delete BEFORE DELETE ON episodio
BEGIN SELECT RAISE(ABORT, 'episodio inmutable'); END;
CREATE TRIGGER episodio_paso_update BEFORE UPDATE ON episodio_paso
BEGIN SELECT RAISE(ABORT, 'paso de episodio inmutable'); END;
CREATE TRIGGER episodio_paso_delete BEFORE DELETE ON episodio_paso
BEGIN SELECT RAISE(ABORT, 'paso de episodio inmutable'); END;
CREATE TRIGGER episodio_resultado_update BEFORE UPDATE ON episodio_resultado
BEGIN SELECT RAISE(ABORT, 'resultado de episodio inmutable'); END;
CREATE TRIGGER episodio_resultado_delete BEFORE DELETE ON episodio_resultado
BEGIN SELECT RAISE(ABORT, 'resultado de episodio inmutable'); END;
