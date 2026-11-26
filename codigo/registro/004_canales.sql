-- Canales entre agentes. A partir de esta migración el registro de control
-- guarda también el texto que una sesión publica deliberadamente en un canal.
-- Sigue sin guardar prompts, respuestas, salidas de herramientas ni texto
-- recuperado: un mensaje de canal sólo entra por la orden explícita de publicar.

-- El canal general es la fila sin proyecto; cada proyecto registrado tiene a lo
-- sumo un canal, creado la primera vez que una sesión suya lo usa.
CREATE TABLE canal (
    id INTEGER PRIMARY KEY,
    proyecto_id TEXT UNIQUE REFERENCES proyecto_local(id)
);
CREATE UNIQUE INDEX canal_general_unico ON canal((proyecto_id IS NULL))
    WHERE proyecto_id IS NULL;
INSERT INTO canal(id, proyecto_id) VALUES (1, NULL);

-- Historial append-only. El remitente es la fila de sesión del registro, nunca
-- un campo libre. El tope de bytes repite MENSAJE_MAX_BYTES como última barrera.
CREATE TABLE canal_mensaje (
    id INTEGER PRIMARY KEY CHECK (id > 0),
    canal_id INTEGER NOT NULL REFERENCES canal(id),
    remitente_id INTEGER NOT NULL REFERENCES sesion_memoria(id),
    audiencia TEXT NOT NULL CHECK (audiencia IN ('todos', 'activos', 'uno', 'rol')),
    responde_a INTEGER REFERENCES canal_mensaje(id),
    texto TEXT NOT NULL CHECK (typeof(texto) = 'text' AND length(trim(texto)) > 0
        AND length(CAST(texto AS BLOB)) <= 4096),
    regla TEXT NOT NULL CHECK (length(trim(regla)) > 0),
    publicado_en INTEGER NOT NULL
);
CREATE INDEX canal_mensaje_canal ON canal_mensaje(canal_id, id);
CREATE INDEX canal_mensaje_remitente ON canal_mensaje(remitente_id, id);

-- Destinatarios congelados al publicar: el único de «uno», los que estaban
-- activos en ese instante para «activos» o los que tenían el rol destino de un
-- pase («rol», migración 005). «todos» no tiene filas. Se insertan
-- antes que su mensaje, en la misma transacción (por eso la clave es diferida):
-- cuando el mensaje existe, su audiencia ya no admite cambios.
CREATE TABLE canal_destinatario (
    mensaje_id INTEGER NOT NULL REFERENCES canal_mensaje(id) DEFERRABLE INITIALLY DEFERRED,
    sesion_id INTEGER NOT NULL REFERENCES sesion_memoria(id),
    PRIMARY KEY (mensaje_id, sesion_id)
) WITHOUT ROWID;

-- Estado operativo, no historial: última señal de vida y cursor de lectura.
CREATE TABLE canal_presencia (
    sesion_id INTEGER PRIMARY KEY REFERENCES sesion_memoria(id),
    ultimo_latido INTEGER NOT NULL
);
CREATE TABLE canal_cursor (
    sesion_id INTEGER NOT NULL REFERENCES sesion_memoria(id),
    canal_id INTEGER NOT NULL REFERENCES canal(id),
    leido_hasta INTEGER NOT NULL CHECK (leido_hasta >= 0),
    PRIMARY KEY (sesion_id, canal_id)
) WITHOUT ROWID;

CREATE TRIGGER canal_update BEFORE UPDATE ON canal
BEGIN SELECT RAISE(ABORT, 'canal inmutable'); END;
CREATE TRIGGER canal_delete BEFORE DELETE ON canal
BEGIN SELECT RAISE(ABORT, 'canal inmutable'); END;
CREATE TRIGGER canal_mensaje_update BEFORE UPDATE ON canal_mensaje
BEGIN SELECT RAISE(ABORT, 'historial de canal append-only'); END;
CREATE TRIGGER canal_mensaje_delete BEFORE DELETE ON canal_mensaje
BEGIN SELECT RAISE(ABORT, 'historial de canal append-only'); END;
CREATE TRIGGER canal_destinatario_update BEFORE UPDATE ON canal_destinatario
BEGIN SELECT RAISE(ABORT, 'historial de canal append-only'); END;
CREATE TRIGGER canal_destinatario_delete BEFORE DELETE ON canal_destinatario
BEGIN SELECT RAISE(ABORT, 'historial de canal append-only'); END;
-- La respuesta queda en el mismo canal que la pregunta.
CREATE TRIGGER canal_respuesta_mismo_canal BEFORE INSERT ON canal_mensaje
WHEN NEW.responde_a IS NOT NULL AND NOT EXISTS
    (SELECT 1 FROM canal_mensaje WHERE id = NEW.responde_a AND canal_id = NEW.canal_id)
