-- SZZ: fallos tardios. Un arreglo senala, con git blame, el commit que introdujo las
-- lineas que corrige; si ese commit aparece en una captura, el episodio queda propuesto.
-- El vinculo es probable: solo una decision explicita lo confirma. Todo append-only.
CREATE TABLE szz_vinculo (
    id INTEGER PRIMARY KEY,
    proyecto TEXT NOT NULL CHECK(length(trim(proyecto))>0),
    arreglo_commit TEXT NOT NULL CHECK(length(arreglo_commit) IN (40,64)),
    senalado_commit TEXT NOT NULL CHECK(length(senalado_commit) IN (40,64) AND senalado_commit<>arreglo_commit),
    -- [{"archivo":..., "lineas":N}]: lo que blame atribuyo a ese commit.
    archivos_json TEXT NOT NULL CHECK(json_valid(archivos_json)),
    -- Captura del `git commit` cuya salida trae el hash.
    instancia_id INTEGER NOT NULL REFERENCES instancia(id),
    origen TEXT NOT NULL CHECK(origen IN ('manual_tarea','automatico_lista')),
    episodio_id INTEGER NOT NULL,
    registrado_en INTEGER NOT NULL,
    UNIQUE(proyecto,arreglo_commit,senalado_commit,origen,episodio_id)
);
CREATE TRIGGER szz_vinculo_validar BEFORE INSERT ON szz_vinculo
WHEN NOT EXISTS(SELECT 1 FROM objetivo_episodio_captura c JOIN instancia_proyecto p ON p.instancia_id=c.instancia_id
    WHERE c.origen=NEW.origen AND c.episodio_id=NEW.episodio_id AND c.instancia_id=NEW.instancia_id AND p.proyecto=NEW.proyecto)
BEGIN SELECT RAISE(ABORT,'la captura no es de ese episodio o proyecto'); END;
CREATE TABLE szz_decision (
    vinculo_id INTEGER PRIMARY KEY REFERENCES szz_vinculo(id),
    decision TEXT NOT NULL CHECK(decision IN ('confirmado','descartado')),
    registrado_en INTEGER NOT NULL
);
-- Un INSERT OR REPLACE no dispara los triggers de DELETE: se frena antes de insertar.
CREATE TRIGGER szz_vinculo_sin_reemplazo BEFORE INSERT ON szz_vinculo
WHEN EXISTS(SELECT 1 FROM szz_vinculo WHERE id=NEW.id)
BEGIN SELECT RAISE(ABORT,'vinculo szz inmutable'); END;
CREATE TRIGGER szz_decision_unica BEFORE INSERT ON szz_decision
WHEN EXISTS(SELECT 1 FROM szz_decision WHERE vinculo_id=NEW.vinculo_id)
BEGIN SELECT RAISE(ABORT,'el vinculo ya tiene decision'); END;
CREATE TRIGGER szz_vinculo_update BEFORE UPDATE ON szz_vinculo
BEGIN SELECT RAISE(ABORT,'vinculo szz inmutable'); END;
CREATE TRIGGER szz_vinculo_delete BEFORE DELETE ON szz_vinculo
BEGIN SELECT RAISE(ABORT,'vinculo szz inmutable'); END;
CREATE TRIGGER szz_decision_update BEFORE UPDATE ON szz_decision
BEGIN SELECT RAISE(ABORT,'decision szz inmutable'); END;
CREATE TRIGGER szz_decision_delete BEFORE DELETE ON szz_decision
BEGIN SELECT RAISE(ABORT,'decision szz inmutable'); END;
