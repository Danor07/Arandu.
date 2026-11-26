-- Competencia entre caminos, archivo y vinculos migrados. No reescribe episodios ni minimas.

-- 010 fijo la regla en un CHECK literal. Se reconstruye la tabla, con las mismas filas,
-- para admitir reglas posteriores: la version la fija el codigo que escribe.
DROP TRIGGER paquete_vigente_validar;
CREATE TABLE recuperacion_vigente_012 (
    evento_id INTEGER PRIMARY KEY REFERENCES evento_recuperacion(id),
    tipo_clave TEXT NOT NULL REFERENCES tipo_objetivo(clave),
    derivacion_id INTEGER NOT NULL REFERENCES secuencia_minima(id),
    regla_version TEXT NOT NULL CHECK(length(trim(regla_version))>0),
    texto_entregado TEXT NOT NULL CHECK(length(CAST(texto_entregado AS BLOB))>0),
    evidencia_json TEXT NOT NULL CHECK(json_valid(evidencia_json))
);
INSERT INTO recuperacion_vigente_012 SELECT * FROM recuperacion_vigente;
DROP TABLE recuperacion_vigente;
-- El renombre moderno revalida todo el esquema y rechaza la vista objetivo_episodio_fuente
-- de 010 (join entre parentesis). Nada referencia el nombre temporal: alcanza el modo legado.
PRAGMA legacy_alter_table=ON;
ALTER TABLE recuperacion_vigente_012 RENAME TO recuperacion_vigente;
PRAGMA legacy_alter_table=OFF;
CREATE TRIGGER recuperacion_vigente_validar BEFORE INSERT ON recuperacion_vigente
WHEN EXISTS(SELECT 1 FROM paquete_recuperacion WHERE evento_id=NEW.evento_id)
 OR NOT EXISTS(SELECT 1 FROM secuencia_minima m JOIN secuencia_minima_sello s ON s.derivacion_id=m.id
    JOIN episodio_tipo_objetivo et ON et.origen=m.origen
        AND et.episodio_id=coalesce(m.episodio_manual_id,m.episodio_automatico_id)
    JOIN tipo_objetivo t ON t.clave=et.tipo_clave
    JOIN evento_recuperacion r ON r.id=NEW.evento_id JOIN instancia_proyecto p ON p.instancia_id=r.problema_instancia_id
    WHERE m.id=NEW.derivacion_id AND m.llego=1 AND et.tipo_clave=NEW.tipo_clave AND t.proyecto=p.proyecto)
BEGIN SELECT RAISE(ABORT,'vigente ajena, sin exito o paquete sellado'); END;
CREATE TRIGGER paquete_vigente_validar BEFORE INSERT ON paquete_recuperacion
WHEN EXISTS(SELECT 1 FROM recuperacion_vigente v WHERE v.evento_id=NEW.evento_id
    AND substr(NEW.texto_memoria,1,length(v.texto_entregado))<>v.texto_entregado)
BEGIN SELECT RAISE(ABORT,'la vigente debe encabezar el paquete completo'); END;
CREATE TRIGGER recuperacion_vigente_update BEFORE UPDATE ON recuperacion_vigente
BEGIN SELECT RAISE(ABORT,'vigente entregada inmutable'); END;
CREATE TRIGGER recuperacion_vigente_delete BEFORE DELETE ON recuperacion_vigente
BEGIN SELECT RAISE(ABORT,'vigente entregada inmutable'); END;

-- Caminos (hash minimo) con al menos una derivacion sellada exitosa dentro de cada tipo.
CREATE VIEW camino_derivacion AS
SELECT et.tipo_clave,m.hash_minima,m.id AS derivacion_id
FROM secuencia_minima m JOIN secuencia_minima_sello s ON s.derivacion_id=m.id
JOIN episodio_tipo_objetivo et ON et.origen=m.origen
    AND et.episodio_id=coalesce(m.episodio_manual_id,m.episodio_automatico_id)
WHERE m.llego=1;

-- Vinculo: una entrega ya sellada que ofrecio el camino que despues se archiva.
-- La entrega no se toca; esta fila la redirige al sucesor.
CREATE TABLE camino_vinculo_migrado (
    evento_id INTEGER NOT NULL REFERENCES recuperacion_vigente(evento_id),
    sucesor_hash TEXT NOT NULL,
    sucesor_derivacion_id INTEGER NOT NULL REFERENCES secuencia_minima(id),
    registrado_en INTEGER NOT NULL,
    PRIMARY KEY(evento_id,sucesor_hash)
);
CREATE TRIGGER camino_vinculo_validar BEFORE INSERT ON camino_vinculo_migrado
WHEN NOT EXISTS(SELECT 1 FROM recuperacion_vigente v
    JOIN camino_derivacion viejo ON viejo.tipo_clave=v.tipo_clave AND viejo.derivacion_id=v.derivacion_id
    JOIN camino_derivacion nuevo ON nuevo.tipo_clave=v.tipo_clave AND nuevo.derivacion_id=NEW.sucesor_derivacion_id
    WHERE v.evento_id=NEW.evento_id AND nuevo.hash_minima=NEW.sucesor_hash AND viejo.hash_minima<>NEW.sucesor_hash)
