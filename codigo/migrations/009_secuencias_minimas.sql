-- Lectura derivada versionada. Los episodios de 007 y 008 no se reescriben.
CREATE TABLE secuencia_minima (
    id INTEGER PRIMARY KEY,
    origen TEXT NOT NULL CHECK (origen IN ('manual_tarea','automatico_lista')),
    episodio_manual_id INTEGER REFERENCES episodio(id),
    episodio_automatico_id INTEGER REFERENCES episodio_automatico(id),
    regla_version TEXT NOT NULL CHECK (regla_version='minima-conservadora-v1'),
    normalizador_version TEXT NOT NULL CHECK (normalizador_version='herramienta-destino-v1'),
    llego INTEGER NOT NULL CHECK (llego IN (0,1)),
    hash_minima TEXT,
    cantidad_original INTEGER NOT NULL CHECK (cantidad_original>=0),
    cantidad_minima INTEGER NOT NULL CHECK (cantidad_minima>=0 AND cantidad_minima<=cantidad_original),
    creada_en INTEGER NOT NULL,
    CHECK ((origen='manual_tarea' AND episodio_manual_id IS NOT NULL AND episodio_automatico_id IS NULL)
        OR (origen='automatico_lista' AND episodio_manual_id IS NULL AND episodio_automatico_id IS NOT NULL)),
    CHECK ((llego=0 AND hash_minima IS NULL AND cantidad_minima=0)
        OR (llego=1 AND hash_minima IS NOT NULL AND length(hash_minima)=64 AND hash_minima NOT GLOB '*[^0-9a-f]*'))
);
CREATE UNIQUE INDEX secuencia_minima_manual ON secuencia_minima(episodio_manual_id,regla_version)
    WHERE episodio_manual_id IS NOT NULL;
CREATE UNIQUE INDEX secuencia_minima_automatica ON secuencia_minima(episodio_automatico_id,regla_version)
    WHERE episodio_automatico_id IS NOT NULL;

CREATE TABLE secuencia_minima_paso (
    derivacion_id INTEGER NOT NULL REFERENCES secuencia_minima(id),
    orden_original INTEGER NOT NULL CHECK (orden_original>0),
    instancia_id INTEGER NOT NULL REFERENCES captura_evento(instancia_id),
    en_minima INTEGER NOT NULL CHECK (en_minima IN (0,1)),
    motivo TEXT NOT NULL CHECK (motivo IN ('conservado_sin_evidencia','candidata_sin_reuso_observado',
        'reintento_equivalente_exitoso','comprobacion_fallida_superada','reescritura_completa_posterior',
        'consulta_fallida','episodio_sin_exito')),
    certeza TEXT NOT NULL CHECK (certeza IN ('alta','media','baja','no_atribuida')),
    apoyo_orden INTEGER,
    PRIMARY KEY (derivacion_id,orden_original),
    UNIQUE (derivacion_id,instancia_id),
    FOREIGN KEY (derivacion_id,apoyo_orden) REFERENCES secuencia_minima_paso(derivacion_id,orden_original)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (apoyo_orden IS NULL OR apoyo_orden>orden_original),
    CHECK ((en_minima=1 AND motivo='conservado_sin_evidencia' AND certeza='no_atribuida' AND apoyo_orden IS NULL)
        OR (en_minima=1 AND motivo='candidata_sin_reuso_observado' AND certeza='baja' AND apoyo_orden IS NULL)
        OR (en_minima=0 AND motivo='consulta_fallida' AND certeza='media' AND apoyo_orden IS NULL)
        OR (en_minima=0 AND motivo='episodio_sin_exito' AND certeza='alta' AND apoyo_orden IS NULL)
        OR (en_minima=0 AND motivo IN ('reintento_equivalente_exitoso','comprobacion_fallida_superada','reescritura_completa_posterior')
            AND certeza='alta' AND apoyo_orden IS NOT NULL))
);
CREATE TABLE secuencia_minima_sello (
    derivacion_id INTEGER PRIMARY KEY REFERENCES secuencia_minima(id)
);

