-- La capa de proyectos: varias carpetas por proyecto, roles con procedimiento
-- escrito y reglas del proyecto. Procedimientos y reglas son contenido que el
-- dueño registra por CLI: entran bajo la misma promesa reescrita en la 004.

-- Todas las carpetas de un proyecto, incluida la raíz con la que se registró.
-- La identidad sigue siendo `proyecto_local` (id y raiz_normalizada), que no se
-- toca: cuál carpeta es la principal es sólo una marca.
CREATE TABLE proyecto_carpeta (
    raiz_normalizada TEXT PRIMARY KEY CHECK (length(trim(raiz_normalizada)) > 0),
    proyecto_id TEXT NOT NULL REFERENCES proyecto_local(id),
    raiz_original TEXT NOT NULL,
    principal INTEGER NOT NULL CHECK (principal IN (0, 1)),
    agregada_en INTEGER NOT NULL
) WITHOUT ROWID;
CREATE UNIQUE INDEX proyecto_carpeta_principal ON proyecto_carpeta(proyecto_id)
    WHERE principal = 1;
INSERT INTO proyecto_carpeta
    SELECT raiz_normalizada, id, raiz_original, 1, registrado_en FROM proyecto_local;

-- Un BEFORE INSERT corre antes de resolver el conflicto: cierra INSERT OR
-- REPLACE, que no dispara los triggers de DELETE.
CREATE TRIGGER proyecto_carpeta_sin_reemplazo BEFORE INSERT ON proyecto_carpeta
WHEN EXISTS (SELECT 1 FROM proyecto_carpeta WHERE raiz_normalizada = NEW.raiz_normalizada
    OR (NEW.principal = 1 AND proyecto_id = NEW.proyecto_id AND principal = 1))
BEGIN SELECT RAISE(ABORT, 'esa carpeta ya pertenece a un proyecto registrado, o el proyecto ya tiene principal'); END;
-- Registrar un proyecto le da su primera carpeta, que nace principal.
CREATE TRIGGER proyecto_carpeta_inicial AFTER INSERT ON proyecto_local
BEGIN
    INSERT INTO proyecto_carpeta
        VALUES (NEW.raiz_normalizada, NEW.id, NEW.raiz_original, 1, NEW.registrado_en);
END;
CREATE TRIGGER proyecto_carpeta_update BEFORE UPDATE ON proyecto_carpeta
WHEN NEW.raiz_normalizada IS NOT OLD.raiz_normalizada OR NEW.proyecto_id IS NOT OLD.proyecto_id
    OR NEW.raiz_original IS NOT OLD.raiz_original OR NEW.agregada_en IS NOT OLD.agregada_en
BEGIN SELECT RAISE(ABORT, 'de una carpeta solo cambia si es la principal'); END;
CREATE TRIGGER proyecto_carpeta_delete BEFORE DELETE ON proyecto_carpeta
BEGIN SELECT RAISE(ABORT, 'las carpetas de un proyecto no se quitan'); END;

-- Procedimiento de un rol, versionado: cada cambio es una fila nueva con la
-- versión siguiente. `pasa_a` nombra otro rol del mismo proyecto.
CREATE TABLE proyecto_rol (
    id INTEGER PRIMARY KEY CHECK (id > 0),
    proyecto_id TEXT NOT NULL REFERENCES proyecto_local(id),
    nombre TEXT NOT NULL CHECK (typeof(nombre) = 'text' AND length(trim(nombre)) > 0
        AND length(CAST(nombre AS BLOB)) <= 64),
    version INTEGER NOT NULL CHECK (version >= 1),
    recibe TEXT NOT NULL CHECK (typeof(recibe) = 'text' AND length(trim(recibe)) > 0
        AND length(CAST(recibe AS BLOB)) <= 4096),
    entrega TEXT NOT NULL CHECK (typeof(entrega) = 'text' AND length(trim(entrega)) > 0
        AND length(CAST(entrega AS BLOB)) <= 4096),
    terminado TEXT NOT NULL CHECK (typeof(terminado) = 'text' AND length(trim(terminado)) > 0
        AND length(CAST(terminado AS BLOB)) <= 4096),
    pasa_a TEXT NOT NULL CHECK (typeof(pasa_a) = 'text' AND length(trim(pasa_a)) > 0
        AND length(CAST(pasa_a AS BLOB)) <= 64 AND pasa_a <> nombre),
    regla TEXT NOT NULL CHECK (length(trim(regla)) > 0),
    definido_en INTEGER NOT NULL,
    UNIQUE (proyecto_id, nombre, version)
);

