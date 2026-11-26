PRAGMA foreign_keys = ON;

CREATE TABLE configuracion (
    id INTEGER PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    parametros_json TEXT NOT NULL CHECK (json_valid(parametros_json))
);
CREATE TABLE dominio (id INTEGER PRIMARY KEY, proyecto TEXT NOT NULL);
CREATE TABLE contexto (
    id INTEGER PRIMARY KEY,
    dominio_id INTEGER NOT NULL REFERENCES dominio(id),
    estado TEXT NOT NULL DEFAULT 'provisional' CHECK (estado IN ('provisional', 'activo', 'archivado')),
    fecha_creacion INTEGER NOT NULL,
    fecha_actualizacion INTEGER NOT NULL
);
CREATE TABLE instancia (
    id INTEGER PRIMARY KEY,
    dominio_id INTEGER REFERENCES dominio(id),
    clave_evento TEXT NOT NULL UNIQUE,
    grupo_evidencia TEXT NOT NULL,
    origen TEXT NOT NULL CHECK (origen IN ('usuario', 'herramienta', 'modelo', 'replay')),
    rol TEXT NOT NULL CHECK (rol IN ('problema', 'solucion', 'correccion', 'otro')),
    contenido TEXT NOT NULL,
    metadatos_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadatos_json)),
    origen_feedback_id INTEGER REFERENCES evento_feedback(id), -- referencia adelantada: evento_feedback se declara más abajo; SQLite no exige orden
    timestamp INTEGER NOT NULL
);
CREATE TABLE trabajo_pendiente (
    id INTEGER PRIMARY KEY,
    instancia_id INTEGER NOT NULL REFERENCES instancia(id),
    configuracion_id INTEGER NOT NULL REFERENCES configuracion(id),
    etapa TEXT NOT NULL,
    estado TEXT NOT NULL DEFAULT 'pendiente'
        CHECK (estado IN ('pendiente', 'en_proceso', 'completado', 'fallido')),
    intentos INTEGER NOT NULL DEFAULT 0 CHECK (intentos >= 0),
    disponible_desde INTEGER NOT NULL,
    reserva_token TEXT,
    reserva_hasta INTEGER,
    ultimo_error TEXT,
    CHECK ((estado = 'en_proceso' AND reserva_token IS NOT NULL AND reserva_hasta IS NOT NULL)
        OR (estado <> 'en_proceso' AND reserva_token IS NULL AND reserva_hasta IS NULL)),
    UNIQUE (instancia_id, configuracion_id, etapa)
);
CREATE TABLE instancia_representacion (
    id INTEGER PRIMARY KEY,
    instancia_id INTEGER NOT NULL REFERENCES instancia(id),
    configuracion_id INTEGER NOT NULL REFERENCES configuracion(id),
    embedding BLOB,
    rasgos_json TEXT NOT NULL CHECK (json_valid(rasgos_json)),
    novedad_score REAL,
    timestamp INTEGER NOT NULL,
    UNIQUE (instancia_id, configuracion_id),
    UNIQUE (id, instancia_id)
);
CREATE TABLE observado_en (
    id INTEGER PRIMARY KEY,
    instancia_id INTEGER NOT NULL REFERENCES instancia(id),
    contexto_id INTEGER REFERENCES contexto(id),
    score REAL,
    timestamp INTEGER NOT NULL
);
CREATE TABLE contexto_peso_evento (
    id INTEGER PRIMARY KEY,
    contexto_id INTEGER NOT NULL REFERENCES contexto(id),
    timestamp INTEGER NOT NULL,
    delta REAL NOT NULL,
    clave_cambio TEXT NOT NULL UNIQUE,
    instancia_id INTEGER REFERENCES instancia(id),
    feedback_id INTEGER REFERENCES evento_feedback(id),
    configuracion_id INTEGER NOT NULL REFERENCES configuracion(id),
    motivo TEXT NOT NULL CHECK (motivo IN ('inicial', 'refuerzo', 'fuga', 'liquidacion'))
);
CREATE TABLE contexto_contenido_evento (
    id INTEGER PRIMARY KEY,
    contexto_id INTEGER NOT NULL REFERENCES contexto(id),
    timestamp INTEGER NOT NULL,
    contenido TEXT NOT NULL,
    motivo TEXT NOT NULL CHECK (motivo IN ('inicial', 'revision')),
    instancia_fuente_id INTEGER NOT NULL REFERENCES instancia(id),
    version_anterior_id INTEGER,
    UNIQUE (id, contexto_id),
    FOREIGN KEY (version_anterior_id, contexto_id)
        REFERENCES contexto_contenido_evento(id, contexto_id)
);
CREATE TABLE contexto_regla_evento (
    id INTEGER PRIMARY KEY,
    contexto_id INTEGER NOT NULL REFERENCES contexto(id),
    contenido_id INTEGER NOT NULL,
    timestamp INTEGER NOT NULL,
    motivo TEXT NOT NULL CHECK (motivo IN ('propuesta', 'activacion', 'restauracion', 'rechazo')),
    regla_origen_id INTEGER,
    configuracion_id INTEGER NOT NULL REFERENCES configuracion(id),
    criterio_json TEXT NOT NULL CHECK (json_valid(criterio_json)),
    evidencia_json TEXT NOT NULL CHECK (json_valid(evidencia_json)),
    UNIQUE (id, contexto_id),
    FOREIGN KEY (contenido_id, contexto_id)
        REFERENCES contexto_contenido_evento(id, contexto_id),
    FOREIGN KEY (regla_origen_id, contexto_id)
        REFERENCES contexto_regla_evento(id, contexto_id)
);
CREATE TABLE pertenece_a (
    id INTEGER PRIMARY KEY,
    instancia_id INTEGER NOT NULL REFERENCES instancia(id),
    contexto_id INTEGER NOT NULL REFERENCES contexto(id),
    vigente_desde INTEGER NOT NULL,
    motivo TEXT NOT NULL,
    peso_evento_id INTEGER REFERENCES contexto_peso_evento(id),
    UNIQUE (instancia_id, vigente_desde)
);
CREATE TABLE evento_recuperacion (
    id INTEGER PRIMARY KEY,
    tarea_hash TEXT NOT NULL,
    clave_evento TEXT NOT NULL UNIQUE,
    problema_instancia_id INTEGER NOT NULL REFERENCES instancia(id),
    representacion_id INTEGER,
    salida_instancia_id INTEGER REFERENCES instancia(id),
    configuracion_id INTEGER NOT NULL REFERENCES configuracion(id),
    salida_hash TEXT,
    timestamp INTEGER NOT NULL,
    FOREIGN KEY (representacion_id, problema_instancia_id)
        REFERENCES instancia_representacion(id, instancia_id)
);
CREATE TABLE evento_recuperacion_contexto (
    evento_id INTEGER NOT NULL REFERENCES evento_recuperacion(id),
    contexto_id INTEGER NOT NULL REFERENCES contexto(id),
    contenido_id INTEGER,
    regla_id INTEGER,
    regla_sombra_id INTEGER,
    similitud REAL NOT NULL,
    prob_exito REAL CHECK (prob_exito BETWEEN 0 AND 1),
    prob_sombra REAL CHECK (prob_sombra BETWEEN 0 AND 1),
    evaluado_detalle INTEGER NOT NULL CHECK (evaluado_detalle IN (0, 1)),
    score REAL NOT NULL,
    orden_inyeccion INTEGER,
    texto_inyectado TEXT,
    fue_inyectado INTEGER NOT NULL CHECK (fue_inyectado IN (0, 1)),
    PRIMARY KEY (evento_id, contexto_id),
    UNIQUE (evento_id, contenido_id),
    UNIQUE (evento_id, orden_inyeccion),
    CHECK ((fue_inyectado = 0 AND orden_inyeccion IS NULL AND texto_inyectado IS NULL)
        OR (fue_inyectado = 1 AND contenido_id IS NOT NULL
            AND orden_inyeccion IS NOT NULL AND orden_inyeccion > 0 AND texto_inyectado IS NOT NULL)),
    FOREIGN KEY (contenido_id, contexto_id)
        REFERENCES contexto_contenido_evento(id, contexto_id),
    FOREIGN KEY (regla_id, contexto_id)
        REFERENCES contexto_regla_evento(id, contexto_id),
    FOREIGN KEY (regla_sombra_id, contexto_id)
        REFERENCES contexto_regla_evento(id, contexto_id)
);
CREATE TABLE evento_recuperacion_instancia (
    evento_id INTEGER NOT NULL REFERENCES evento_recuperacion(id),
    instancia_id INTEGER NOT NULL REFERENCES instancia(id),
    PRIMARY KEY (evento_id, instancia_id)
);
CREATE TABLE evento_feedback (
    id INTEGER PRIMARY KEY,
    evento_recuperacion_id INTEGER NOT NULL REFERENCES evento_recuperacion(id),
    clave_evento TEXT NOT NULL UNIQUE,
    clave_evaluacion TEXT NOT NULL,
    contenido_objetivo_id INTEGER,
    tipo TEXT NOT NULL CHECK (tipo IN ('correccion_usuario', 'confirmacion_explicita',
        'fallo_detectado_downstream', 'exito_detectado_downstream')),
    fuente TEXT NOT NULL CHECK (fuente IN ('usuario_explicito', 'verificador_externo')),
    resultado INTEGER CHECK (resultado IN (0, 1)),
    nota_breve TEXT,
    timestamp INTEGER NOT NULL,
    FOREIGN KEY (evento_recuperacion_id, contenido_objetivo_id)
        REFERENCES evento_recuperacion_contexto(evento_id, contenido_id)
);
CREATE TABLE evento_uso (
    id INTEGER PRIMARY KEY,
    instancia_id INTEGER NOT NULL UNIQUE REFERENCES instancia(id),
    intento_clave TEXT NOT NULL,
    evento_recuperacion_id INTEGER REFERENCES evento_recuperacion(id),
    contenido_id INTEGER REFERENCES contexto_contenido_evento(id),
    tipo TEXT NOT NULL CHECK (tipo IN ('inicio', 'resultado', 'ajuste', 'prueba', 'interrupcion')),
    metricas_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metricas_json)),
    timestamp INTEGER NOT NULL,
    FOREIGN KEY (evento_recuperacion_id, contenido_id)
        REFERENCES evento_recuperacion_contexto(evento_id, contenido_id)
);
CREATE VIEW pertenece_a_vigencia AS
SELECT *, LEAD(vigente_desde) OVER (
    PARTITION BY instancia_id ORDER BY vigente_desde
) AS vigente_hasta
FROM pertenece_a;
CREATE VIEW pertenece_a_actual AS
SELECT * FROM pertenece_a_vigencia WHERE vigente_hasta IS NULL;
CREATE VIEW contexto_peso_actual AS
SELECT c.id AS contexto_id, COALESCE(SUM(e.delta), 0.0) AS peso
FROM contexto c LEFT JOIN contexto_peso_evento e ON e.contexto_id = c.id
GROUP BY c.id;
CREATE VIEW contexto_contenido_actual AS
SELECT id AS contenido_id, contexto_id, contenido, timestamp
FROM (
    SELECT id, contexto_id, contenido, timestamp,
           ROW_NUMBER() OVER (PARTITION BY contexto_id ORDER BY timestamp DESC, id DESC) AS orden
    FROM contexto_contenido_evento
) WHERE orden = 1;

