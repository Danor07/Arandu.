-- Registro de control de la tanda 4. Migraciones propias, separadas de las de
-- contenido. Solo identidades, rutas y estado operativo: ni prompts, ni
-- respuestas, ni salidas de herramientas, ni textos recuperados.

CREATE TABLE memoria (
    id TEXT PRIMARY KEY CHECK (id GLOB '[0-9a-f]*' AND length(id) = 32),
    nombre TEXT NOT NULL UNIQUE CHECK (length(trim(nombre)) > 0),
    archivo_relativo TEXT NOT NULL UNIQUE CHECK (length(trim(archivo_relativo)) > 0),
    -- 0 mientras el archivo de contenido todavía no está migrado y sellado.
    lista INTEGER NOT NULL CHECK (lista IN (0, 1)),
    creada_en INTEGER NOT NULL
);

CREATE TABLE proyecto_local (
    id TEXT PRIMARY KEY CHECK (id GLOB '[0-9a-f]*' AND length(id) = 32),
    raiz_normalizada TEXT NOT NULL UNIQUE CHECK (length(trim(raiz_normalizada)) > 0),
    raiz_original TEXT NOT NULL,
    memoria_predeterminada TEXT REFERENCES memoria(id),
    registrado_en INTEGER NOT NULL
);

-- Una sesión = un chat. Queda ligada como máximo a una memoria durante su vida.
CREATE TABLE sesion_memoria (
    id INTEGER PRIMARY KEY,
    instalacion TEXT NOT NULL CHECK (length(trim(instalacion)) > 0),
    sesion_host TEXT NOT NULL CHECK (length(trim(sesion_host)) > 0),
    proyecto_id TEXT REFERENCES proyecto_local(id),
    memoria_id TEXT REFERENCES memoria(id),
    estado TEXT NOT NULL CHECK (estado IN ('apagada', 'encendida')),
    origen_seleccion TEXT NOT NULL CHECK (origen_seleccion IN
        ('sin_seleccion', 'predeterminado_proyecto', 'sesion_explicita')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    creada_en INTEGER NOT NULL,
    actualizado_en INTEGER NOT NULL,
    UNIQUE (instalacion, sesion_host),
    CHECK (estado = 'apagada' OR memoria_id IS NOT NULL),
    CHECK (origen_seleccion = 'sin_seleccion' OR memoria_id IS NOT NULL)
);

-- Delimita qué registros pueden capturarse. Un callback atrasado se rechaza
-- comparando su intervalo/revisión contra el estado vigente.
CREATE TABLE intervalo_captura (
    id INTEGER PRIMARY KEY,
    sesion_id INTEGER NOT NULL REFERENCES sesion_memoria(id),
    memoria_id TEXT NOT NULL REFERENCES memoria(id),
    revision_activacion INTEGER NOT NULL CHECK (revision_activacion >= 0),
    frontera_apertura TEXT NOT NULL CHECK (json_valid(frontera_apertura)),
    frontera_cierre TEXT CHECK (frontera_cierre IS NULL OR json_valid(frontera_cierre)),
    estado TEXT NOT NULL CHECK (estado IN ('abierto', 'cerrado')),
    abierto_en INTEGER NOT NULL,
    cerrado_en INTEGER,
    CHECK ((estado = 'abierto' AND cerrado_en IS NULL AND frontera_cierre IS NULL)
        OR (estado = 'cerrado' AND cerrado_en IS NOT NULL))
);
CREATE INDEX intervalo_sesion ON intervalo_captura(sesion_id, id);
CREATE UNIQUE INDEX intervalo_abierto_unico ON intervalo_captura(sesion_id)
    WHERE estado = 'abierto';
CREATE UNIQUE INDEX intervalo_revision ON intervalo_captura(sesion_id, revision_activacion);

-- Asignación determinista de identidad/secuencia por payload de hook. Se
-- confirma antes del commit del crudo: un replay reutiliza la misma asignación
-- en lugar de fabricar un evento nuevo.
CREATE TABLE asignacion_evento (
    clave_idem TEXT PRIMARY KEY CHECK (length(clave_idem) = 64),
    intervalo_id INTEGER NOT NULL REFERENCES intervalo_captura(id),
    clave_evento TEXT NOT NULL UNIQUE,
    grupo_evidencia TEXT NOT NULL,
    secuencia INTEGER NOT NULL CHECK (secuencia >= 0),
    -- Tiempo de fuente congelado: un replay reconstruye los mismos bytes.
    timestamp INTEGER NOT NULL,
    asignado_en INTEGER NOT NULL,
    UNIQUE (grupo_evidencia, secuencia)
);

-- Cursor por intervalo/fuente. Avanza DESPUÉS del commit del crudo; una caída
-- intermedia produce replay idempotente, nunca pérdida.
CREATE TABLE cursor_ingesta (
    intervalo_id INTEGER NOT NULL REFERENCES intervalo_captura(id),
    fuente TEXT NOT NULL CHECK (length(trim(fuente)) > 0),
    posicion TEXT NOT NULL,
    confirmados INTEGER NOT NULL CHECK (confirmados >= 0),
    actualizado_en INTEGER NOT NULL,
    PRIMARY KEY (intervalo_id, fuente)
);

CREATE TABLE control_historial (
    id INTEGER PRIMARY KEY,
    sesion_id INTEGER REFERENCES sesion_memoria(id),
    accion TEXT NOT NULL CHECK (accion IN
        ('memoria_creada', 'proyecto_registrado', 'predeterminado_fijado', 'predeterminado_quitado',
         'sesion_inicializada', 'memoria_encendida', 'memoria_apagada', 'callback_rechazado')),
    detalle_json TEXT NOT NULL CHECK (json_valid(detalle_json)),
    registrado_en INTEGER NOT NULL
);
CREATE INDEX control_historial_sesion ON control_historial(sesion_id, id);

CREATE TRIGGER asignacion_update BEFORE UPDATE ON asignacion_evento
BEGIN SELECT RAISE(ABORT, 'asignacion inmutable'); END;
CREATE TRIGGER asignacion_delete BEFORE DELETE ON asignacion_evento
BEGIN SELECT RAISE(ABORT, 'asignacion inmutable'); END;
CREATE TRIGGER control_historial_update BEFORE UPDATE ON control_historial
BEGIN SELECT RAISE(ABORT, 'historial de control append-only'); END;
CREATE TRIGGER control_historial_delete BEFORE DELETE ON control_historial
BEGIN SELECT RAISE(ABORT, 'historial de control append-only'); END;
CREATE TRIGGER memoria_delete BEFORE DELETE ON memoria
BEGIN SELECT RAISE(ABORT, 'esta tanda no borra memorias'); END;
-- Una sesión no cambia de memoria: apagar deja memoria_id y permite reactivarla.
CREATE TRIGGER sesion_memoria_fija BEFORE UPDATE OF memoria_id ON sesion_memoria
WHEN OLD.memoria_id IS NOT NULL AND NEW.memoria_id IS NOT OLD.memoria_id
BEGIN SELECT RAISE(ABORT, 'la sesion ya esta vinculada a otra memoria'); END;
CREATE TRIGGER sesion_revision_monotona BEFORE UPDATE OF revision ON sesion_memoria
WHEN NEW.revision < OLD.revision
BEGIN SELECT RAISE(ABORT, 'la revision no retrocede'); END;
CREATE TRIGGER intervalo_reapertura BEFORE UPDATE OF estado ON intervalo_captura
WHEN OLD.estado = 'cerrado' AND NEW.estado = 'abierto'
BEGIN SELECT RAISE(ABORT, 'un intervalo cerrado no se reabre'); END;
