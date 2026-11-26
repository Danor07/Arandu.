-- Tanda 5 en el registro de control: modo de operación explícito y selección de
-- tarea por sesión. Solo IDs, revisiones y estado operativo; ninguna
-- descripción, criterio o evidencia vive acá. La migración 001 no se modifica.

-- Tres modos visibles. `recuperacion` distingue solo registro de memoria activa
-- dentro del estado 'encendida'; el valor 1 para las filas existentes es
-- historicamente exacto, porque la tanda 4 siempre recuperaba al estar encendida.
ALTER TABLE sesion_memoria ADD COLUMN recuperacion INTEGER NOT NULL DEFAULT 1
    CHECK (recuperacion IN (0, 1));
-- Selección de tarea por sesión, con revisión propia: cambiarla invalida los
-- callbacks que ya habían congelado la selección anterior.
ALTER TABLE sesion_memoria ADD COLUMN tarea_clave TEXT;
ALTER TABLE sesion_memoria ADD COLUMN tarea_definicion_version INTEGER;
ALTER TABLE sesion_memoria ADD COLUMN tarea_revision INTEGER NOT NULL DEFAULT 0
    CHECK (tarea_revision >= 0);

-- El modo queda fijado en el intervalo: un cambio de modo cierra el intervalo y
-- abre otro, así una corrida comparativa puede comprobarse constante.
ALTER TABLE intervalo_captura ADD COLUMN modo TEXT NOT NULL DEFAULT 'memoria_activa'
    CHECK (modo IN ('solo_registro', 'memoria_activa'));

CREATE TRIGGER sesion_tarea_revision_monotona BEFORE UPDATE OF tarea_revision ON sesion_memoria
WHEN NEW.tarea_revision < OLD.tarea_revision
BEGIN SELECT RAISE(ABORT, 'la revision de tarea no retrocede'); END;

-- `control_historial` se recrea porque su CHECK enumera las acciones y hay tres
-- nuevas. Las filas anteriores se copian con su id y contenido intactos.
CREATE TABLE control_historial_nuevo (
    id INTEGER PRIMARY KEY,
    sesion_id INTEGER REFERENCES sesion_memoria(id),
    accion TEXT NOT NULL CHECK (accion IN
        ('memoria_creada', 'proyecto_registrado', 'predeterminado_fijado', 'predeterminado_quitado',
         'sesion_inicializada', 'memoria_encendida', 'memoria_apagada', 'callback_rechazado',
         'modo_cambiado', 'tarea_seleccionada', 'tarea_quitada')),
    detalle_json TEXT NOT NULL CHECK (json_valid(detalle_json)),
    registrado_en INTEGER NOT NULL
);
INSERT INTO control_historial_nuevo(id, sesion_id, accion, detalle_json, registrado_en)
    SELECT id, sesion_id, accion, detalle_json, registrado_en FROM control_historial;
DROP TRIGGER control_historial_update;
DROP TRIGGER control_historial_delete;
DROP TABLE control_historial;
ALTER TABLE control_historial_nuevo RENAME TO control_historial;
CREATE INDEX control_historial_sesion_v2 ON control_historial(sesion_id, id);
CREATE TRIGGER control_historial_update BEFORE UPDATE ON control_historial
BEGIN SELECT RAISE(ABORT, 'historial de control append-only'); END;
CREATE TRIGGER control_historial_delete BEFORE DELETE ON control_historial
BEGIN SELECT RAISE(ABORT, 'historial de control append-only'); END;
