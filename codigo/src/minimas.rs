//! Reducción observacional y versionada. Las reglas completas están en README,
//! «Secuencias mínimas». Conservar un paso no acredita contribución causal.
use crate::cortes::{es_comprobacion, resultado};
use crate::episodios::{hash_secuencia, PasoNormalizado, NORMALIZADOR_VERSION};
use crate::execution::{nonempty, rows};
use crate::{now, Store};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};

pub const REGLA_VERSION: &str = "minima-conservadora-v1";

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct DecisionPaso {
    pub orden_original: usize,
    pub en_minima: bool,
    pub motivo: &'static str,
    pub certeza: &'static str,
    pub apoyo_orden: Option<usize>,
}

fn payload(v: &Value) -> &Value {
    v.pointer("/metadatos/payload_hook")
        .filter(|p| p.is_object())
        .unwrap_or(v)
}

fn texto(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.trim().is_empty())
}

fn estado(v: &Value) -> Option<bool> {
    let p = payload(v);
    if p["tool_name"] == "Bash" {
        return resultado(p);
    }
    if p["is_interrupt"] == true {
        return None;
    }
    match p["hook_event_name"].as_str() {
        Some("PostToolUse") => Some(true),
        Some("PostToolUseFailure") => Some(false),
        _ => None,
    }
}

fn mismo_contexto(a: &Value, b: &Value) -> bool {
    let grupo = texto(&a["grupo_evidencia"]);
    let cwd = texto(&payload(a)["cwd"]);
    let intervalo = a["metadatos"]["intervalo_captura"].as_i64();
    grupo.is_some()
        && grupo == texto(&b["grupo_evidencia"])
        && cwd.is_some()
        && cwd == texto(&payload(b)["cwd"])
        && payload(a)["agent_id"] == payload(b)["agent_id"]
        && intervalo.is_some()
        && intervalo == b["metadatos"]["intervalo_captura"].as_i64()
}

fn llamadas_distintas(a: &Value, b: &Value) -> bool {
    let uno = texto(&payload(a)["tool_use_id"]);
    let dos = texto(&payload(b)["tool_use_id"]);
    uno.is_some() && dos.is_some() && uno != dos
}

fn equivalente(a: &Value, b: &Value) -> bool {
    let pa = payload(a);
    let pb = payload(b);
    let entrada_igual = if pa["tool_name"] == "Bash" {
        texto(&pa["tool_input"]["command"]).is_some()
            && pa["tool_input"]["command"] == pb["tool_input"]["command"]
    } else {
        pa["tool_input"].is_object() && pa["tool_input"] == pb["tool_input"]
    };
    mismo_contexto(a, b)
        && llamadas_distintas(a, b)
        && texto(&pa["tool_name"]).is_some()
        && pa["tool_name"] == pb["tool_name"]
        && entrada_igual
}

fn archivo(v: &Value) -> Option<&str> {
    texto(&payload(v)["tool_input"]["file_path"])
}

fn edicion(v: &Value) -> bool {
    matches!(payload(v)["tool_name"].as_str(), Some("Edit" | "Write"))
}

fn destino_consulta(v: &Value) -> Option<&str> {
    match payload(v)["tool_name"].as_str()? {
        "Read" => archivo(v),
        "Grep" | "Glob" => texto(&payload(v)["tool_input"]["path"]),
        _ => None,
    }
}

fn descartar(
    d: &mut DecisionPaso,
    motivo: &'static str,
    certeza: &'static str,
    apoyo: Option<usize>,
) {
    d.en_minima = false;
    d.motivo = motivo;
    d.certeza = certeza;
    d.apoyo_orden = apoyo;
}

