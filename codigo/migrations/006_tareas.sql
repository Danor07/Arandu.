-- Tanda 5: tareas explícitas con criterios versionados y resultados externos.
-- Una tarea agrupa observaciones de varios turnos y chats; no reemplaza la
-- pertenencia de una instancia a una `ejecucion` ni toca `grupo_evidencia`.
-- Las migraciones 001-005 permanecen sin cambios.

CREATE TABLE tarea (
    id INTEGER PRIMARY KEY,
    clave TEXT NOT NULL UNIQUE CHECK (length(trim(clave)) > 0),
    proyecto TEXT NOT NULL CHECK (length(trim(proyecto)) > 0),
    creada_en INTEGER NOT NULL,
    contrato_json TEXT NOT NULL CHECK (json_valid(contrato_json))
);
CREATE INDEX tarea_proyecto ON tarea(proyecto, id);

-- Historial append-only de la tarea. `definicion` guarda una instantánea
-- completa de descripción y criterios; el resto referencia la versión vigente.
CREATE TABLE tarea_evento (
    id INTEGER PRIMARY KEY,
    clave TEXT NOT NULL UNIQUE CHECK (length(trim(clave)) > 0),
    tarea_id INTEGER NOT NULL REFERENCES tarea(id),
    tipo TEXT NOT NULL CHECK (tipo IN
        ('definicion', 'inicio', 'pausa', 'reanudacion', 'cierre', 'ajuste', 'cambio_requisitos')),
    definicion_version INTEGER NOT NULL CHECK (definicion_version > 0),
    descripcion TEXT,
    -- Arreglo de {id, version, descripcion, obligatorio}; solo en `definicion`.
    criterios_json TEXT CHECK (criterios_json IS NULL OR json_valid(criterios_json)),
    detalle_json TEXT NOT NULL CHECK (json_valid(detalle_json)),
    -- Los marcadores explícitos declaran su fuente; nadie los infiere.
    fuente TEXT CHECK (fuente IS NULL OR fuente = 'usuario_explicito'),
    registrado_en INTEGER NOT NULL,
    contrato_json TEXT NOT NULL CHECK (json_valid(contrato_json)),
    CHECK ((tipo = 'definicion' AND descripcion IS NOT NULL AND criterios_json IS NOT NULL)
        OR (tipo <> 'definicion' AND descripcion IS NULL AND criterios_json IS NULL))
);
CREATE UNIQUE INDEX tarea_definicion_unica ON tarea_evento(tarea_id, definicion_version)
    WHERE tipo = 'definicion';
CREATE INDEX tarea_evento_orden ON tarea_evento(tarea_id, id);

-- Asociación de una instancia ya capturada a una tarea. Significa «capturado
-- durante esta tarea», nunca causalidad. Una instancia pertenece a lo sumo a
-- una tarea; sin fila, queda fuera de los totales de la tarea.
CREATE TABLE tarea_vinculo (
    instancia_id INTEGER PRIMARY KEY REFERENCES instancia(id),
    tarea_id INTEGER NOT NULL REFERENCES tarea(id),
    definicion_version INTEGER NOT NULL CHECK (definicion_version > 0),
    sesion_clave TEXT,
    intervalo_id INTEGER,
    revision_seleccion INTEGER,
    metodo TEXT NOT NULL CHECK (metodo IN ('seleccion_explicita', 'enlace_manual')),
    evidencia TEXT NOT NULL CHECK (length(trim(evidencia)) > 0),
    vinculado_en INTEGER NOT NULL,
    contrato_json TEXT NOT NULL CHECK (json_valid(contrato_json)),
    -- La selección congelada trae su sesión; un enlace manual trae su motivo.
    CHECK ((metodo = 'seleccion_explicita' AND sesion_clave IS NOT NULL
            AND intervalo_id IS NOT NULL AND revision_seleccion IS NOT NULL)
        OR metodo = 'enlace_manual')
);
CREATE INDEX tarea_vinculo_tarea ON tarea_vinculo(tarea_id, instancia_id);