-- Reglas del proyecto, independientes de la plataforma: texto versionado.
CREATE TABLE proyecto_reglas (
    id INTEGER PRIMARY KEY CHECK (id > 0),
    proyecto_id TEXT NOT NULL REFERENCES proyecto_local(id),
    version INTEGER NOT NULL CHECK (version >= 1),
    texto TEXT NOT NULL CHECK (typeof(texto) = 'text' AND length(trim(texto)) > 0
        AND length(CAST(texto AS BLOB)) <= 16384),
    regla TEXT NOT NULL CHECK (length(trim(regla)) > 0),
    fijado_en INTEGER NOT NULL,
    UNIQUE (proyecto_id, version)
);

-- Qué rol tiene cada sesión: historial append-only; vale la última fila de la
-- sesión, y `rol` nulo significa que lo soltó.
CREATE TABLE sesion_rol (
    id INTEGER PRIMARY KEY CHECK (id > 0),
    sesion_id INTEGER NOT NULL REFERENCES sesion_memoria(id),
    rol TEXT,
    tomado_en INTEGER NOT NULL
);
CREATE INDEX sesion_rol_sesion ON sesion_rol(sesion_id, id);

-- Un pase entre roles es un mensaje de canal con audiencia `rol`; acá queda de
-- qué rol (y versión de su procedimiento) a qué rol.
CREATE TABLE rol_pase (
    mensaje_id INTEGER PRIMARY KEY REFERENCES canal_mensaje(id),
    rol_origen TEXT NOT NULL,
    rol_origen_version INTEGER NOT NULL,
    rol_destino TEXT NOT NULL
);

-- La versión de un rol o de las reglas es siempre la siguiente: ni saltos ni
-- reemplazo de una versión que ya existe.
CREATE TRIGGER proyecto_rol_version BEFORE INSERT ON proyecto_rol
WHEN NEW.version <> 1 + (SELECT coalesce(max(version), 0) FROM proyecto_rol
        WHERE proyecto_id = NEW.proyecto_id AND nombre = NEW.nombre)
    OR EXISTS (SELECT 1 FROM proyecto_rol WHERE id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'la version de un rol es la siguiente a la ultima'); END;
CREATE TRIGGER proyecto_reglas_version BEFORE INSERT ON proyecto_reglas
WHEN NEW.version <> 1 + (SELECT coalesce(max(version), 0) FROM proyecto_reglas
        WHERE proyecto_id = NEW.proyecto_id)
    OR EXISTS (SELECT 1 FROM proyecto_reglas WHERE id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'la version de las reglas es la siguiente a la ultima'); END;
-- Sólo se toma un rol definido en el proyecto de la sesión.
CREATE TRIGGER sesion_rol_del_proyecto BEFORE INSERT ON sesion_rol
WHEN EXISTS (SELECT 1 FROM sesion_rol WHERE id = NEW.id)
    OR (NEW.rol IS NOT NULL AND NOT EXISTS (SELECT 1 FROM proyecto_rol r
        JOIN sesion_memoria s ON s.proyecto_id = r.proyecto_id
        WHERE s.id = NEW.sesion_id AND r.nombre = NEW.rol))
BEGIN SELECT RAISE(ABORT, 'rol sin definir en el proyecto de la sesion'); END;
CREATE TRIGGER rol_pase_de_un_mensaje_de_rol BEFORE INSERT ON rol_pase
WHEN EXISTS (SELECT 1 FROM rol_pase WHERE mensaje_id = NEW.mensaje_id)
    OR NOT EXISTS (SELECT 1 FROM canal_mensaje
        WHERE id = NEW.mensaje_id AND audiencia = 'rol')
BEGIN SELECT RAISE(ABORT, 'un pase acompana a un mensaje con audiencia rol'); END;

CREATE TRIGGER proyecto_rol_update BEFORE UPDATE ON proyecto_rol
BEGIN SELECT RAISE(ABORT, 'procedimiento de rol append-only'); END;
CREATE TRIGGER proyecto_rol_delete BEFORE DELETE ON proyecto_rol
BEGIN SELECT RAISE(ABORT, 'procedimiento de rol append-only'); END;
CREATE TRIGGER proyecto_reglas_update BEFORE UPDATE ON proyecto_reglas
BEGIN SELECT RAISE(ABORT, 'reglas de proyecto append-only'); END;
CREATE TRIGGER proyecto_reglas_delete BEFORE DELETE ON proyecto_reglas
BEGIN SELECT RAISE(ABORT, 'reglas de proyecto append-only'); END;
CREATE TRIGGER sesion_rol_update BEFORE UPDATE ON sesion_rol
BEGIN SELECT RAISE(ABORT, 'historial de roles append-only'); END;
CREATE TRIGGER sesion_rol_delete BEFORE DELETE ON sesion_rol
BEGIN SELECT RAISE(ABORT, 'historial de roles append-only'); END;
CREATE TRIGGER rol_pase_update BEFORE UPDATE ON rol_pase
BEGIN SELECT RAISE(ABORT, 'historial de pases append-only'); END;
CREATE TRIGGER rol_pase_delete BEFORE DELETE ON rol_pase
BEGIN SELECT RAISE(ABORT, 'historial de pases append-only'); END;