CREATE VIEW contexto_regla_actual AS
SELECT id AS regla_id, contexto_id, contenido_id, criterio_json, timestamp
FROM (
    SELECT *, ROW_NUMBER() OVER (
        PARTITION BY contexto_id, contenido_id ORDER BY timestamp DESC, id DESC
    ) AS orden
    FROM contexto_regla_evento WHERE motivo IN ('activacion', 'restauracion')
) WHERE orden = 1;
CREATE TRIGGER proteger_configuracion_update BEFORE UPDATE ON configuracion
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_configuracion_delete BEFORE DELETE ON configuracion
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_instancia_representacion_update BEFORE UPDATE ON instancia_representacion
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_instancia_representacion_delete BEFORE DELETE ON instancia_representacion
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_observado_en_update BEFORE UPDATE ON observado_en
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_observado_en_delete BEFORE DELETE ON observado_en
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_contexto_peso_evento_update BEFORE UPDATE ON contexto_peso_evento
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_contexto_peso_evento_delete BEFORE DELETE ON contexto_peso_evento
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_contexto_contenido_evento_update BEFORE UPDATE ON contexto_contenido_evento
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_contexto_contenido_evento_delete BEFORE DELETE ON contexto_contenido_evento
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_contexto_regla_evento_update BEFORE UPDATE ON contexto_regla_evento
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_contexto_regla_evento_delete BEFORE DELETE ON contexto_regla_evento
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_pertenece_a_update BEFORE UPDATE ON pertenece_a
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_pertenece_a_delete BEFORE DELETE ON pertenece_a
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_evento_feedback_update BEFORE UPDATE ON evento_feedback
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_evento_feedback_delete BEFORE DELETE ON evento_feedback
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_instancia_crudo BEFORE UPDATE OF
    clave_evento, grupo_evidencia, origen, rol, contenido, metadatos_json, timestamp ON instancia
