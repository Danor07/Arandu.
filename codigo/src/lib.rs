//! Captura sin selección semántica y cola local con entrega al menos una vez.
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub mod adapter;
pub mod canales;
pub mod competencia;
pub mod control;
pub mod cortes;
pub mod episodios;
pub mod episodios_automaticos;
pub mod execution;
pub mod measure;
pub mod minimas;
pub mod plataformas;
pub mod proyectos;
pub mod retrieval;
pub mod secuencia;
pub mod soluciones;
pub mod szz;
pub mod tasks;
pub mod urna;
pub mod vigentes;

const CONFIG: &str = r#"{"etapa":"tecnico-v1","algoritmo":"sha256-y-longitudes-utf8-v1","max_intentos":5,"reintento_base_us":100000,"reintento_max_us":60000000}"#;
const STAGE: &str = "tecnico-v1";
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/001_erd.sql")),
    (2, include_str!("../migrations/002_captura_tecnica.sql")),
    (
        3,
        include_str!("../migrations/003_recuperacion_textual.sql"),
    ),
    (
        4,
        include_str!("../migrations/004_ejecucion_evaluacion.sql"),
    ),
    (
        5,
        include_str!("../migrations/005_entrega_contexto_host.sql"),
    ),
    (6, include_str!("../migrations/006_tareas.sql")),
    (7, include_str!("../migrations/007_episodios.sql")),
    (8, include_str!("../migrations/008_cortes_automaticos.sql")),
    (9, include_str!("../migrations/009_secuencias_minimas.sql")),
    (
        10,
        include_str!("../migrations/010_objetivos_y_vigente.sql"),
    ),
    (11, include_str!("../migrations/011_arandu_secuencias.sql")),
    (
        12,
        include_str!("../migrations/012_competencia_y_archivo.sql"),
    ),
    (13, include_str!("../migrations/013_szz.sql")),
];

// Filas de las dos consultas de trayectoria; no son modelos de aprendizaje.
type UsageRow = (String, String, String, Option<i64>, Option<i64>);
type JobRow = (
    i64,
    String,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
);

fn now() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_micros()
        .try_into()?)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn migration_digest(sql: &str) -> String {
    // Un checkout con CRLF no debe invalidar una migración aplicada desde LF.
    digest(sql.replace("\r\n", "\n").as_bytes())
}

fn object() -> Value {
    json!({})
}

#[derive(Debug, Deserialize)]
pub struct Event {
    pub version: u32,
    pub clave_evento: String,
    pub grupo_evidencia: String,
    pub secuencia: i64,
    pub origen: String,
    pub rol: String,
    pub timestamp: i64,
    pub contenido: String,
    #[serde(default = "object")]
    pub metadatos: Value,
    pub uso: Option<Usage>,
    #[serde(default)]
    pub referencias: Vec<Reference>,
}

#[derive(Debug, Deserialize)]
pub struct Reference {
    pub tipo: String,
    pub clave_evento: String,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub intento_clave: String,
    pub tipo: String,
    #[serde(default = "object")]
    pub metricas: Value,
}