/// Total, determinista y sin E/S. Los índices públicos son órdenes desde 1.
/// Sólo los pasos que se conservan pueden aportar evidencia para retirar otros.
pub fn clasificar_pasos(pasos: &[Value], exito: bool) -> Vec<DecisionPaso> {
    let mut decisiones: Vec<_> = (0..pasos.len())
        .map(|i| DecisionPaso {
            orden_original: i + 1,
            en_minima: exito,
            motivo: if exito {
                "conservado_sin_evidencia"
            } else {
                "episodio_sin_exito"
            },
            certeza: if exito { "no_atribuida" } else { "alta" },
            apoyo_orden: None,
        })
        .collect();
    if !exito {
        return decisiones;
    }
    let mut siguiente: Option<usize> = None;
    for i in (0..pasos.len()).rev() {
        let p = &pasos[i];
        if let Some(j) = siguiente {
            let posterior = payload(&pasos[j]);
            if edicion(p)
                && estado(p).is_some()
                && posterior["tool_name"] == "Write"
                && posterior["tool_input"]["content"].is_string()
                && estado(&pasos[j]) == Some(true)
                && archivo(p).is_some()
                && archivo(p) == archivo(&pasos[j])
                && mismo_contexto(p, &pasos[j])
                && llamadas_distintas(p, &pasos[j])
            {
                descartar(
                    &mut decisiones[i],
                    "reescritura_completa_posterior",
                    "alta",
                    Some(j + 1),
                );
                continue;
            }
        }
        if estado(p) == Some(false) {
            if let Some(j) = (i + 1..pasos.len()).find(|j| {
                decisiones[*j].en_minima
                    && estado(&pasos[*j]) == Some(true)
                    && equivalente(p, &pasos[*j])
            }) {
                let motivo = if es_comprobacion(payload(p)) {
                    "comprobacion_fallida_superada"
                } else {
                    "reintento_equivalente_exitoso"
                };
                descartar(&mut decisiones[i], motivo, "alta", Some(j + 1));
                continue;
            }
            if matches!(
                payload(p)["tool_name"].as_str(),
                Some("Read" | "Grep" | "Glob")
            ) {
                descartar(&mut decisiones[i], "consulta_fallida", "media", None);
                continue;
            }
        }
        siguiente = Some(i);
    }
    for i in 0..pasos.len() {
        if decisiones[i].en_minima {
            if let Some(destino) = destino_consulta(&pasos[i]) {
                let reaparece = (i + 1..pasos.len()).any(|j| {
                    decisiones[j].en_minima
                        && mismo_contexto(&pasos[i], &pasos[j])
                        && (archivo(&pasos[j]).or_else(|| destino_consulta(&pasos[j]))
                            == Some(destino))
                });
                if !reaparece {
                    decisiones[i].motivo = "candidata_sin_reuso_observado";
                    decisiones[i].certeza = "baja";
                }
            }
        }
    }
    decisiones
}

struct Fuente {
    id: i64,
    exito: bool,
    pasos: Vec<Value>,
}