CREATE TRIGGER secuencia_minima_fuente BEFORE INSERT ON secuencia_minima
WHEN NOT (
    (NEW.origen='manual_tarea' AND EXISTS (
        SELECT 1 FROM episodio e JOIN episodio_resultado r ON r.episodio_id=e.id
        WHERE e.id=NEW.episodio_manual_id AND e.cantidad_pasos=NEW.cantidad_original
          AND e.normalizador_version=NEW.normalizador_version AND (r.resultado=1)=NEW.llego))
    OR (NEW.origen='automatico_lista' AND EXISTS (
        SELECT 1 FROM episodio_automatico e JOIN episodio_automatico_sello s ON s.episodio_id=e.id
        LEFT JOIN episodio_automatico_resultado r ON r.episodio_id=e.id
        WHERE e.id=NEW.episodio_automatico_id AND e.cantidad_pasos=NEW.cantidad_original
          AND e.normalizador_version=NEW.normalizador_version AND coalesce(r.resultado=1,0)=NEW.llego))
)
BEGIN SELECT RAISE(ABORT,'fuente de secuencia minima incompatible o sin sellar'); END;

CREATE TRIGGER secuencia_minima_paso_validar BEFORE INSERT ON secuencia_minima_paso
WHEN EXISTS (SELECT 1 FROM secuencia_minima_sello WHERE derivacion_id=NEW.derivacion_id)
 OR NEW.orden_original<>(SELECT count(*)+1 FROM secuencia_minima_paso WHERE derivacion_id=NEW.derivacion_id)
 OR NOT EXISTS (
    SELECT 1 FROM secuencia_minima d WHERE d.id=NEW.derivacion_id
      AND ((d.llego=1 AND NEW.motivo<>'episodio_sin_exito')
        OR (d.llego=0 AND NEW.en_minima=0 AND NEW.motivo='episodio_sin_exito'))
      AND ((d.origen='manual_tarea' AND EXISTS (SELECT 1 FROM episodio_paso p
            WHERE p.episodio_id=d.episodio_manual_id AND p.orden=NEW.orden_original AND p.instancia_id=NEW.instancia_id))
        OR (d.origen='automatico_lista' AND EXISTS (SELECT 1 FROM episodio_automatico_paso p
            WHERE p.episodio_id=d.episodio_automatico_id AND p.orden=NEW.orden_original AND p.instancia_id=NEW.instancia_id)))
 )
BEGIN SELECT RAISE(ABORT,'paso derivado ajeno, fuera de orden o sellado'); END;

CREATE TRIGGER secuencia_minima_sellar BEFORE INSERT ON secuencia_minima_sello
WHEN NOT EXISTS (
    SELECT 1 FROM secuencia_minima d WHERE d.id=NEW.derivacion_id
      AND d.cantidad_original=(SELECT count(*) FROM secuencia_minima_paso WHERE derivacion_id=d.id)
      AND d.cantidad_minima=(SELECT count(*) FROM secuencia_minima_paso WHERE derivacion_id=d.id AND en_minima=1)
      AND NOT EXISTS (SELECT 1 FROM secuencia_minima_paso p WHERE p.derivacion_id=d.id AND p.apoyo_orden IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM secuencia_minima_paso apoyo
            WHERE apoyo.derivacion_id=d.id AND apoyo.orden_original=p.apoyo_orden AND apoyo.en_minima=1))
 )
BEGIN SELECT RAISE(ABORT,'particion incompleta o evidencia descartada'); END;

CREATE TRIGGER secuencia_minima_update BEFORE UPDATE ON secuencia_minima
BEGIN SELECT RAISE(ABORT,'derivacion inmutable'); END;
CREATE TRIGGER secuencia_minima_delete BEFORE DELETE ON secuencia_minima
BEGIN SELECT RAISE(ABORT,'derivacion inmutable'); END;
CREATE TRIGGER secuencia_minima_paso_update BEFORE UPDATE ON secuencia_minima_paso
BEGIN SELECT RAISE(ABORT,'decision de paso inmutable'); END;
CREATE TRIGGER secuencia_minima_paso_delete BEFORE DELETE ON secuencia_minima_paso
BEGIN SELECT RAISE(ABORT,'decision de paso inmutable'); END;
CREATE TRIGGER secuencia_minima_sello_update BEFORE UPDATE ON secuencia_minima_sello
BEGIN SELECT RAISE(ABORT,'sello de derivacion inmutable'); END;
CREATE TRIGGER secuencia_minima_sello_delete BEFORE DELETE ON secuencia_minima_sello
BEGIN SELECT RAISE(ABORT,'sello de derivacion inmutable'); END;
