-- Proyeccion de fuentes selladas. No cambia episodios ni les agrega resultados.
CREATE VIEW objetivo_episodio_fuente AS
SELECT 'manual_tarea' AS origen,e.id AS episodio_id,t.proyecto,d.descripcion AS objetivo,
    r.resultado,tr.fuente_instancia_id AS evidencia_id,i.timestamp AS fecha,
    e.creada_en AS disponible_en,tr.id AS tarea_resultado_id,
    e.criterio_id,e.criterio_version,
    m.id AS derivacion_id,m.hash_minima,m.creada_en AS minima_disponible_en,tr.evaluacion_ejecucion_id
FROM episodio e JOIN tarea t ON t.id=e.tarea_id
JOIN tarea_evento d ON d.id=e.definicion_evento_id
JOIN episodio_resultado r ON r.episodio_id=e.id
JOIN tarea_resultado tr ON tr.id=r.tarea_resultado_id JOIN instancia i ON i.id=tr.fuente_instancia_id
LEFT JOIN (secuencia_minima m JOIN secuencia_minima_sello s ON s.derivacion_id=m.id)
    ON m.episodio_manual_id=e.id AND m.regla_version='minima-conservadora-v1'
UNION ALL
SELECT 'automatico_lista',e.id,e.proyecto,e.objetivo,r.resultado,r.comprobacion_instancia_id,
    coalesce(i.timestamp,e.creado_en),e.creado_en,NULL,NULL,NULL,
    m.id,m.hash_minima,m.creada_en,NULL
FROM episodio_automatico e JOIN episodio_automatico_sello s ON s.episodio_id=e.id
LEFT JOIN episodio_automatico_resultado r ON r.episodio_id=e.id
LEFT JOIN instancia i ON i.id=r.comprobacion_instancia_id
LEFT JOIN (secuencia_minima m JOIN secuencia_minima_sello ms ON ms.derivacion_id=m.id)
    ON m.episodio_automatico_id=e.id AND m.regla_version='minima-conservadora-v1';

CREATE VIEW objetivo_episodio_captura AS
SELECT 'manual_tarea' AS origen,episodio_id,instancia_id FROM episodio_paso
UNION SELECT 'manual_tarea',e.id,t.fuente_instancia_id FROM episodio e
    JOIN episodio_resultado r ON r.episodio_id=e.id JOIN tarea_resultado t ON t.id=r.tarea_resultado_id
UNION SELECT 'automatico_lista',episodio_id,instancia_id FROM episodio_automatico_paso
UNION SELECT 'automatico_lista',id,inicio_instancia_id FROM episodio_automatico
UNION SELECT 'automatico_lista',id,fin_instancia_id FROM episodio_automatico WHERE fin_instancia_id IS NOT NULL;

CREATE TABLE tipo_objetivo (
    clave TEXT PRIMARY KEY CHECK (length(clave)=64 AND clave NOT GLOB '*[^0-9a-f]*'),
    proyecto TEXT NOT NULL CHECK(length(trim(proyecto))>0),
    regla_version TEXT NOT NULL CHECK(regla_version='objetivo-terminos-v1'),
    terminos_json TEXT NOT NULL CHECK(json_valid(terminos_json) AND json_type(terminos_json)='array'
        AND json_array_length(terminos_json) BETWEEN 1 AND 64),
    UNIQUE(proyecto,regla_version,terminos_json)
);
CREATE TABLE episodio_tipo_objetivo (
    origen TEXT NOT NULL CHECK(origen IN ('manual_tarea','automatico_lista')),
    episodio_id INTEGER NOT NULL,
    tipo_clave TEXT NOT NULL REFERENCES tipo_objetivo(clave),
    PRIMARY KEY(origen,episodio_id)
);
CREATE INDEX episodio_tipo_buscar ON episodio_tipo_objetivo(tipo_clave,origen,episodio_id);
CREATE TRIGGER episodio_tipo_fuente BEFORE INSERT ON episodio_tipo_objetivo
WHEN NOT EXISTS (SELECT 1 FROM objetivo_episodio_fuente f JOIN tipo_objetivo t ON t.clave=NEW.tipo_clave
    WHERE f.origen=NEW.origen AND f.episodio_id=NEW.episodio_id AND f.proyecto=t.proyecto)
BEGIN SELECT RAISE(ABORT,'tipo de objetivo ajeno o episodio sin sellar'); END;

-- El bloque elegido y su evidencia se congelan con el paquete, no se recalculan al releer.
CREATE TABLE recuperacion_vigente (
    evento_id INTEGER PRIMARY KEY REFERENCES evento_recuperacion(id),
    tipo_clave TEXT NOT NULL REFERENCES tipo_objetivo(clave),
    derivacion_id INTEGER NOT NULL REFERENCES secuencia_minima(id),
    regla_version TEXT NOT NULL CHECK(regla_version='vigente-mayoria-reciente-v1'),
    texto_entregado TEXT NOT NULL CHECK(length(CAST(texto_entregado AS BLOB))>0),
    evidencia_json TEXT NOT NULL CHECK(json_valid(evidencia_json))
);
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

CREATE TRIGGER tipo_objetivo_update BEFORE UPDATE ON tipo_objetivo
BEGIN SELECT RAISE(ABORT,'tipo inmutable'); END;
CREATE TRIGGER tipo_objetivo_delete BEFORE DELETE ON tipo_objetivo
BEGIN SELECT RAISE(ABORT,'tipo inmutable'); END;
CREATE TRIGGER episodio_tipo_update BEFORE UPDATE ON episodio_tipo_objetivo
BEGIN SELECT RAISE(ABORT,'tipo de episodio inmutable'); END;
CREATE TRIGGER episodio_tipo_delete BEFORE DELETE ON episodio_tipo_objetivo
BEGIN SELECT RAISE(ABORT,'tipo de episodio inmutable'); END;
CREATE TRIGGER recuperacion_vigente_update BEFORE UPDATE ON recuperacion_vigente
BEGIN SELECT RAISE(ABORT,'vigente entregada inmutable'); END;
CREATE TRIGGER recuperacion_vigente_delete BEFORE DELETE ON recuperacion_vigente
BEGIN SELECT RAISE(ABORT,'vigente entregada inmutable'); END;
