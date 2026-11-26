//! Episodios explícitos sobre tareas. Reglas del normalizador: README,
//! sección «Episodios y normalización v1». El crudo nunca se modifica.
use crate::execution::{nonempty, rows};
use crate::tasks::ventanas;
use crate::{digest, now, Store};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};

pub const NORMALIZADOR_VERSION: &str = "herramienta-destino-v1";

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct PasoNormalizado {
    pub herramienta: String,
    pub tipo_destino: String,
}

pub(crate) fn hash_secuencia(pasos: &[PasoNormalizado]) -> Result<String> {
    Ok(digest(serde_json::to_string(pasos)?.as_bytes()))
}

fn identificador(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

fn nombre(s: &str) -> &str {
    s.rsplit(['/', '\\']).next().unwrap_or("")
}

/// Total y sin E/S. Acepta el payload del hook o el sobre JSONL que lo conserva.
pub fn normalizar_herramienta(evento: &Value) -> PasoNormalizado {
    let payload = evento
        .pointer("/metadatos/payload_hook")
        .filter(|v| v.is_object())
        .unwrap_or(evento);
    let herramienta = payload["tool_name"].as_str().unwrap_or("");
    let entrada = &payload["tool_input"];
    let destino = match herramienta {
        "Read" | "Edit" | "Write" => entrada["file_path"].as_str().map(|ruta| {
            let archivo = nombre(ruta);
            match archivo.rsplit_once('.') {
                Some((base, extension)) if !base.is_empty() && identificador(extension) => {
                    format!("archivo:{}", extension.to_ascii_lowercase())
                }
                Some((base, extension)) if !base.is_empty() && !extension.is_empty() => {
                    "otro".into()
                }
                _ if !archivo.is_empty() => "archivo:sin_extension".into(),
                _ => "otro".into(),
            }
        }),
        "Bash" => entrada["command"]
            .as_str()
            .and_then(|c| c.split_whitespace().next())
            .filter(|palabra| {
                palabra
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-./\\:".contains(&b))
            })
            .map(|palabra| {
                let programa = nombre(palabra).to_ascii_lowercase();
                let programa = programa.strip_suffix(".exe").unwrap_or(&programa);
                if identificador(programa) {
                    format!("comando:{programa}")
                } else {
                    "otro".into()
                }
            }),
        "Grep" | "Glob" | "WebSearch" => Some("busqueda".into()),
        "WebFetch" => {
            entrada["url"]
                .as_str()
                .and_then(|u| u.split_once(':'))
                .map(|(esquema, _)| {
                    if esquema.starts_with(|c: char| c.is_ascii_alphabetic())
                        && esquema
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
                    {
                        format!("url:{}", esquema.to_ascii_lowercase())
                    } else {
                        "otro".into()
                    }
                })
        }
        "Task" => Some("subagente".into()),
        "MCP" => entrada["server"]
            .as_str()
            .filter(|s| identificador(s))
            .map(|s| format!("mcp:{s}")),
        _ => None,
    };
    if let Some(resto) = herramienta.strip_prefix("mcp__") {
        if let Some((servidor, operacion)) = resto.split_once("__") {
            if identificador(servidor) && !operacion.is_empty() {
                return PasoNormalizado {
                    herramienta: "MCP".into(),
                    tipo_destino: format!("mcp:{servidor}"),
                };
            }
        }
    }
    let conocida = matches!(
        herramienta,
        "Read"
            | "Edit"
            | "Write"
            | "Bash"
            | "Grep"
            | "Glob"
            | "WebSearch"
            | "WebFetch"
            | "Task"
            | "MCP"
    );
    PasoNormalizado {
        herramienta: if conocida { herramienta } else { "otro" }.into(),
        tipo_destino: destino.unwrap_or_else(|| "otro".into()),
    }
}

fn leer(c: &Connection, proyecto: &str, clave: &str) -> Result<Value> {
    let cabecera = rows(c,
        "SELECT e.*,t.clave AS tarea,t.proyecto,d.definicion_version,d.descripcion,d.criterios_json,
                cierre.clave AS cierre FROM episodio e JOIN tarea t ON t.id=e.tarea_id
         JOIN tarea_evento d ON d.id=e.definicion_evento_id JOIN tarea_evento cierre ON cierre.id=e.cierre_evento_id
         WHERE e.clave=?1 AND t.proyecto=?2", params![clave,proyecto])?.pop().context("episodio ausente en el proyecto")?;
    let id = cabecera["id"].as_i64().context("episodio sin id")?;
    let pasos = rows(c,
        "SELECT p.orden,p.instancia_id,p.herramienta,p.tipo_destino,i.clave_evento,c.grupo_evidencia,c.secuencia
         FROM episodio_paso p JOIN instancia i ON i.id=p.instancia_id JOIN captura_evento c ON c.instancia_id=i.id
         WHERE p.episodio_id=?1 ORDER BY p.orden", [id])?;
    let resultado = rows(
        c,
        "SELECT er.*,r.clave AS comprobacion,r.criterio_id,r.criterio_version,r.fuente_instancia_id
         FROM episodio_resultado er JOIN tarea_resultado r ON r.id=er.tarea_resultado_id
         WHERE er.episodio_id=?1",
        [id],
    )?
    .pop()
    .context("episodio sin sellar")?;
    Ok(
        json!({"origen_objetivo":"manual_tarea","episodio":cabecera,"pasos":pasos,"resultado":resultado,
        "orden":"grupo_evidencia BINARY, secuencia de fuente, instancia_id; sin cronologia global entre chats",
        "costo":{"duracion":"union de ventanas explicitas de la tarea hasta el cierre; NULL sin ventanas completas",
            "reintentos":"NULL: el hook no garantiza correlacion ni cobertura de reintentos; repetir herramienta no demuestra reintento",
            "tokens":"NULL: los hooks actuales no exponen tokens"},
        "alcance":"instantanea de la tarea bajo la comprobacion elegida; no afirma calidad global ni causalidad"}),
    )
}

impl Store {
    pub fn leer_episodio(&mut self, proyecto: &str, clave: &str) -> Result<Value> {
        nonempty(proyecto)?;
        nonempty(clave)?;
        let tx = self.conn.transaction()?;
        let salida = leer(&tx, proyecto, clave)?;
        tx.commit()?;
        Ok(salida)
    }

    /// Congela lo conocido en una transacción. La clave identifica la instantánea,
    /// incluso si después llegan enlaces tardíos o correcciones del resultado.
    pub fn construir_episodio(
        &mut self,
        clave: &str,
        proyecto: &str,
        tarea: &str,
        resultado: &str,
    ) -> Result<Value> {
        for valor in [clave, proyecto, tarea, resultado] {
            nonempty(valor)?;
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existente) = rows(
            &tx,
            "SELECT t.clave AS tarea,t.proyecto,r.clave AS resultado FROM episodio e
             JOIN tarea t ON t.id=e.tarea_id JOIN episodio_resultado er ON er.episodio_id=e.id
             JOIN tarea_resultado r ON r.id=er.tarea_resultado_id WHERE e.clave=?1",
            [clave],
        )?
        .pop()
        {
            ensure!(
                existente["tarea"] == tarea
                    && existente["proyecto"] == proyecto
                    && existente["resultado"] == resultado,
                "conflicto de identidad del episodio"
            );
            let salida = leer(&tx, proyecto, clave)?;
            tx.commit()?;
            return Ok(salida);
        }
        let t = rows(
            &tx,
            "SELECT id FROM tarea WHERE clave=?1 AND proyecto=?2",
            params![tarea, proyecto],
        )?
        .pop()
        .context("tarea ausente en el proyecto")?;
        let tarea_id = t["id"].as_i64().context("tarea sin id")?;
        let cierre = rows(
            &tx,
            "SELECT id,tipo,definicion_version FROM tarea_evento WHERE tarea_id=?1
             AND tipo IN ('inicio','pausa','reanudacion','cierre') ORDER BY id DESC LIMIT 1",
            [tarea_id],
        )?
        .pop()
        .context("tarea sin cierre explicito")?;
        ensure!(cierre["tipo"] == "cierre", "la tarea no esta cerrada");
        let cierre_id = cierre["id"].as_i64().context("cierre sin id")?;
        let version = cierre["definicion_version"]
            .as_i64()
            .context("cierre sin definicion")?;
        let comprobacion = rows(&tx,
            "SELECT * FROM tarea_resultado WHERE clave=?1 AND tarea_id=?2
             AND definicion_version=?3 AND alcance='tarea' AND resultado IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM tarea_resultado posterior WHERE posterior.anterior_id=tarea_resultado.id)",
            params![resultado,tarea_id,version])?.pop().context("comprobacion ausente, desconocida, corregida o incompatible con el cierre")?;
        let definicion: i64 = tx.query_row("SELECT id FROM tarea_evento WHERE tarea_id=?1 AND tipo='definicion' AND definicion_version=?2",params![tarea_id,version],|r|r.get(0))?;
        let eventos = rows(&tx,
            "SELECT i.id,c.json_original FROM tarea_vinculo v JOIN instancia i ON i.id=v.instancia_id
             JOIN captura_evento c ON c.instancia_id=i.id WHERE v.tarea_id=?1
               AND i.origen='herramienta' AND (json_extract(i.metadatos_json,'$.tipo') IN ('herramienta-ok','herramienta-error')
                 OR json_type(i.metadatos_json,'$.payload_hook.tool_name')='text'
                 OR json_type(c.json_original,'$.tool_name')='text')
             ORDER BY c.grupo_evidencia COLLATE BINARY,c.secuencia,i.id",[tarea_id])?;
        let mut pasos = Vec::new();
        for evento in &eventos {
            let crudo: Value =
                serde_json::from_str(evento["json_original"].as_str().context("crudo ausente")?)?;
            pasos.push(normalizar_herramienta(&crudo));
        }
        let cronologia = rows(
            &tx,
            "SELECT tipo,registrado_en FROM tarea_evento WHERE tarea_id=?1 AND id<=?2 ORDER BY id",
            params![tarea_id, cierre_id],
        )?;
        let marcas: Vec<(String, i64)> = cronologia
            .iter()
            .map(|m| {
                Ok((
                    m["tipo"].as_str().context("marcador sin tipo")?.into(),
                    m["registrado_en"].as_i64().context("marcador sin tiempo")?,
                ))
            })
            .collect::<Result<_>>()?;
        let (ventanas, abierta) = ventanas(&marcas);
        let duracion = if abierta.is_none() && !ventanas.is_empty() {
            Some(ventanas.iter().map(|(a, b)| b - a).sum::<i64>())
        } else {
            None
        };
        let hash = hash_secuencia(&pasos)?;
        tx.execute("INSERT INTO episodio(clave,tarea_id,definicion_evento_id,cierre_evento_id,criterio_id,criterio_version,normalizador_version,secuencia_sha256,cantidad_pasos,creada_en)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![clave,tarea_id,definicion,cierre_id,
                comprobacion["criterio_id"].as_str(),comprobacion["criterio_version"].as_str(),NORMALIZADOR_VERSION,hash,pasos.len() as i64,now()?])?;
        let id = tx.last_insert_rowid();
        for (indice, (evento, paso)) in eventos.iter().zip(&pasos).enumerate() {
            tx.execute(
                "INSERT INTO episodio_paso VALUES (?1,?2,?3,?4,?5)",
                params![
                    id,
                    (indice + 1) as i64,
                    evento["id"].as_i64(),
                    paso.herramienta,
                    paso.tipo_destino
                ],
            )?;
        }
        // No se infieren reintentos ni tokens desde errores, repeticiones o bytes.
        tx.execute(
            "INSERT INTO episodio_resultado VALUES (?1,?2,?3,?4,NULL,NULL)",
            params![
                id,
                comprobacion["id"].as_i64(),
                comprobacion["resultado"].as_i64(),
                duracion
            ],
        )?;
        let salida = leer(&tx, proyecto, clave)?;
        tx.commit()?;
        Ok(salida)
    }
}
