-- Los predeterminados anteriores siempre encendian memoria_activa.
-- El modo del proyecto solo se hereda al crear una sesion, no al reanudarla.
ALTER TABLE proyecto_local ADD COLUMN modo_predeterminado TEXT NOT NULL DEFAULT 'memoria_activa'
    CHECK (modo_predeterminado IN ('solo_registro', 'memoria_activa'));
