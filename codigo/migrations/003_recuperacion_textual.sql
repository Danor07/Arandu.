-- Proyecto declarado por evento. Un dato ausente o inválido queda NULL;
-- no se infiere a partir de otros eventos del episodio.
CREATE VIEW instancia_proyecto AS
SELECT id AS instancia_id,
    CASE WHEN json_type(metadatos_json, '$.proyecto')='text'
          AND length(trim(json_extract(metadatos_json, '$.proyecto')))>0
         THEN json_extract(metadatos_json, '$.proyecto') END AS proyecto
FROM instancia;

CREATE VIRTUAL TABLE instancia_fts USING fts5(
    contenido, content='instancia', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER instancia_fts_insert AFTER INSERT ON instancia BEGIN
    INSERT INTO instancia_fts(rowid,contenido) VALUES (NEW.id,NEW.contenido);
END;
-- El crudo ya es inmutable. Rebuild incluye todas las filas anteriores.
INSERT INTO instancia_fts(instancia_fts) VALUES ('rebuild');

CREATE TABLE paquete_recuperacion (
    evento_id INTEGER PRIMARY KEY REFERENCES evento_recuperacion(id),
    proyecto TEXT NOT NULL,
    parametros_json TEXT NOT NULL CHECK(json_valid(parametros_json)),
    presupuesto_bytes INTEGER NOT NULL CHECK(presupuesto_bytes>=0),
    texto_memoria TEXT NOT NULL,
    bytes_utf8 INTEGER NOT NULL CHECK(bytes_utf8=length(CAST(texto_memoria AS BLOB))),
    paquete_json TEXT NOT NULL CHECK(json_valid(paquete_json)),
    CHECK(bytes_utf8<=presupuesto_bytes)
);
CREATE TABLE recuperacion_entrega_instancia (
    evento_id INTEGER NOT NULL,
    instancia_id INTEGER NOT NULL,
    orden INTEGER NOT NULL CHECK(orden>0),
    score_textual REAL NOT NULL,
    texto_entregado TEXT NOT NULL,
    PRIMARY KEY(evento_id,instancia_id),
    UNIQUE(evento_id,orden),
    FOREIGN KEY(evento_id,instancia_id)
        REFERENCES evento_recuperacion_instancia(evento_id,instancia_id)
);
CREATE TRIGGER paquete_update BEFORE UPDATE ON paquete_recuperacion
BEGIN SELECT RAISE(ABORT,'paquete inmutable'); END;
CREATE TRIGGER paquete_delete BEFORE DELETE ON paquete_recuperacion
BEGIN SELECT RAISE(ABORT,'paquete inmutable'); END;
CREATE TRIGGER entrega_update BEFORE UPDATE ON recuperacion_entrega_instancia
BEGIN SELECT RAISE(ABORT,'entrega inmutable'); END;
CREATE TRIGGER entrega_delete BEFORE DELETE ON recuperacion_entrega_instancia
BEGIN SELECT RAISE(ABORT,'entrega inmutable'); END;
-- La inserción del paquete sella la selección. No afecta recuperaciones futuras
-- que tengan su propio escritor y aún no hayan preparado un paquete episódico.
CREATE TRIGGER entrega_sellada BEFORE INSERT ON recuperacion_entrega_instancia
WHEN EXISTS(SELECT 1 FROM paquete_recuperacion WHERE evento_id=NEW.evento_id)
BEGIN SELECT RAISE(ABORT,'seleccion sellada'); END;
CREATE TRIGGER recuperacion_instancia_sellada_insert BEFORE INSERT ON evento_recuperacion_instancia
WHEN EXISTS(SELECT 1 FROM paquete_recuperacion WHERE evento_id=NEW.evento_id)
BEGIN SELECT RAISE(ABORT,'seleccion sellada'); END;
CREATE TRIGGER recuperacion_instancia_sellada_update BEFORE UPDATE ON evento_recuperacion_instancia
WHEN EXISTS(SELECT 1 FROM paquete_recuperacion WHERE evento_id IN (OLD.evento_id,NEW.evento_id))
BEGIN SELECT RAISE(ABORT,'seleccion sellada'); END;
CREATE TRIGGER recuperacion_instancia_sellada_delete BEFORE DELETE ON evento_recuperacion_instancia
WHEN EXISTS(SELECT 1 FROM paquete_recuperacion WHERE evento_id=OLD.evento_id)
BEGIN SELECT RAISE(ABORT,'seleccion sellada'); END;
CREATE TRIGGER recuperacion_paquete_update BEFORE UPDATE ON evento_recuperacion
WHEN EXISTS(SELECT 1 FROM paquete_recuperacion WHERE evento_id=OLD.id)
BEGIN SELECT RAISE(ABORT,'recuperacion episodica sellada'); END;
CREATE TRIGGER recuperacion_paquete_delete BEFORE DELETE ON evento_recuperacion
WHEN EXISTS(SELECT 1 FROM paquete_recuperacion WHERE evento_id=OLD.id)
BEGIN SELECT RAISE(ABORT,'recuperacion episodica sellada'); END;
