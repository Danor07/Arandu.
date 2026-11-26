//! Recuperación textual y entregas locales; no estima utilidad ni uso por una LLM.
use crate::{digest, now, Store};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const CONFIG: &str = r#"{"algoritmo":"episodica-fts5-v1","tokenizer":"unicode61 remove_diacritics 2","consulta":"terminos-literales-OR","ranking":"bm25 ASC,clave_evento BINARY ASC,id ASC","fragmento_terminos":24,"paquete":"eventos-completos-jsonl-v1","unidad":"bytes_utf8","corte":"id<problema.id y timestamp<=problema.timestamp; otro episodio"}"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub proyecto: String,
    pub consulta: String,
    pub origen: Option<String>,
    pub rol: Option<String>,
    pub limite: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub instancia_id: i64,
    pub clave_evento: String,
    pub grupo_evidencia: String,
    pub secuencia: i64,
    pub origen: String,
    pub rol: String,
    pub fragmento: String,
    pub score_textual: f64,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub proyecto: String,
    pub sin_proyecto_en_base: i64,
    pub resultados: Vec<Hit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepareRequest {
    pub problema_id: i64,
    pub busqueda: SearchRequest,
    pub presupuesto_bytes: usize,
}

fn project(value: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty(),
        "se exige proyecto explicito no vacio"
    );
    Ok(())
}

impl SearchRequest {
    fn validate(&self) -> Result<()> {
        project(&self.proyecto)?;
        ensure!(self.limite <= 1000, "limite maximo: 1000 candidatos");
        ensure!(
            self.consulta.len() <= 16_384,
            "consulta supera 16384 bytes UTF-8"
        );
        if let Some(origin) = &self.origen {
            ensure!(
                ["usuario", "herramienta", "modelo", "replay"].contains(&origin.as_str()),
                "origen invalido"
            );
        }
        if let Some(role) = &self.rol {
            ensure!(
                ["problema", "solucion", "correccion", "otro"].contains(&role.as_str()),
                "rol invalido"
            );
        }
        Ok(())
    }
}

// FTS recibe términos citados, nunca la gramática aportada por el usuario.
// Separar en palabras alfanuméricas también convierte solo puntuación en vacío.
fn literal_query(text: &str) -> Result<String> {
    let mut terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect();
    terms.sort();
    terms.dedup();
    ensure!(terms.len() <= 64, "consulta supera 64 terminos distintos");
    Ok(terms
        .into_iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(" OR "))
}

pub(crate) struct Cutoff {
    pub(crate) problem_id: i64,
    pub(crate) episode: String,
    pub(crate) timestamp: i64,
}

/// Reutiliza el corte exacto del buscador para un evento consultante con rol
/// problema. Sólo lectura: no crea eventos de recuperación ni paquetes.
pub(crate) fn buscar_con_corte(
    conn: &Connection,
    request: &SearchRequest,
    problema_id: i64,
    episodio: &str,
    timestamp: i64,
) -> Result<Vec<Hit>> {
    let cutoff = Cutoff {
        problem_id: problema_id,
        episode: episodio.to_string(),
        timestamp,
    };
    search(conn, request, Some(&cutoff))
}

