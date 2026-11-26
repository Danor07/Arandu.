CREATE TABLE ejecucion (
 id INTEGER PRIMARY KEY, intento_clave TEXT NOT NULL UNIQUE CHECK(length(trim(intento_clave))>0),
 proyecto TEXT NOT NULL CHECK(length(trim(proyecto))>0), problema_instancia_id INTEGER NOT NULL REFERENCES instancia(id),
 grupo_evidencia TEXT NOT NULL, registrado_en INTEGER NOT NULL,
 contrato_json TEXT NOT NULL CHECK(json_valid(contrato_json))
);
CREATE TABLE ejecucion_evento (
 instancia_id INTEGER PRIMARY KEY REFERENCES instancia(id), ejecucion_id INTEGER NOT NULL REFERENCES ejecucion(id),
 enlazado_en INTEGER NOT NULL, contrato_json TEXT NOT NULL CHECK(json_valid(contrato_json)),
 UNIQUE(ejecucion_id,instancia_id)
);

CREATE TABLE exposicion_memoria (
 id INTEGER PRIMARY KEY, clave TEXT NOT NULL UNIQUE CHECK(length(trim(clave))>0), ejecucion_id INTEGER NOT NULL REFERENCES ejecucion(id),
 paquete_id INTEGER NOT NULL REFERENCES paquete_recuperacion(evento_id), solicitud_instancia_id INTEGER NOT NULL,
 llamada_id TEXT NOT NULL CHECK(length(trim(llamada_id))>0), posicion INTEGER NOT NULL CHECK(posicion>=0),
 texto_memoria TEXT NOT NULL CHECK(length(CAST(texto_memoria AS BLOB))>0),
 procedencia TEXT NOT NULL CHECK(procedencia IN ('real','simulada')),
 registrado_en INTEGER NOT NULL, contrato_json TEXT NOT NULL CHECK(json_valid(contrato_json)),
 UNIQUE(solicitud_instancia_id,posicion), UNIQUE(solicitud_instancia_id,paquete_id),
 FOREIGN KEY(ejecucion_id,solicitud_instancia_id) REFERENCES ejecucion_evento(ejecucion_id,instancia_id)
);
CREATE INDEX exposicion_intento ON exposicion_memoria(ejecucion_id,solicitud_instancia_id,posicion);
CREATE INDEX exposicion_paquete ON exposicion_memoria(paquete_id);
CREATE TABLE evaluacion_ejecucion (
 id INTEGER PRIMARY KEY, clave TEXT NOT NULL UNIQUE CHECK(length(trim(clave))>0), ejecucion_id INTEGER NOT NULL REFERENCES ejecucion(id),
 fuente_instancia_id INTEGER NOT NULL UNIQUE, objetivo_instancia_id INTEGER,
 criterio_clave TEXT NOT NULL CHECK(length(trim(criterio_clave))>0), criterio_version TEXT NOT NULL CHECK(length(trim(criterio_version))>0),
 alcance TEXT NOT NULL CHECK(alcance IN ('intento','evento')),
 fuente TEXT NOT NULL CHECK(fuente IN ('usuario_explicito','verificador_externo')),
 resultado INTEGER CHECK(resultado IN (0,1)), detalle_json TEXT NOT NULL CHECK(json_valid(detalle_json)), nota TEXT NOT NULL,
 procedencia TEXT NOT NULL CHECK(procedencia IN ('real','simulada')),
 anterior_id INTEGER UNIQUE REFERENCES evaluacion_ejecucion(id), registrado_en INTEGER NOT NULL,
 contrato_json TEXT NOT NULL CHECK(json_valid(contrato_json)),
 CHECK((alcance='evento' AND objetivo_instancia_id IS NOT NULL) OR (alcance='intento' AND objetivo_instancia_id IS NULL)),
 FOREIGN KEY(ejecucion_id,fuente_instancia_id) REFERENCES ejecucion_evento(ejecucion_id,instancia_id),
 FOREIGN KEY(ejecucion_id,objetivo_instancia_id) REFERENCES ejecucion_evento(ejecucion_id,instancia_id)
);
CREATE INDEX evaluacion_intento ON evaluacion_ejecucion(ejecucion_id);
CREATE TRIGGER ejecucion_update BEFORE UPDATE ON ejecucion BEGIN SELECT RAISE(ABORT,'historial de ejecucion inmutable'); END;
CREATE TRIGGER ejecucion_delete BEFORE DELETE ON ejecucion BEGIN SELECT RAISE(ABORT,'historial de ejecucion inmutable'); END;
CREATE TRIGGER ejecucion_evento_update BEFORE UPDATE ON ejecucion_evento BEGIN SELECT RAISE(ABORT,'historial de ejecucion inmutable'); END;
CREATE TRIGGER ejecucion_evento_delete BEFORE DELETE ON ejecucion_evento BEGIN SELECT RAISE(ABORT,'historial de ejecucion inmutable'); END;
CREATE TRIGGER exposicion_memoria_update BEFORE UPDATE ON exposicion_memoria BEGIN SELECT RAISE(ABORT,'historial de ejecucion inmutable'); END;
CREATE TRIGGER exposicion_memoria_delete BEFORE DELETE ON exposicion_memoria BEGIN SELECT RAISE(ABORT,'historial de ejecucion inmutable'); END;
CREATE TRIGGER evaluacion_ejecucion_update BEFORE UPDATE ON evaluacion_ejecucion BEGIN SELECT RAISE(ABORT,'historial de ejecucion inmutable'); END;
CREATE TRIGGER evaluacion_ejecucion_delete BEFORE DELETE ON evaluacion_ejecucion BEGIN SELECT RAISE(ABORT,'historial de ejecucion inmutable'); END;