BEGIN SELECT RAISE(ABORT, 'la respuesta debe referenciar un mensaje del mismo canal'); END;
CREATE TRIGGER canal_cursor_monotono BEFORE UPDATE OF leido_hasta ON canal_cursor
WHEN NEW.leido_hasta < OLD.leido_hasta
BEGIN SELECT RAISE(ABORT, 'el cursor de lectura no retrocede'); END;
CREATE TRIGGER canal_cursor_delete BEFORE DELETE ON canal_cursor
BEGIN SELECT RAISE(ABORT, 'el cursor de lectura no se borra'); END;


-- Un INSERT OR REPLACE borra la fila anterior sin disparar los triggers de
-- DELETE (SQLite sólo lo hace con recursive_triggers, que es por conexión).
-- Estos BEFORE INSERT corren antes de resolver el conflicto y lo cierran.
CREATE TRIGGER canal_sin_reemplazo BEFORE INSERT ON canal
WHEN EXISTS (SELECT 1 FROM canal WHERE id = NEW.id OR proyecto_id IS NEW.proyecto_id)
BEGIN SELECT RAISE(ABORT, 'canal inmutable'); END;
CREATE TRIGGER canal_mensaje_sin_reemplazo BEFORE INSERT ON canal_mensaje
WHEN EXISTS (SELECT 1 FROM canal_mensaje WHERE id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'historial de canal append-only'); END;
CREATE TRIGGER canal_cursor_sin_reemplazo BEFORE INSERT ON canal_cursor
WHEN EXISTS (SELECT 1 FROM canal_cursor
    WHERE sesion_id = NEW.sesion_id AND canal_id = NEW.canal_id)
BEGIN SELECT RAISE(ABORT, 'el cursor de lectura no retrocede'); END;

-- En el canal de un proyecto sólo publican las sesiones de ese proyecto.
CREATE TRIGGER canal_mensaje_remitente_del_proyecto BEFORE INSERT ON canal_mensaje
WHEN EXISTS (SELECT 1 FROM canal c WHERE c.id = NEW.canal_id AND c.proyecto_id IS NOT NULL
    AND c.proyecto_id IS NOT (SELECT proyecto_id FROM sesion_memoria WHERE id = NEW.remitente_id))
BEGIN SELECT RAISE(ABORT, 'el remitente no pertenece al proyecto de este canal'); END;
-- Audiencia coherente con los destinatarios ya insertados para ese id.
CREATE TRIGGER canal_mensaje_audiencia_coherente BEFORE INSERT ON canal_mensaje
WHEN (NEW.audiencia = 'todos' AND EXISTS
        (SELECT 1 FROM canal_destinatario WHERE mensaje_id = NEW.id))
    OR (NEW.audiencia = 'uno' AND 1 <>
        (SELECT count(*) FROM canal_destinatario WHERE mensaje_id = NEW.id))
    OR (NEW.audiencia = 'rol' AND NOT EXISTS
        (SELECT 1 FROM canal_destinatario WHERE mensaje_id = NEW.id))
    OR EXISTS (SELECT 1 FROM canal_destinatario
        WHERE mensaje_id = NEW.id AND sesion_id = NEW.remitente_id)
BEGIN SELECT RAISE(ABORT, 'audiencia incoherente con sus destinatarios'); END;
CREATE TRIGGER canal_destinatario_congelado BEFORE INSERT ON canal_destinatario
WHEN EXISTS (SELECT 1 FROM canal_mensaje WHERE id = NEW.mensaje_id)
BEGIN SELECT RAISE(ABORT, 'los destinatarios se congelan al publicar'); END;