fn search(conn: &Connection, request: &SearchRequest, cutoff: Option<&Cutoff>) -> Result<Vec<Hit>> {
    request.validate()?;
    let query = literal_query(&request.consulta)?;
    if query.is_empty() || request.limite == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT i.id,i.clave_evento,i.grupo_evidencia,c.secuencia,i.origen,i.rol,
                snippet(instancia_fts,0,'[',']',' … ',24),bm25(instancia_fts)
         FROM instancia_fts JOIN instancia i ON i.id=instancia_fts.rowid
         JOIN captura_evento c ON c.instancia_id=i.id
         JOIN instancia_proyecto p ON p.instancia_id=i.id
         WHERE instancia_fts MATCH ?1 AND p.proyecto=?2
           AND (?3 IS NULL OR i.origen=?3) AND (?4 IS NULL OR i.rol=?4)
           AND (?5 IS NULL OR (i.id<?5 AND i.grupo_evidencia<>?6 AND i.timestamp<=?7))
         ORDER BY bm25(instancia_fts),i.clave_evento COLLATE BINARY,i.id LIMIT ?8",
    )?;
    let rows = stmt.query_map(
        params![
            query,
            request.proyecto,
            request.origen,
            request.rol,
            cutoff.map(|c| c.problem_id),
            cutoff.map(|c| c.episode.as_str()),
            cutoff.map(|c| c.timestamp),
            request.limite as i64
        ],
        |r| {
            Ok(Hit {
                instancia_id: r.get(0)?,
                clave_evento: r.get(1)?,
                grupo_evidencia: r.get(2)?,
                secuencia: r.get(3)?,
                origen: r.get(4)?,
                rol: r.get(5)?,
                fragmento: r.get(6)?,
                score_textual: r.get(7)?,
            })
        },
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

impl Store {
    pub fn search(&mut self, request: &SearchRequest) -> Result<SearchResults> {
        let tx = self.conn.transaction()?;
        let hits = search(&tx, request, None)?;
        let missing = tx.query_row(
            "SELECT count(*) FROM instancia_proyecto WHERE proyecto IS NULL",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        Ok(SearchResults {
            proyecto: request.proyecto.clone(),
            sin_proyecto_en_base: missing,
            resultados: hits,
        })
    }

    /// Exploración de vecinos; no crea un evento de recuperación ni una entrega.
    pub fn detail(
        &mut self,
        requested_project: &str,
        id: i64,
        before: usize,
        after: usize,
    ) -> Result<Value> {
        project(requested_project)?;
        ensure!(
            before <= 1000 && after <= 1000,
            "maximo 1000 vecinos por lado"
        );
        let tx = self.conn.transaction()?;
        let (episode, sequence): (String,i64) = tx.query_row(
            "SELECT i.grupo_evidencia,c.secuencia FROM instancia i
             JOIN captura_evento c ON c.instancia_id=i.id JOIN instancia_proyecto p ON p.instancia_id=i.id
             WHERE i.id=?1 AND p.proyecto=?2",params![id,requested_project],|r|Ok((r.get(0)?,r.get(1)?)))
             .optional()?.context("evento no encontrado en el proyecto indicado")?;
        let mut ids = Vec::new();
        for (sql, count) in [
            ("SELECT c.instancia_id FROM captura_evento c JOIN instancia_proyecto p ON p.instancia_id=c.instancia_id WHERE c.grupo_evidencia=?1 AND c.secuencia<?2 AND p.proyecto=?3 ORDER BY c.secuencia DESC LIMIT ?4",before),
            ("SELECT c.instancia_id FROM captura_evento c JOIN instancia_proyecto p ON p.instancia_id=c.instancia_id WHERE c.grupo_evidencia=?1 AND c.secuencia>?2 AND p.proyecto=?3 ORDER BY c.secuencia ASC LIMIT ?4",after),
        ] {
            let mut stmt = tx.prepare(sql)?;
            let rows = stmt.query_map(params![episode,sequence,requested_project,count as i64],|r|r.get::<_,i64>(0))?;
            ids.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        ids.push(id);
        let mut events = Vec::new();
        for row_id in ids {
            let (seq, raw): (i64, String) = tx.query_row(
                "SELECT secuencia,json_original FROM captura_evento WHERE instancia_id=?1",
                [row_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            events.push((seq,json!({"instancia_id":row_id,"secuencia":seq,"json_original":raw,"evento":serde_json::from_str::<Value>(&raw)?})));
        }
        events.sort_by_key(|(seq, _)| *seq);
        tx.commit()?;
        Ok(
            json!({"proyecto":requested_project,"instancia_id":id,"grupo_evidencia":episode,
            "eventos":events.into_iter().map(|(_,v)|v).collect::<Vec<_>>()}),
        )
    }

    pub fn rebuild_search(&mut self) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO instancia_fts(instancia_fts) VALUES ('rebuild')",
            [],
        )?;
        tx.execute(
            "INSERT INTO instancia_fts(instancia_fts,rank) VALUES ('integrity-check',1)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Congela la preparación local antes de devolverla. No confirma recepción externa.
    pub fn prepare_memory(&mut self, key: &str, request: &PrepareRequest) -> Result<Value> {
        ensure!(!key.trim().is_empty(), "clave de paquete vacia");
        request.busqueda.validate()?;
        let budget: i64 = request.presupuesto_bytes.try_into()?;
        let parameters = serde_json::to_string(request)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT p.parametros_json,p.paquete_json FROM evento_recuperacion r
             LEFT JOIN paquete_recuperacion p ON p.evento_id=r.id WHERE r.clave_evento=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((previous, packet)) = existing {
            ensure!(
                previous.as_deref() == Some(parameters.as_str()),
                "conflicto de identidad de paquete: {key}"
            );
            let packet: Value = serde_json::from_str(&packet.context("paquete ausente")?)?;
            tx.commit()?;
            return Ok(packet);
        }
        let (episode,timestamp,content,role):(String,i64,String,String) = tx.query_row(
            "SELECT i.grupo_evidencia,i.timestamp,i.contenido,i.rol FROM instancia i
             JOIN instancia_proyecto p ON p.instancia_id=i.id JOIN captura_evento c ON c.instancia_id=i.id
             WHERE i.id=?1 AND p.proyecto=?2",params![request.problema_id,request.busqueda.proyecto],
             |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.context("problema no encontrado en el proyecto indicado")?;
        ensure!(
            role == "problema",
            "el evento consultante debe tener rol problema"
        );
        let cutoff = Cutoff {
            problem_id: request.problema_id,
            episode,
            timestamp,
        };
        let hits = search(&tx, &request.busqueda, Some(&cutoff))?;
        let vigente = if request.busqueda.limite > 0
            && request.busqueda.origen.is_none()
            && request.busqueda.rol.is_none()
        {
            crate::vigentes::recuperar(&tx, &request.busqueda.proyecto, &content, &cutoff)?
        } else {
            None
        };
        let bloque_vigente = vigente
            .as_ref()
            .map(|v| serde_json::to_string(v).map(|s| s + "\n"))
            .transpose()?;
        let cabe_vigente = bloque_vigente
            .as_ref()
            .is_some_and(|b| b.len() <= request.presupuesto_bytes);
        let config_mixta = if cabe_vigente {
            let mut config: Value = serde_json::from_str(CONFIG)?;
            config["vigente"] = json!(crate::vigentes::VIGENTE_VERSION);
            config["regla_tipo"] = json!(crate::vigentes::TIPO_VERSION);
            config["prioridad"] = json!("vigente completa antes de textual");
            Some(config.to_string())
        } else {
            None
        };
        let config_json = config_mixta.as_deref().unwrap_or(CONFIG);
        let time = now()?;
        let config: Option<i64> = tx
            .query_row(
                "SELECT id FROM configuracion WHERE parametros_json=?1 ORDER BY id LIMIT 1",
                [config_json],
                |r| r.get(0),
            )
            .optional()?;
        let config = match config {
            Some(id) => id,
            None => {
                tx.execute(
                    "INSERT INTO configuracion(timestamp,parametros_json) VALUES (?1,?2)",
                    params![time, config_json],
                )?;
                tx.last_insert_rowid()
            }
        };
        tx.execute("INSERT INTO evento_recuperacion(tarea_hash,clave_evento,problema_instancia_id,configuracion_id,timestamp) VALUES (?1,?2,?3,?4,?5)",params![digest(content.as_bytes()),key,request.problema_id,config,time])?;
        let recovery_id = tx.last_insert_rowid();
        let mut memory = String::new();
        let mut selected = Vec::new();
        let mut omitted = Vec::new();
        if let (Some(v), Some(block)) = (&vigente, &bloque_vigente) {
            if cabe_vigente {
                tx.execute(
                    "INSERT INTO recuperacion_vigente VALUES (?1,?2,?3,?4,?5,?6)",
                    params![
                        recovery_id,
                        v["tipo"]["clave"].as_str(),
                        v["vigente"]["camino"]["derivacion_id"].as_i64(),
                        crate::vigentes::VIGENTE_VERSION,
                        block,
                        v.to_string()
                    ],
                )?;
                crate::vigentes::actualizar_archivo(&tx, &request.busqueda.proyecto, &content)?;
                memory.push_str(block);
            } else {
                omitted.push(json!({"tipo_registro":"solucion_vigente_provisional","motivo":"no_cabe_completa","bytes_utf8":block.len()}));
            }
        }
        for hit in hits {
            let original: String = tx.query_row(
                "SELECT contenido FROM instancia WHERE id=?1",
                [hit.instancia_id],
                |r| r.get(0),
            )?;
            // El JSONL integra identificadores y texto: también sus escapes y LF
            // consumen presupuesto. Nunca se trunca el original ni el bloque.
            let block = serde_json::to_string(
                &json!({"instancia_id":hit.instancia_id,"clave_evento":hit.clave_evento,
                "grupo_evidencia":hit.grupo_evidencia,"secuencia":hit.secuencia,"origen":hit.origen,"rol":hit.rol,"contenido":original}),
            )? + "\n";
            if block.len() > request.presupuesto_bytes - memory.len() {
                omitted.push(json!({"instancia_id":hit.instancia_id,"motivo":"no_cabe_completo","bytes_utf8":block.len()}));
                continue;
            }
            let order = selected.len() + 1;
            tx.execute(
                "INSERT INTO evento_recuperacion_instancia VALUES (?1,?2)",
                params![recovery_id, hit.instancia_id],
            )?;
            tx.execute(
                "INSERT INTO recuperacion_entrega_instancia VALUES (?1,?2,?3,?4,?5)",
                params![
                    recovery_id,
                    hit.instancia_id,
                    order as i64,
                    hit.score_textual,
                    block
                ],
            )?;
            memory.push_str(&block);
            selected.push(json!({"orden":order,"candidato":hit,"texto_entregado":block,"bytes_utf8":block.len()}));
        }
        let mut packet = json!({"version":1,"clave_paquete":key,"evento_recuperacion_id":recovery_id,
            "estado":"preparado_localmente","preparado_en":time,"parametros":request,
            "corte":{"instancia_id_exclusiva":cutoff.problem_id,"timestamp_maximo":cutoff.timestamp,"episodio_excluido":cutoff.episode},
            "texto_memoria":memory,"bytes_utf8":memory.len(),"seleccion":selected,"omitidos":omitted});
        if cabe_vigente {
            packet["vigente"] = json!(vigente);
        }
        tx.execute(
            "INSERT INTO paquete_recuperacion VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                recovery_id,
                request.busqueda.proyecto,
                parameters,
                budget,
                memory,
                memory.len() as i64,
                serde_json::to_string(&packet)?
            ],
        )?;
        tx.commit()?;
        Ok(packet)
    }

    pub fn memory_packet(&self, requested_project: &str, key: &str) -> Result<Value> {
        project(requested_project)?;
        let raw:String = self.conn.query_row(
            "SELECT p.paquete_json FROM paquete_recuperacion p JOIN evento_recuperacion r ON r.id=p.evento_id WHERE r.clave_evento=?1 AND p.proyecto=?2",
            params![key,requested_project],|r|r.get(0)).optional()?.context("paquete no encontrado en el proyecto indicado")?;
        Ok(serde_json::from_str(&raw)?)
    }
}