impl Event {
    fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "version JSONL no soportada");
        ensure!(
            !self.clave_evento.trim().is_empty() && !self.grupo_evidencia.trim().is_empty(),
            "claves vacias"
        );
        ensure!(self.secuencia >= 0, "secuencia debe ser no negativa");
        ensure!(
            ["usuario", "herramienta", "modelo", "replay"].contains(&self.origen.as_str()),
            "origen invalido"
        );
        ensure!(
            ["problema", "solucion", "correccion", "otro"].contains(&self.rol.as_str()),
            "rol invalido"
        );
        ensure!(self.metadatos.is_object(), "metadatos debe ser un objeto");
        for r in &self.referencias {
            ensure!(
                !r.tipo.trim().is_empty() && !r.clave_evento.trim().is_empty(),
                "referencia vacia"
            );
        }
        if let Some(u) = &self.uso {
            ensure!(!u.intento_clave.trim().is_empty(), "intento_clave vacia");
            ensure!(
                ["inicio", "resultado", "ajuste", "prueba", "interrupcion"]
                    .contains(&u.tipo.as_str()),
                "tipo de uso invalido"
            );
            ensure!(u.metricas.is_object(), "metricas debe ser un objeto");
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct Receipt {
    pub instancia_id: i64,
    pub clave_evento: String,
    pub insertado: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct Lease {
    pub trabajo_id: i64,
    pub instancia_id: i64,
    pub token: String,
    pub reserva_hasta: i64,
}

pub struct Store {
    conn: Connection,
    config_id: i64,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY, sha256 TEXT NOT NULL, aplicado_en INTEGER NOT NULL);")?;
        let versions: Vec<(i64, String)> = {
            let mut s =
                tx.prepare("SELECT version,sha256 FROM schema_migrations ORDER BY version")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        ensure!(
            versions.len() <= MIGRATIONS.len(),
            "base de una version mas nueva"
        );
        for (index, (version, hash)) in versions.iter().enumerate() {
            ensure!(
                *version == MIGRATIONS[index].0 && *hash == migration_digest(MIGRATIONS[index].1),
                "migracion alterada o historial discontinuo: {version}"
            );
        }
        for (version, sql) in MIGRATIONS.iter().skip(versions.len()) {
            tx.execute_batch(sql)
                .with_context(|| format!("migracion {version}"))?;
            tx.execute(
                "INSERT INTO schema_migrations VALUES (?1,?2,?3)",
                params![version, migration_digest(sql), now()?],
            )?;
        }
        // No se ocupa un ID fijo que pueda colisionar con otra configuración.
        let config_id = match tx
            .query_row(
                "SELECT id FROM configuracion WHERE parametros_json=?1 ORDER BY id LIMIT 1",
                [CONFIG],
                |r| r.get(0),
            )
            .optional()?
        {
            Some(id) => id,
            None => {
                tx.execute(
                    "INSERT INTO configuracion(timestamp,parametros_json) VALUES (?1,?2)",
                    params![now()?, CONFIG],
                )?;
                tx.last_insert_rowid()
            }
        };
        tx.commit()?;
        Ok(Self { conn, config_id })
    }

    /// Una confirmación se devuelve exclusivamente después del commit.
    /// La línea exacta se conserva; igualdad idempotente compara sus bytes UTF-8.
    pub fn capture(&mut self, raw: &str) -> Result<Receipt> {
        let value: Value = serde_json::from_str(raw).context("JSON invalido")?;
        let event: Event = serde_json::from_str(raw).context("sobre JSONL invalido")?;
        event.validate()?;
        ensure!(value.is_object(), "sobre JSONL debe ser un objeto");
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(i64, String)> = tx.query_row(
            "SELECT i.id,c.json_original FROM instancia i JOIN captura_evento c ON c.instancia_id=i.id WHERE i.clave_evento=?1",
            [&event.clave_evento], |r| Ok((r.get(0)?,r.get(1)?))
        ).optional()?;
        if let Some((id, previous)) = existing {
            ensure!(
                previous == raw,
                "conflicto de clave_evento: {}",
                event.clave_evento
            );
            tx.commit()?;
            return Ok(Receipt {
                instancia_id: id,
                clave_evento: event.clave_evento,
                insertado: false,
            });
        }
        let occupied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM captura_evento WHERE grupo_evidencia=?1 AND secuencia=?2)",
            params![event.grupo_evidencia, event.secuencia],
            |r| r.get(0),
        )?;
        ensure!(
            !occupied,
            "conflicto de secuencia en episodio: {} / {}",
            event.grupo_evidencia,
            event.secuencia
        );
        tx.execute("INSERT INTO instancia(clave_evento,grupo_evidencia,origen,rol,contenido,metadatos_json,timestamp) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![event.clave_evento,event.grupo_evidencia,event.origen,event.rol,event.contenido,serde_json::to_string(&event.metadatos)?,event.timestamp])?;
        let id = tx.last_insert_rowid();
        let received = now()?;
        tx.execute(
            "INSERT INTO observado_en(instancia_id,timestamp) VALUES (?1,?2)",
            params![id, received],
        )?;
        tx.execute("INSERT INTO trabajo_pendiente(instancia_id,configuracion_id,etapa,disponible_desde) VALUES (?1,?2,?3,?4)",params![id,self.config_id,STAGE,received])?;
        tx.execute(
            "INSERT INTO captura_evento VALUES (?1,?2,?3,?4,?5)",
            params![id, event.grupo_evidencia, event.secuencia, raw, received],
        )?;
        if let Some(u) = event.uso {
            tx.execute("INSERT INTO evento_uso(instancia_id,intento_clave,tipo,metricas_json,timestamp) VALUES (?1,?2,?3,?4,?5)",params![id,u.intento_clave,u.tipo,serde_json::to_string(&u.metricas)?,event.timestamp])?;
        }
        tx.commit()?;
        Ok(Receipt {
            instancia_id: id,
            clave_evento: event.clave_evento,
            insertado: true,
        })
    }

    /// ID de la memoria a la que pertenece este archivo, si ya fue sellado.
    pub fn identity(&self) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT memoria_id FROM memoria_identidad", [], |r| r.get(0))
            .optional()?)
    }

    /// Sella el archivo con su ID. Repetirlo con el mismo ID es idempotente;
    /// con otro ID se rechaza, para que un registro no apunte a otro contenido.
    pub fn stamp_identity(&mut self, id: &str, name: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row("SELECT memoria_id FROM memoria_identidad", [], |r| r.get(0))
            .optional()?;
        match existing {
            Some(previous) => ensure!(previous == id, "el archivo ya pertenece a otra memoria"),
            None => {
                tx.execute(
                    "INSERT INTO memoria_identidad VALUES (1,?1,?2,?3)",
                    params![id, name, now()?],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Copia consistente con `VACUUM INTO`: no copia el archivo de una base WAL
    /// activa y no modifica el original.
    pub fn vacuum_into(&self, destino: &Path) -> Result<()> {
        let texto = destino
            .to_str()
            .context("ruta de snapshot no representable en UTF-8")?;
        self.conn.execute("VACUUM INTO ?1", [texto])?;
        Ok(())
    }

    pub fn reserve(&mut self, duration: Duration) -> Result<Option<Lease>> {
        let micros: i64 = duration.as_micros().try_into()?;
        ensure!(micros > 0, "reserva debe durar al menos un microsegundo");
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let time = now()?;
        let until = time
            .checked_add(micros)
            .context("duracion de reserva desbordada")?;
        let lease = tx.query_row(
            "UPDATE trabajo_pendiente SET estado='en_proceso',intentos=intentos+1,reserva_token=lower(hex(randomblob(32))),reserva_hasta=?1
             WHERE id=(SELECT id FROM trabajo_pendiente WHERE etapa=?2 AND configuracion_id=?3 AND
               ((estado='pendiente' AND disponible_desde<=?4) OR (estado='en_proceso' AND reserva_hasta<=?4)) ORDER BY id LIMIT 1)
             RETURNING id,instancia_id,reserva_token,reserva_hasta",
            params![until,STAGE,self.config_id,time], |r| Ok(Lease { trabajo_id:r.get(0)?,instancia_id:r.get(1)?,token:r.get(2)?,reserva_hasta:r.get(3)? })
        ).optional()?;
        if let Some(l) = &lease {
            log_job(
                &tx,
                l.trabajo_id,
                "reserva",
                Some(&l.token),
                json!({"reserva_hasta":until}),
                time,
            )?;
        }
        tx.commit()?;
        Ok(lease)
    }

    /// Operación técnica lineal en el tamaño del contenido, sin juicio ni aprendizaje.
    /// El cálculo ocurre sin mantener una transacción escritora.
    pub fn calculate(&self, lease: &Lease) -> Result<Value> {
        let content: String = self.conn.query_row("SELECT i.contenido FROM instancia i JOIN trabajo_pendiente t ON t.instancia_id=i.id WHERE t.id=?1 AND i.id=?2 AND t.etapa=?3",params![lease.trabajo_id,lease.instancia_id,STAGE],|r|r.get(0))?;
        Ok(
            json!({"procesamiento":"tecnico-v1","bytes_utf8":content.len(),"caracteres_unicode":content.chars().count(),"sha256_contenido":digest(content.as_bytes())}),
        )
    }

    /// true: finalización nueva; false: confirmación ya aplicada por ese token.
    pub fn finish(&mut self, lease: &Lease, result: &Value) -> Result<bool> {
        let serialized = serde_json::to_string(result)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT reserva_token,resultado_json FROM resultado_tecnico WHERE trabajo_id=?1",
                [lease.trabajo_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((token, previous)) = existing {
            ensure!(token == lease.token, "reserva obsoleta");
            ensure!(previous == serialized, "conflicto de resultado derivado");
            tx.commit()?;
            return Ok(false);
        }
        let time = now()?;
        let changed = tx.execute("UPDATE trabajo_pendiente SET estado='completado',reserva_token=NULL,reserva_hasta=NULL WHERE id=?1 AND instancia_id=?2 AND estado='en_proceso' AND reserva_token=?3 AND reserva_hasta>?4 AND etapa=?5 AND configuracion_id=?6",params![lease.trabajo_id,lease.instancia_id,lease.token,time,STAGE,self.config_id])?;
        ensure!(changed == 1, "reserva vencida u obsoleta");
        tx.execute(
            "INSERT INTO resultado_tecnico VALUES (?1,?2,?3,?4,?5)",
            params![
                lease.trabajo_id,
                format!("tecnico-v1/{}", lease.trabajo_id),
                lease.token,
                serialized,
                time
            ],
        )?;
        log_job(
            &tx,
            lease.trabajo_id,
            "completado",
            Some(&lease.token),
            json!({}),
            time,
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Error del procesador, nunca resultado negativo de la tarea del usuario.
    pub fn fail(&mut self, lease: &Lease, error: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let time = now()?;
        let attempts: Option<i64> = tx.query_row("SELECT intentos FROM trabajo_pendiente WHERE id=?1 AND estado='en_proceso' AND reserva_token=?2 AND reserva_hasta>?3 AND etapa=?4 AND configuracion_id=?5",params![lease.trabajo_id,lease.token,time,STAGE,self.config_id],|r|r.get(0)).optional()?;
        let attempts = attempts.context("reserva vencida u obsoleta")?;
        let terminal = attempts >= 5;
        let delay = (100_000_i64 * (1_i64 << (attempts - 1).clamp(0, 10))).min(60_000_000);
        let state = if terminal { "fallido" } else { "pendiente" };
        tx.execute("UPDATE trabajo_pendiente SET estado=?1,reserva_token=NULL,reserva_hasta=NULL,ultimo_error=?2,disponible_desde=?3 WHERE id=?4",params![state,error,time+delay,lease.trabajo_id])?;
        log_job(
            &tx,
            lease.trabajo_id,
            if terminal { "fallido" } else { "reintento" },
            Some(&lease.token),
            json!({"error":error,"disponible_desde":time+delay}),
            time,
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn retry(&mut self, id: i64) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let time = now()?;
        let changed = tx.execute("UPDATE trabajo_pendiente SET estado='pendiente',disponible_desde=?1 WHERE id=?2 AND estado='fallido' AND etapa=?3 AND configuracion_id=?4",params![time,id,STAGE,self.config_id])?;
        ensure!(changed == 1, "trabajo no encontrado o no fallido");
        log_job(&tx, id, "reanudado", None, json!({}), time)?;
        tx.commit()?;
        Ok(())
    }

    pub fn episode(&mut self, group: &str) -> Result<Value> {
        // Una lectura consistente aunque otro proceso avance la cola.
        let tx = self.conn.transaction()?;
        let rows: Vec<(i64, i64, String)> = {
            let mut stmt = tx.prepare("SELECT i.id,c.secuencia,c.json_original FROM captura_evento c JOIN instancia i ON i.id=c.instancia_id WHERE c.grupo_evidencia=?1 ORDER BY c.secuencia")?;
            let mapped = stmt.query_map([group], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            mapped.collect::<rusqlite::Result<_>>()?
        };
        if rows.is_empty() {
            bail!("episodio no encontrado: {group}");
        }
        let mut events = Vec::new();
        for (id, sequence, raw) in rows {
            let event: Value = serde_json::from_str(&raw)?;
            let usage: Option<UsageRow> = tx.query_row("SELECT intento_clave,tipo,metricas_json,evento_recuperacion_id,contenido_id FROM evento_uso WHERE instancia_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
            let usage = usage.map(|(attempt,kind,metrics,recovery,content)| -> Result<Value> { Ok(json!({"intento_clave":attempt,"tipo":kind,"metricas":serde_json::from_str::<Value>(&metrics)?,"evento_recuperacion_id":recovery,"contenido_id":content})) }).transpose()?;
            let jobs: Vec<JobRow> = {
                let mut stmt=tx.prepare("SELECT t.id,t.estado,t.intentos,t.reserva_hasta,t.ultimo_error,r.resultado_json FROM trabajo_pendiente t LEFT JOIN resultado_tecnico r ON r.trabajo_id=t.id WHERE t.instancia_id=?1 ORDER BY t.id")?;
                let mapped = stmt.query_map([id], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })?;
                mapped.collect::<rusqlite::Result<_>>()?
            };
            let mut job_values = Vec::new();
            for (job, state, attempts, until, error, result) in jobs {
                let history: Vec<(String, Option<String>, String, i64)> = {
                    let mut stmt=tx.prepare("SELECT tipo,reserva_token,detalle_json,timestamp FROM trabajo_historial WHERE trabajo_id=?1 ORDER BY id")?;
                    let mapped = stmt
                        .query_map([job], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                    mapped.collect::<rusqlite::Result<_>>()?
                };
                let history=history.into_iter().map(|(kind,token,details,time)|->Result<Value>{Ok(json!({"tipo":kind,"token":token,"detalle":serde_json::from_str::<Value>(&details)?,"timestamp":time}))}).collect::<Result<Vec<_>>>()?;
                job_values.push(json!({"id":job,"estado":state,"reservas_totales":attempts,"reserva_hasta":until,"ultimo_error":error,"resultado_tecnico":result.map(|s|serde_json::from_str::<Value>(&s)).transpose()?,"historial":history}));
            }
            let envelope: Event = serde_json::from_value(event.clone())?;
            let mut references = Vec::new();
            for r in envelope.referencias {
                let target: Option<i64> = tx
                    .query_row(
                        "SELECT id FROM instancia WHERE clave_evento=?1",
                        [&r.clave_evento],
                        |row| row.get(0),
                    )
                    .optional()?;
                references.push(json!({"tipo":r.tipo,"clave_evento":r.clave_evento,"instancia_id":target,"disponible":target.is_some()}));
            }
            events.push(json!({"instancia_id":id,"secuencia":sequence,"evento":event,"json_original":raw,"uso":usage,"referencias":references,"trabajos":job_values}));
        }
        tx.commit()?;
        Ok(json!({"grupo_evidencia":group,"eventos":events}))
    }
}

fn log_job(
    tx: &Transaction<'_>,
    id: i64,
    kind: &str,
    token: Option<&str>,
    detail: Value,
    time: i64,
) -> Result<()> {
    tx.execute("INSERT INTO trabajo_historial(trabajo_id,tipo,reserva_token,detalle_json,timestamp) VALUES (?1,?2,?3,?4,?5)",params![id,kind,token,serde_json::to_string(&detail)?,time])?;
    Ok(())
}