BEGIN SELECT RAISE(ABORT,'vinculo sin entrega o sucesor ajeno al tipo'); END;

-- Historial de estados. Sin fila, el camino es candidato. Archivar agrega una fila;
-- volver a vigente por evidencia nueva agrega otra. Nunca se borra.
CREATE TABLE camino_estado (
    id INTEGER PRIMARY KEY,
    tipo_clave TEXT NOT NULL REFERENCES tipo_objetivo(clave),
    hash_minima TEXT NOT NULL,
    estado TEXT NOT NULL CHECK(estado IN ('vigente','archivado','candidato')),
    sucesor_hash TEXT,
    regla_version TEXT NOT NULL CHECK(length(trim(regla_version))>0),
    evidencia_json TEXT NOT NULL CHECK(json_valid(evidencia_json)),
    registrado_en INTEGER NOT NULL,
    CHECK((estado='archivado')=(sucesor_hash IS NOT NULL)),
    CHECK(sucesor_hash<>hash_minima)
);
CREATE INDEX camino_estado_buscar ON camino_estado(tipo_clave,hash_minima,id);
CREATE VIEW camino_estado_actual AS
SELECT * FROM camino_estado e
WHERE id=(SELECT max(id) FROM camino_estado WHERE tipo_clave=e.tipo_clave AND hash_minima=e.hash_minima);

CREATE TRIGGER camino_estado_validar BEFORE INSERT ON camino_estado
WHEN NOT EXISTS(SELECT 1 FROM camino_derivacion WHERE tipo_clave=NEW.tipo_clave AND hash_minima=NEW.hash_minima)
 OR (NEW.sucesor_hash IS NOT NULL AND NOT EXISTS(SELECT 1 FROM camino_derivacion
    WHERE tipo_clave=NEW.tipo_clave AND hash_minima=NEW.sucesor_hash))
BEGIN SELECT RAISE(ABORT,'camino o sucesor ajeno al tipo'); END;
CREATE TRIGGER camino_estado_archivar BEFORE INSERT ON camino_estado
WHEN NEW.estado='archivado' AND EXISTS(SELECT 1 FROM recuperacion_vigente v
    JOIN camino_derivacion d ON d.tipo_clave=v.tipo_clave AND d.derivacion_id=v.derivacion_id
    WHERE v.tipo_clave=NEW.tipo_clave AND d.hash_minima=NEW.hash_minima
      AND NOT EXISTS(SELECT 1 FROM camino_vinculo_migrado g
        WHERE g.evento_id=v.evento_id AND g.sucesor_hash=NEW.sucesor_hash))
BEGIN SELECT RAISE(ABORT,'migrar los vinculos antes de archivar'); END;

CREATE TRIGGER camino_vinculo_update BEFORE UPDATE ON camino_vinculo_migrado
BEGIN SELECT RAISE(ABORT,'vinculo migrado inmutable'); END;
CREATE TRIGGER camino_vinculo_delete BEFORE DELETE ON camino_vinculo_migrado
BEGIN SELECT RAISE(ABORT,'vinculo migrado inmutable'); END;
CREATE TRIGGER camino_estado_update BEFORE UPDATE ON camino_estado
BEGIN SELECT RAISE(ABORT,'historial de caminos append-only'); END;
CREATE TRIGGER camino_estado_delete BEFORE DELETE ON camino_estado
BEGIN SELECT RAISE(ABORT,'historial de caminos append-only'); END;

-- Un INSERT OR REPLACE no dispara los triggers de DELETE: se frena antes de insertar.
CREATE TRIGGER camino_estado_sin_reemplazo BEFORE INSERT ON camino_estado
WHEN EXISTS(SELECT 1 FROM camino_estado WHERE id=NEW.id)
BEGIN SELECT RAISE(ABORT,'historial de caminos append-only'); END;
CREATE TRIGGER camino_vinculo_sin_reemplazo BEFORE INSERT ON camino_vinculo_migrado
WHEN EXISTS(SELECT 1 FROM camino_vinculo_migrado WHERE evento_id=NEW.evento_id AND sucesor_hash=NEW.sucesor_hash)
BEGIN SELECT RAISE(IGNORE); END;
