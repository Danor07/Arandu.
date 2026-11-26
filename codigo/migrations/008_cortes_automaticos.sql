-- Los objetivos de lista no requieren una tarea manual de 006 ni alteran 007.
CREATE TABLE episodio_automatico (
    id INTEGER PRIMARY KEY,
    origen_objetivo TEXT NOT NULL CHECK (origen_objetivo = 'automatico_lista'),
    proyecto TEXT NOT NULL CHECK (length(trim(proyecto)) > 0),
    chat TEXT NOT NULL CHECK (length(trim(chat)) > 0),
    objetivo TEXT NOT NULL CHECK (length(trim(objetivo)) > 0),
    fuente TEXT NOT NULL CHECK (fuente IN ('TodoWrite','TaskUpdate')),
    inicio_instancia_id INTEGER NOT NULL UNIQUE REFERENCES captura_evento(instancia_id),
    fin_instancia_id INTEGER REFERENCES captura_evento(instancia_id),
    -- Evidencia de SessionEnd en el registro separado; no es un evento crudo.
    cierre_intervalo_id INTEGER,
    cierre_control_en INTEGER,
    motivo_fin TEXT NOT NULL CHECK (motivo_fin IN ('completado','siguiente','sesion_terminada')),
    ultima_comprobacion_id INTEGER REFERENCES captura_evento(instancia_id),
    detector_version TEXT NOT NULL CHECK (detector_version = 'cortes-lista-v1'),
    normalizador_version TEXT NOT NULL CHECK (normalizador_version = 'herramienta-destino-v1'),
    secuencia_sha256 TEXT NOT NULL CHECK (length(secuencia_sha256)=64 AND secuencia_sha256 NOT GLOB '*[^0-9a-f]*'),
    cantidad_pasos INTEGER NOT NULL CHECK (cantidad_pasos >= 0),
    creado_en INTEGER NOT NULL,
    CHECK ((fin_instancia_id IS NOT NULL AND cierre_intervalo_id IS NULL AND cierre_control_en IS NULL)
        OR (fin_instancia_id IS NULL AND cierre_intervalo_id IS NOT NULL AND cierre_control_en IS NOT NULL
            AND motivo_fin='sesion_terminada'))
);
CREATE INDEX episodio_automatico_chat ON episodio_automatico(proyecto,chat,id);

CREATE TABLE episodio_automatico_paso (
    episodio_id INTEGER NOT NULL REFERENCES episodio_automatico(id),
    orden INTEGER NOT NULL CHECK (orden > 0),
    instancia_id INTEGER NOT NULL UNIQUE REFERENCES captura_evento(instancia_id),
    herramienta TEXT NOT NULL CHECK (length(trim(herramienta)) > 0),
    tipo_destino TEXT NOT NULL CHECK (length(trim(tipo_destino)) > 0),
    PRIMARY KEY (episodio_id,orden),
    UNIQUE (episodio_id,instancia_id)
);
CREATE TABLE episodio_automatico_resultado (
    episodio_id INTEGER PRIMARY KEY REFERENCES episodio_automatico(id),
    comprobacion_instancia_id INTEGER NOT NULL,
    resultado INTEGER NOT NULL CHECK (resultado IN (0,1)),
    regla TEXT NOT NULL CHECK (regla='ultima-comprobacion-v1'),
    FOREIGN KEY (episodio_id,comprobacion_instancia_id)
        REFERENCES episodio_automatico_paso(episodio_id,instancia_id)
);
-- Un tramo cerrado sin comprobación también se sella, sin fabricar resultado.
CREATE TABLE episodio_automatico_sello (
    episodio_id INTEGER PRIMARY KEY REFERENCES episodio_automatico(id)
);

CREATE TRIGGER episodio_automatico_limites BEFORE INSERT ON episodio_automatico
WHEN NOT EXISTS (SELECT 1 FROM captura_evento c JOIN instancia i ON i.id=c.instancia_id
    WHERE c.instancia_id=NEW.inicio_instancia_id AND c.grupo_evidencia=NEW.chat
      AND json_extract(i.metadatos_json,'$.proyecto')=NEW.proyecto)
 OR (NEW.fin_instancia_id IS NOT NULL AND NOT EXISTS (
    SELECT 1 FROM captura_evento c JOIN instancia i ON i.id=c.instancia_id
    JOIN captura_evento a ON a.instancia_id=NEW.inicio_instancia_id
    WHERE c.instancia_id=NEW.fin_instancia_id AND c.grupo_evidencia=NEW.chat
      AND c.secuencia>a.secuencia AND json_extract(i.metadatos_json,'$.proyecto')=NEW.proyecto))
