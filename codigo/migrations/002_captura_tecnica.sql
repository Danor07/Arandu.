-- Extensiones de transporte y operación; el DDL normativo permanece en 001.
CREATE TABLE captura_evento (
    instancia_id INTEGER PRIMARY KEY REFERENCES instancia(id),
    grupo_evidencia TEXT NOT NULL,
    secuencia INTEGER NOT NULL CHECK (secuencia >= 0),
    json_original TEXT NOT NULL CHECK (json_valid(json_original)),
    recibido_en INTEGER NOT NULL,
    UNIQUE (grupo_evidencia, secuencia)
);
CREATE TABLE resultado_tecnico (
    trabajo_id INTEGER PRIMARY KEY REFERENCES trabajo_pendiente(id),
    clave_resultado TEXT NOT NULL UNIQUE,
    reserva_token TEXT NOT NULL,
    resultado_json TEXT NOT NULL CHECK (json_valid(resultado_json)),
    timestamp INTEGER NOT NULL
);
CREATE TABLE trabajo_historial (
    id INTEGER PRIMARY KEY,
    trabajo_id INTEGER NOT NULL REFERENCES trabajo_pendiente(id),
    tipo TEXT NOT NULL CHECK (tipo IN ('reserva', 'completado', 'reintento', 'fallido', 'reanudado')),
    reserva_token TEXT,
    detalle_json TEXT NOT NULL CHECK (json_valid(detalle_json)),
    timestamp INTEGER NOT NULL
);
CREATE INDEX idx_trabajo_historia ON trabajo_historial(trabajo_id, id);
CREATE TRIGGER captura_update BEFORE UPDATE ON captura_evento
BEGIN SELECT RAISE(ABORT, 'captura inmutable'); END;
CREATE TRIGGER captura_delete BEFORE DELETE ON captura_evento
BEGIN SELECT RAISE(ABORT, 'captura inmutable'); END;
CREATE TRIGGER resultado_tecnico_update BEFORE UPDATE ON resultado_tecnico
BEGIN SELECT RAISE(ABORT, 'resultado tecnico inmutable'); END;
CREATE TRIGGER resultado_tecnico_delete BEFORE DELETE ON resultado_tecnico
BEGIN SELECT RAISE(ABORT, 'resultado tecnico inmutable'); END;
CREATE TRIGGER trabajo_historial_update BEFORE UPDATE ON trabajo_historial
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER trabajo_historial_delete BEFORE DELETE ON trabajo_historial
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER trabajo_conservar BEFORE DELETE ON trabajo_pendiente
BEGIN SELECT RAISE(ABORT, 'conservar trabajo'); END;
CREATE TRIGGER captura_grupo BEFORE INSERT ON captura_evento
WHEN NOT EXISTS (SELECT 1 FROM instancia WHERE id=NEW.instancia_id AND grupo_evidencia=NEW.grupo_evidencia)
BEGIN SELECT RAISE(ABORT, 'grupo de captura incompatible'); END;
CREATE TRIGGER uso_intento_episodio BEFORE INSERT ON evento_uso
WHEN EXISTS (
    SELECT 1 FROM evento_uso u JOIN instancia previa ON previa.id=u.instancia_id
    JOIN instancia nueva ON nueva.id=NEW.instancia_id
    WHERE u.intento_clave=NEW.intento_clave AND previa.grupo_evidencia<>nueva.grupo_evidencia
)
BEGIN SELECT RAISE(ABORT, 'intento pertenece a otro episodio'); END;