BEGIN SELECT RAISE(ABORT, 'evento crudo inmutable'); END;
CREATE TRIGGER proteger_instancia_delete BEFORE DELETE ON instancia
BEGIN SELECT RAISE(ABORT, 'evento crudo inmutable'); END;
CREATE TRIGGER proteger_evento_uso_update BEFORE UPDATE ON evento_uso
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER proteger_evento_uso_delete BEFORE DELETE ON evento_uso
BEGIN SELECT RAISE(ABORT, 'historial append-only'); END;
CREATE TRIGGER uso_version_inyectada BEFORE INSERT ON evento_uso
WHEN NEW.evento_recuperacion_id IS NOT NULL AND NEW.contenido_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM evento_recuperacion_contexto
    WHERE evento_id = NEW.evento_recuperacion_id
      AND contenido_id = NEW.contenido_id AND fue_inyectado = 1
)
BEGIN SELECT RAISE(ABORT, 'version de uso no inyectada'); END;
CREATE TRIGGER feedback_solo_inyectado BEFORE INSERT ON evento_feedback
WHEN NEW.contenido_objetivo_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM evento_recuperacion_contexto
    WHERE evento_id = NEW.evento_recuperacion_id
      AND contenido_id = NEW.contenido_objetivo_id AND fue_inyectado = 1
)
BEGIN SELECT RAISE(ABORT, 'version objetivo no inyectada'); END;