-- Resultado externo por criterio. Reutiliza los contratos de tanda 3: puede
-- referenciar una `evaluacion_ejecucion` existente en lugar de votar otra vez.
CREATE TABLE tarea_resultado (
    id INTEGER PRIMARY KEY,
    clave TEXT NOT NULL UNIQUE CHECK (length(trim(clave)) > 0),
    tarea_id INTEGER NOT NULL REFERENCES tarea(id),
    definicion_version INTEGER NOT NULL CHECK (definicion_version > 0),
    criterio_id TEXT NOT NULL CHECK (length(trim(criterio_id)) > 0),
    criterio_version TEXT NOT NULL CHECK (length(trim(criterio_version)) > 0),
    fuente_instancia_id INTEGER NOT NULL UNIQUE REFERENCES instancia(id),
    objetivo_instancia_id INTEGER REFERENCES instancia(id),
    alcance TEXT NOT NULL CHECK (alcance IN ('tarea', 'evento')),
    fuente TEXT NOT NULL CHECK (fuente IN ('usuario_explicito', 'verificador_externo')),
    resultado INTEGER CHECK (resultado IN (0, 1)),
    detalle_json TEXT NOT NULL CHECK (json_valid(detalle_json)),
    -- Misma observación que tanda 3, no un voto adicional.
    evaluacion_ejecucion_id INTEGER REFERENCES evaluacion_ejecucion(id),
    anterior_id INTEGER UNIQUE REFERENCES tarea_resultado(id),
    registrado_en INTEGER NOT NULL,
    contrato_json TEXT NOT NULL CHECK (json_valid(contrato_json)),
    CHECK ((alcance = 'evento' AND objetivo_instancia_id IS NOT NULL)
        OR (alcance = 'tarea' AND objetivo_instancia_id IS NULL))
);
CREATE INDEX tarea_resultado_criterio
    ON tarea_resultado(tarea_id, criterio_id, criterio_version, id);
CREATE UNIQUE INDEX tarea_resultado_evaluacion
    ON tarea_resultado(evaluacion_ejecucion_id) WHERE evaluacion_ejecucion_id IS NOT NULL;

CREATE TRIGGER tarea_update BEFORE UPDATE ON tarea
BEGIN SELECT RAISE(ABORT, 'historial de tarea inmutable'); END;
CREATE TRIGGER tarea_delete BEFORE DELETE ON tarea
BEGIN SELECT RAISE(ABORT, 'historial de tarea inmutable'); END;
CREATE TRIGGER tarea_evento_update BEFORE UPDATE ON tarea_evento
BEGIN SELECT RAISE(ABORT, 'historial de tarea append-only'); END;
CREATE TRIGGER tarea_evento_delete BEFORE DELETE ON tarea_evento
BEGIN SELECT RAISE(ABORT, 'historial de tarea append-only'); END;
CREATE TRIGGER tarea_vinculo_update BEFORE UPDATE ON tarea_vinculo
BEGIN SELECT RAISE(ABORT, 'vinculo de tarea inmutable'); END;
CREATE TRIGGER tarea_vinculo_delete BEFORE DELETE ON tarea_vinculo
BEGIN SELECT RAISE(ABORT, 'vinculo de tarea inmutable'); END;
CREATE TRIGGER tarea_resultado_update BEFORE UPDATE ON tarea_resultado
BEGIN SELECT RAISE(ABORT, 'resultado de tarea append-only'); END;
CREATE TRIGGER tarea_resultado_delete BEFORE DELETE ON tarea_resultado
BEGIN SELECT RAISE(ABORT, 'resultado de tarea append-only'); END;

-- Un evento no referencia una definición que todavía no existe.
CREATE TRIGGER tarea_evento_version BEFORE INSERT ON tarea_evento
WHEN NEW.tipo <> 'definicion' AND NOT EXISTS (
    SELECT 1 FROM tarea_evento d WHERE d.tarea_id = NEW.tarea_id
      AND d.tipo = 'definicion' AND d.definicion_version = NEW.definicion_version)
BEGIN SELECT RAISE(ABORT, 'definicion de tarea inexistente'); END;
CREATE TRIGGER tarea_vinculo_version BEFORE INSERT ON tarea_vinculo
WHEN NOT EXISTS (
    SELECT 1 FROM tarea_evento d WHERE d.tarea_id = NEW.tarea_id
      AND d.tipo = 'definicion' AND d.definicion_version = NEW.definicion_version)
BEGIN SELECT RAISE(ABORT, 'definicion de tarea inexistente'); END;
CREATE TRIGGER tarea_resultado_version BEFORE INSERT ON tarea_resultado
WHEN NOT EXISTS (
    SELECT 1 FROM tarea_evento d WHERE d.tarea_id = NEW.tarea_id
      AND d.tipo = 'definicion' AND d.definicion_version = NEW.definicion_version)
BEGIN SELECT RAISE(ABORT, 'definicion de tarea inexistente'); END;