fn fuente(c: &Connection, proyecto: &str, origen: &str, referencia: &str) -> Result<Fuente> {
    nonempty(proyecto)?;
    nonempty(referencia)?;
    let cabecera = match origen {
        "manual_tarea" => rows(c,
            "SELECT e.id,e.normalizador_version,e.cantidad_pasos,r.resultado FROM episodio e
             JOIN episodio_resultado r ON r.episodio_id=e.id JOIN tarea t ON t.id=e.tarea_id
             WHERE t.proyecto=?1 AND e.clave=?2",params![proyecto,referencia])?,
        "automatico_lista" => rows(c,
            "SELECT e.id,e.normalizador_version,e.cantidad_pasos,r.resultado FROM episodio_automatico e
             JOIN episodio_automatico_sello s ON s.episodio_id=e.id
             LEFT JOIN episodio_automatico_resultado r ON r.episodio_id=e.id
             WHERE e.proyecto=?1 AND e.id=?2",params![proyecto,referencia.parse::<i64>().context("ID de episodio automatico invalido")?])?,
        _ => bail!("origen invalido: usar manual_tarea o automatico_lista"),
    }.pop().context("episodio ausente en el proyecto o sin sellar")?;
    ensure!(
        cabecera["normalizador_version"] == NORMALIZADOR_VERSION,
        "normalizador del episodio no soportado por esta regla"
    );
    let id = cabecera["id"].as_i64().context("episodio sin id")?;
    // Los nombres SQL son constantes internas, nunca texto recibido del usuario.
    let sql = if origen == "manual_tarea" {
        "SELECT p.orden,p.instancia_id,p.herramienta,p.tipo_destino,c.json_original FROM episodio_paso p
         JOIN captura_evento c ON c.instancia_id=p.instancia_id WHERE p.episodio_id=?1 ORDER BY p.orden"
    } else {
        "SELECT p.orden,p.instancia_id,p.herramienta,p.tipo_destino,c.json_original FROM episodio_automatico_paso p
         JOIN captura_evento c ON c.instancia_id=p.instancia_id WHERE p.episodio_id=?1 ORDER BY p.orden"
    };
    let pasos = rows(c, sql, [id])?;
    ensure!(
        cabecera["cantidad_pasos"].as_u64() == Some(pasos.len() as u64),
        "episodio incompleto"
    );
    Ok(Fuente {
        id,
        exito: cabecera["resultado"] == 1,
        pasos,
    })
}

fn existente(c: &Connection, origen: &str, id: i64) -> Result<Option<Value>> {
    Ok(rows(c,"SELECT d.* FROM secuencia_minima d JOIN secuencia_minima_sello s ON s.derivacion_id=d.id
        WHERE origen=?1 AND coalesce(episodio_manual_id,episodio_automatico_id)=?2 AND regla_version=?3",
        params![origen,id,REGLA_VERSION])?.pop())
}

fn leer(c: &Connection, origen: &str, f: &Fuente) -> Result<Value> {
    let cabecera = existente(c, origen, f.id)?.context("secuencia minima todavia no calculada")?;
    let id = cabecera["id"].as_i64().context("derivacion sin id")?;
    let mut decisiones = rows(
        c,
        "SELECT * FROM secuencia_minima_paso WHERE derivacion_id=?1 ORDER BY orden_original",
        [id],
    )?;
    for d in &mut decisiones {
        let orden = d["orden_original"].as_u64().context("decision sin orden")? as usize;
        let paso = f
            .pasos
            .get(orden.checked_sub(1).context("orden de decision invalido")?)
            .context("decision fuera del episodio")?;
        d["herramienta"] = paso["herramienta"].clone();
        d["tipo_destino"] = paso["tipo_destino"].clone();
    }
    let minima = if cabecera["llego"] == 1 {
        json!(decisiones
            .iter()
            .filter(|d| d["en_minima"] == 1)
            .collect::<Vec<_>>())
    } else {
        Value::Null
    };
    let descartados: Vec<_> = decisiones.iter().filter(|d| d["en_minima"] == 0).collect();
    Ok(
        json!({"derivacion":cabecera,"minima":minima,"descartados":descartados,
        "alcance":"reduccion observacional por reglas; conservar no acredita contribucion causal",
        "certeza":"alta o media califican el descarte; baja indica una candidata que se conserva",
        "sin_exito":"no llego este episodio; no afirma que cada herramienta haya fallado"}),
    )
}

impl Store {
    pub fn leer_secuencia_minima(
        &mut self,
        proyecto: &str,
        origen: &str,
        referencia: &str,
    ) -> Result<Value> {
        let tx = self.conn.transaction()?;
        let f = fuente(&tx, proyecto, origen, referencia)?;
        let salida = leer(&tx, origen, &f)?;
        tx.commit()?;
        Ok(salida)
    }

    /// Una derivación por episodio y versión. Sólo lee la instantánea sellada;
    /// las clasificaciones y su sello se confirman en la misma transacción.
    pub fn calcular_secuencia_minima(
        &mut self,
        proyecto: &str,
        origen: &str,
        referencia: &str,
    ) -> Result<Value> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let f = fuente(&tx, proyecto, origen, referencia)?;
        if existente(&tx, origen, f.id)?.is_some() {
            let salida = leer(&tx, origen, &f)?;
            tx.commit()?;
            return Ok(salida);
        }
        let crudos: Vec<Value> = f
            .pasos
            .iter()
            .map(|p| {
                serde_json::from_str(p["json_original"].as_str().context("paso sin crudo")?)
                    .context("crudo invalido")
            })
            .collect::<Result<_>>()?;
        let decisiones = clasificar_pasos(&crudos, f.exito);
        let mut conservados = Vec::new();
        for (d, p) in decisiones.iter().zip(&f.pasos) {
            if d.en_minima {
                conservados.push(PasoNormalizado {
                    herramienta: p["herramienta"]
                        .as_str()
                        .context("paso sin herramienta")?
                        .into(),
                    tipo_destino: p["tipo_destino"]
                        .as_str()
                        .context("paso sin destino")?
                        .into(),
                });
            }
        }
        let hash = if f.exito {
            Some(hash_secuencia(&conservados)?)
        } else {
            None
        };
        let manual = (origen == "manual_tarea").then_some(f.id);
        let automatico = (origen == "automatico_lista").then_some(f.id);
        tx.execute("INSERT INTO secuencia_minima(origen,episodio_manual_id,episodio_automatico_id,regla_version,normalizador_version,llego,hash_minima,cantidad_original,cantidad_minima,creada_en)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![origen,manual,automatico,REGLA_VERSION,NORMALIZADOR_VERSION,f.exito,hash,f.pasos.len() as i64,conservados.len() as i64,now()?])?;
        let id = tx.last_insert_rowid();
        for (d, p) in decisiones.iter().zip(&f.pasos) {
            tx.execute(
                "INSERT INTO secuencia_minima_paso VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    id,
                    d.orden_original as i64,
                    p["instancia_id"].as_i64(),
                    d.en_minima,
                    d.motivo,
                    d.certeza,
                    d.apoyo_orden.map(|n| n as i64)
                ],
            )?;
        }
        tx.execute("INSERT INTO secuencia_minima_sello VALUES (?1)", [id])?;
        let salida = leer(&tx, origen, &f)?;
        tx.commit()?;
        Ok(salida)
    }
}
