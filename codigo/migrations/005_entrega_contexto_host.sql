-- Tanda 4: identidad del archivo de memoria y registro append-only de lo que el
-- adaptador ofreció al canal del host. No guarda solicitudes a un proveedor.
-- 001-004 permanecen sin cambios.

-- Una sola fila. Permite comprobar que un archivo corresponde al ID pedido por
-- el registro de control antes de capturar o entregar nada.
CREATE TABLE memoria_identidad (
    unica INTEGER PRIMARY KEY CHECK (unica = 1),
    memoria_id TEXT NOT NULL CHECK (memoria_id GLOB '[0-9a-f]*' AND length(memoria_id) = 32),
    nombre_inicial TEXT NOT NULL,
    creada_en INTEGER NOT NULL
);

-- Preparación local de un bloque ofrecido a un canal del host. `frontera` es
-- siempre 'preparada': lo observado después vive en el historial, nunca aquí.
CREATE TABLE entrega_contexto_host (
    id INTEGER PRIMARY KEY,
    clave TEXT NOT NULL UNIQUE CHECK (length(trim(clave)) > 0),
    paquete_id INTEGER NOT NULL REFERENCES paquete_recuperacion(evento_id),
    problema_instancia_id INTEGER REFERENCES instancia(id),
    ejecucion_id INTEGER REFERENCES ejecucion(id),
    sesion_clave TEXT NOT NULL CHECK (length(trim(sesion_clave)) > 0),
    intervalo_id INTEGER NOT NULL CHECK (intervalo_id > 0),
    revision_activacion INTEGER NOT NULL CHECK (revision_activacion >= 0),
    hook_evento TEXT NOT NULL CHECK (length(trim(hook_evento)) > 0),
    hook_clave_evento TEXT,
    bloque_texto TEXT NOT NULL,
    salida_serializada TEXT NOT NULL,
    bytes_salida INTEGER NOT NULL CHECK (bytes_salida = length(CAST(salida_serializada AS BLOB))),
    presupuesto_integracion_bytes INTEGER NOT NULL CHECK (presupuesto_integracion_bytes >= 0),
    frontera TEXT NOT NULL CHECK (frontera = 'preparada'),
    adaptador_version TEXT NOT NULL CHECK (length(trim(adaptador_version)) > 0),
    configuracion_json TEXT NOT NULL CHECK (json_valid(configuracion_json)),
    fronteras_observadas_json TEXT NOT NULL CHECK (json_valid(fronteras_observadas_json)),
    registrado_en INTEGER NOT NULL,
    CHECK (bytes_salida <= presupuesto_integracion_bytes)
);
CREATE INDEX entrega_contexto_paquete ON entrega_contexto_host(paquete_id);
CREATE INDEX entrega_contexto_sesion ON entrega_contexto_host(sesion_clave, intervalo_id, id);

-- Historial de la entrega. 'preparada' se confirma antes de escribir a stdout;
-- una caída posterior deja la historia en ese punto y el estado es incierto.
CREATE TABLE entrega_contexto_evento (
    id INTEGER PRIMARY KEY,
    entrega_id INTEGER NOT NULL REFERENCES entrega_contexto_host(id),
    tipo TEXT NOT NULL CHECK (tipo IN ('preparada', 'escritura_completada', 'escritura_incierta', 'error')),
    detalle_json TEXT NOT NULL CHECK (json_valid(detalle_json)),
    registrado_en INTEGER NOT NULL
);
CREATE INDEX entrega_contexto_historial ON entrega_contexto_evento(entrega_id, id);

CREATE TRIGGER memoria_identidad_update BEFORE UPDATE ON memoria_identidad
BEGIN SELECT RAISE(ABORT, 'identidad de memoria inmutable'); END;
CREATE TRIGGER memoria_identidad_delete BEFORE DELETE ON memoria_identidad
BEGIN SELECT RAISE(ABORT, 'identidad de memoria inmutable'); END;
CREATE TRIGGER entrega_contexto_host_update BEFORE UPDATE ON entrega_contexto_host
BEGIN SELECT RAISE(ABORT, 'entrega de contexto inmutable'); END;
CREATE TRIGGER entrega_contexto_host_delete BEFORE DELETE ON entrega_contexto_host
BEGIN SELECT RAISE(ABORT, 'entrega de contexto inmutable'); END;
CREATE TRIGGER entrega_contexto_evento_update BEFORE UPDATE ON entrega_contexto_evento
BEGIN SELECT RAISE(ABORT, 'historial de entrega append-only'); END;
CREATE TRIGGER entrega_contexto_evento_delete BEFORE DELETE ON entrega_contexto_evento
BEGIN SELECT RAISE(ABORT, 'historial de entrega append-only'); END;