CREATE INDEX idx_obs_contexto ON observado_en(contexto_id, instancia_id);
CREATE INDEX idx_obs_instancia ON observado_en(instancia_id, timestamp);
CREATE INDEX idx_peso_contexto_tiempo ON contexto_peso_evento(contexto_id, timestamp, id);
CREATE INDEX idx_ctx_dominio ON contexto(dominio_id);
CREATE INDEX idx_inst_dominio ON instancia(dominio_id);
CREATE INDEX idx_evtctx_contexto ON evento_recuperacion_contexto(contexto_id, evento_id);
CREATE INDEX idx_feedback_evento ON evento_feedback(evento_recuperacion_id);
CREATE INDEX idx_contenido_contexto_tiempo ON contexto_contenido_evento(contexto_id, timestamp, id);
CREATE INDEX idx_instancia_origen_feedback ON instancia(origen_feedback_id);
CREATE INDEX idx_regla_contenido_tiempo ON contexto_regla_evento(contenido_id, timestamp, id);
CREATE INDEX idx_feedback_version ON evento_feedback(contenido_objetivo_id, clave_evaluacion);
CREATE INDEX idx_instancia_grupo ON instancia(grupo_evidencia);
CREATE INDEX idx_trabajo_listo ON trabajo_pendiente(estado, disponible_desde);
CREATE INDEX idx_trabajo_reserva ON trabajo_pendiente(estado, reserva_hasta);
CREATE INDEX idx_uso_intento ON evento_uso(intento_clave, timestamp, id);
CREATE INDEX idx_uso_version ON evento_uso(contenido_id, timestamp);