BEGIN SELECT RAISE(ABORT,'limites de episodio automatico incompatibles'); END;

CREATE TRIGGER episodio_automatico_paso_validar BEFORE INSERT ON episodio_automatico_paso
WHEN EXISTS (SELECT 1 FROM episodio_automatico_sello WHERE episodio_id=NEW.episodio_id)
 OR NOT EXISTS (SELECT 1 FROM episodio_automatico e
    JOIN captura_evento a ON a.instancia_id=e.inicio_instancia_id
    JOIN captura_evento c ON c.instancia_id=NEW.instancia_id JOIN instancia i ON i.id=c.instancia_id
    LEFT JOIN captura_evento f ON f.instancia_id=e.fin_instancia_id
    WHERE e.id=NEW.episodio_id AND c.grupo_evidencia=e.chat
      AND json_extract(i.metadatos_json,'$.proyecto')=e.proyecto AND i.origen='herramienta'
      AND c.secuencia>a.secuencia AND (f.instancia_id IS NULL OR c.secuencia<f.secuencia))
 OR NEW.orden <> (SELECT count(*)+1 FROM episodio_automatico_paso WHERE episodio_id=NEW.episodio_id)
 OR NEW.orden > (SELECT cantidad_pasos FROM episodio_automatico WHERE id=NEW.episodio_id)
BEGIN SELECT RAISE(ABORT,'paso automatico ajeno, fuera de orden o sellado'); END;

CREATE TRIGGER episodio_automatico_resultado_validar BEFORE INSERT ON episodio_automatico_resultado
WHEN EXISTS (SELECT 1 FROM episodio_automatico_sello WHERE episodio_id=NEW.episodio_id)
 OR NOT EXISTS (SELECT 1 FROM episodio_automatico WHERE id=NEW.episodio_id
                AND ultima_comprobacion_id=NEW.comprobacion_instancia_id)
BEGIN SELECT RAISE(ABORT,'comprobacion automatica incompatible o episodio sellado'); END;
CREATE TRIGGER episodio_automatico_sellar BEFORE INSERT ON episodio_automatico_sello
WHEN NOT EXISTS (SELECT 1 FROM episodio_automatico e WHERE e.id=NEW.episodio_id
    AND e.cantidad_pasos=(SELECT count(*) FROM episodio_automatico_paso WHERE episodio_id=e.id)
    AND (e.ultima_comprobacion_id IS NULL OR EXISTS (
        SELECT 1 FROM episodio_automatico_paso WHERE episodio_id=e.id AND instancia_id=e.ultima_comprobacion_id)))
BEGIN SELECT RAISE(ABORT,'episodio automatico incompleto'); END;

CREATE TRIGGER episodio_automatico_update BEFORE UPDATE ON episodio_automatico
BEGIN SELECT RAISE(ABORT,'episodio automatico inmutable'); END;
CREATE TRIGGER episodio_automatico_delete BEFORE DELETE ON episodio_automatico
BEGIN SELECT RAISE(ABORT,'episodio automatico inmutable'); END;
CREATE TRIGGER episodio_automatico_paso_update BEFORE UPDATE ON episodio_automatico_paso
BEGIN SELECT RAISE(ABORT,'paso automatico inmutable'); END;
CREATE TRIGGER episodio_automatico_paso_delete BEFORE DELETE ON episodio_automatico_paso
BEGIN SELECT RAISE(ABORT,'paso automatico inmutable'); END;
CREATE TRIGGER episodio_automatico_resultado_update BEFORE UPDATE ON episodio_automatico_resultado
BEGIN SELECT RAISE(ABORT,'resultado automatico inmutable'); END;
CREATE TRIGGER episodio_automatico_resultado_delete BEFORE DELETE ON episodio_automatico_resultado
BEGIN SELECT RAISE(ABORT,'resultado automatico inmutable'); END;
CREATE TRIGGER episodio_automatico_sello_update BEFORE UPDATE ON episodio_automatico_sello
BEGIN SELECT RAISE(ABORT,'sello automatico inmutable'); END;
CREATE TRIGGER episodio_automatico_sello_delete BEFORE DELETE ON episodio_automatico_sello
BEGIN SELECT RAISE(ABORT,'sello automatico inmutable'); END;
