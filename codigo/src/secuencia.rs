//! Representación derivada del recorrido observado de episodios sellados.
//! No busca un mínimo causal: conserva todo paso sin una referencia explícita
//! de descarte y mantiene los fallos como observaciones, no como ramas muertas.
use crate::execution::{nonempty, rows};
use crate::{digest, now, Store};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

pub const ALGORITMO: &str = "camino-observado-descartes-v1";
pub const REF_DESCARTA: &str = "descarta_paso";
pub const REF_REVIERTE: &str = "revierte_paso";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenciaDescarte {
    pub tipo: String,
    pub clave_evento: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasoObservado {
    pub orden: i64,
    pub instancia_id: i64,
    pub clave_evento: String,
    pub herramienta: String,
    pub tipo_destino: String,
    pub grupo_evidencia: String,
    pub timestamp: i64,
    pub estado_tecnico: String,
    pub referencias: Vec<ReferenciaDescarte>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescarteCalculado {
    pub instancia_id: i64,
    pub evidencia_instancia_id: i64,
    pub tipo_referencia: String,
    pub aplicada: bool,
    pub motivo_no_aplicada: Option<String>,
    pub orden_descartado: i64,
    pub orden_evidencia: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionEstado {
    pub estado: &'static str,
    pub motivo: &'static str,
    /// El resultado acredita el episodio fuente, no una reejecución del camino
    /// derivado. La variante reducida es sólo una candidata para comparar.
    pub acredita_episodio_fuente: bool,
}

fn no_candidata(motivo: &'static str) -> DecisionEstado {
    DecisionEstado {
        estado: "no_candidata",
        motivo,
        acredita_episodio_fuente: false,
    }
}

/// Decisión pura. Un fallo, una ausencia o una corrección nunca producen una
/// candidata; tampoco un episodio sin pasos observados. Marcar descartes no
/// reejecuta el camino: el resultado sigue acreditando el episodio fuente.
pub fn decidir_estado(
    resultado: Option<i64>,
    corregido: bool,
    pasos: usize,
    descartados: usize,
) -> DecisionEstado {
    if resultado != Some(1) {
        return no_candidata(if resultado == Some(0) {
            "resultado_fallido"
        } else {
            "sin_resultado"
        });
    }
    if corregido {
        return no_candidata("resultado_corregido");
    }
    if pasos == 0 {
        return no_candidata("sin_pasos_observados");
    }
    if descartados > 0 {
        DecisionEstado {
            estado: "candidata_con_descartes",
            motivo: "camino_derivado_con_descartes; el resultado acredita el episodio fuente",
            acredita_episodio_fuente: true,
        }
    } else {
        DecisionEstado {
            estado: "candidata_sin_descartes",
            motivo: "recorrido_observado; el resultado acredita el episodio fuente",
            acredita_episodio_fuente: true,
        }
    }
}

/// Aplica sólo referencias tipadas entre pasos del mismo episodio. Se recorre
/// de atrás hacia adelante: una evidencia posterior puede apartar un paso
/// anterior. Una referencia hacia adelante, al mismo paso, emitida por una
/// llamada fallida/desconocida, de otro grupo de evidencia o que pretenda
/// apartar la comprobación que acredita el resultado se conserva como no
/// aplicada con su motivo.
pub fn calcular_descartes(
    pasos: &[PasoObservado],
    evidencia_resultado: Option<i64>,
) -> Vec<DescarteCalculado> {
    let mut por_clave = BTreeMap::new();
    for (indice, paso) in pasos.iter().enumerate() {
        por_clave
            .entry(paso.clave_evento.as_str())
            .or_insert(indice);
    }
    let mut excluidos = BTreeSet::new();
    let mut salida = Vec::new();
    for indice in (0..pasos.len()).rev() {
        let evidencia = &pasos[indice];
        for referencia in &evidencia.referencias {
            let Some(&destino) = por_clave.get(referencia.clave_evento.as_str()) else {
                // Una referencia a un evento fuera del episodio no tiene paso
                // que conservar en la tabla de descartes.
                continue;
            };
            let paso = &pasos[destino];
            let motivo = if excluidos.contains(&indice) {
                Some("evidencia_descartada")
            } else if evidencia.estado_tecnico != "ok" {
                Some("evidencia_fallida_o_desconocida")
            } else if evidencia.grupo_evidencia != paso.grupo_evidencia {
                Some("grupo_evidencia_distinto")
            } else if destino == indice {
                Some("referencia_al_mismo_paso")
            } else if destino.cmp(&indice) == Ordering::Greater {
                Some("evidencia_no_posterior")
            } else if Some(paso.instancia_id) == evidencia_resultado {
                Some("comprobacion_del_resultado")
            } else if excluidos.contains(&destino) {
                Some("paso_ya_descartado")
            } else {
                None
            };
            if motivo.is_none() {
                excluidos.insert(destino);
            }
            salida.push(DescarteCalculado {
                instancia_id: paso.instancia_id,
                evidencia_instancia_id: evidencia.instancia_id,
                tipo_referencia: referencia.tipo.clone(),
                aplicada: motivo.is_none(),
                motivo_no_aplicada: motivo.map(str::to_string),
                orden_descartado: paso.orden,
                orden_evidencia: evidencia.orden,
            });
        }
    }
    salida.sort_by(|a, b| {
        a.orden_descartado
            .cmp(&b.orden_descartado)
            .then(a.orden_evidencia.cmp(&b.orden_evidencia))
            .then(a.tipo_referencia.cmp(&b.tipo_referencia))
    });
    salida
}

fn estado_tecnico(crudo: &Value) -> &'static str {
    let payload = crudo
        .pointer("/metadatos/payload_hook")
        .filter(|v| v.is_object())
        .unwrap_or(crudo);
    let hook = payload["hook_event_name"].as_str();
    let tipo = crudo["metadatos"]["tipo"].as_str();
    if hook == Some("PostToolUseFailure") || tipo == Some("herramienta-error") {
        "error"
    } else if hook == Some("PostToolUse") || tipo == Some("herramienta-ok") {
        "ok"
    } else {
        "desconocido"
    }
}

fn referencias_descarte(crudo: &Value) -> Vec<ReferenciaDescarte> {
    let mut salida = Vec::new();
    let Some(items) = crudo["referencias"].as_array() else {
        return salida;
    };
    for item in items {
        let Some(tipo) = item["tipo"].as_str() else {
            continue;
        };
        if tipo != REF_DESCARTA && tipo != REF_REVIERTE {
            continue;
        }
        let Some(clave) = item["clave_evento"].as_str() else {
            continue;
        };
        if salida
            .iter()
            .any(|r: &ReferenciaDescarte| r.tipo == tipo && r.clave_evento == clave)
        {
            continue;
        }
        salida.push(ReferenciaDescarte {
            tipo: tipo.into(),
            clave_evento: clave.into(),
        });
    }
    salida
}

fn paso_desde_fila(fila: &Value) -> Result<PasoObservado> {
    let crudo: Value =
        serde_json::from_str(fila["json_original"].as_str().context("crudo ausente")?)?;
    Ok(PasoObservado {
        orden: fila["orden"].as_i64().context("orden ausente")?,
        instancia_id: fila["instancia_id"].as_i64().context("instancia ausente")?,
        clave_evento: fila["clave_evento"]
            .as_str()
            .context("clave de evento ausente")?
            .into(),
        herramienta: fila["herramienta"]
            .as_str()
            .context("herramienta ausente")?
            .into(),
        tipo_destino: fila["tipo_destino"]
            .as_str()
            .context("destino ausente")?
            .into(),
        grupo_evidencia: fila["grupo_evidencia"]
            .as_str()
            .context("grupo ausente")?
            .into(),
        timestamp: fila["timestamp"].as_i64().context("timestamp ausente")?,
        estado_tecnico: estado_tecnico(&crudo).into(),
        referencias: referencias_descarte(&crudo),
    })
}

struct Fuente {
    origen: &'static str,
    proyecto: String,
    episodio_id: i64,
    clave_episodio: String,
    tipo_objetivo_json: String,
    verificacion: &'static str,
    normalizador_version: String,
    secuencia_original_sha256: String,
    resultado: Option<i64>,
    evidencia_resultado_instancia_id: Option<i64>,
    evidencia_identidad: String,
    corregido: bool,
    pasos: Vec<PasoObservado>,
}

fn cargar_manual(c: &Connection, proyecto: &str, episodio: &str) -> Result<Fuente> {
    let h = rows(c,
        "SELECT e.id,e.clave,t.clave AS tarea,d.definicion_version,e.criterio_id,e.criterio_version,
                e.normalizador_version,e.secuencia_sha256,e.cantidad_pasos,
                er.resultado,er.tarea_resultado_id,tr.fuente_instancia_id,
                CASE WHEN EXISTS (SELECT 1 FROM tarea_resultado c WHERE c.anterior_id=er.tarea_resultado_id)
                     THEN 1 ELSE 0 END AS corregido
         FROM episodio e JOIN tarea t ON t.id=e.tarea_id
         JOIN tarea_evento d ON d.id=e.definicion_evento_id
         LEFT JOIN episodio_resultado er ON er.episodio_id=e.id
         LEFT JOIN tarea_resultado tr ON tr.id=er.tarea_resultado_id
         WHERE e.clave=?1 AND t.proyecto=?2", params![episodio, proyecto])?
        .pop().context("episodio manual ausente en el proyecto")?;
    let id = h["id"].as_i64().context("episodio sin id")?;
    let tarea = h["tarea"].as_str().context("tarea ausente")?;
    let definicion_version = h["definicion_version"]
        .as_i64()
        .context("definicion de episodio ausente")?;
    let criterio = h["criterio_id"].as_str().context("criterio ausente")?;
    let version = h["criterio_version"].as_str().context("version ausente")?;
    let resultado = h["resultado"]
        .as_i64()
        .context("episodio manual sin resultado sellado")?;
    let pasos = rows(
        c,
        "SELECT p.orden,p.instancia_id,p.herramienta,p.tipo_destino,
                i.clave_evento,i.grupo_evidencia,i.timestamp,c.json_original
         FROM episodio_paso p JOIN instancia i ON i.id=p.instancia_id
         JOIN captura_evento c ON c.instancia_id=i.id
         WHERE p.episodio_id=?1 ORDER BY p.orden",
        [id],
    )?
    .iter()
    .map(paso_desde_fila)
    .collect::<Result<Vec<_>>>()?;
    let cantidad_pasos = h["cantidad_pasos"].as_i64().context("cantidad ausente")?;
    ensure!(
        pasos.len() as i64 == cantidad_pasos,
        "episodio manual incompleto"
    );
    let tarea_resultado_id = h["tarea_resultado_id"].as_i64();
    Ok(Fuente {
        origen: "manual_tarea",
        proyecto: proyecto.into(),
        episodio_id: id,
        clave_episodio: h["clave"].as_str().context("clave ausente")?.into(),
        tipo_objetivo_json: serde_json::to_string(&json!([
            "manual_tarea",
            tarea,
            definicion_version,
            criterio,
            version
        ]))?,
        verificacion: "criterio_tarea",
        normalizador_version: h["normalizador_version"]
            .as_str()
            .context("normalizador ausente")?
            .into(),
        secuencia_original_sha256: h["secuencia_sha256"]
            .as_str()
            .context("hash ausente")?
            .into(),
        resultado: Some(resultado),
        evidencia_resultado_instancia_id: h["fuente_instancia_id"].as_i64(),
        evidencia_identidad: match tarea_resultado_id {
            Some(id) => format!("manual_resultado:{id}"),
            None => format!("manual_episodio:{id}"),
        },
        corregido: h["corregido"].as_i64().unwrap_or(0) != 0,
        pasos,
    })
}

fn cargar_automatico(c: &Connection, proyecto: &str, episodio_id: i64) -> Result<Fuente> {
    let h = rows(
        c,
        "SELECT e.id,e.objetivo,e.normalizador_version,e.secuencia_sha256,e.cantidad_pasos,
                r.resultado,r.comprobacion_instancia_id
         FROM episodio_automatico e
         LEFT JOIN episodio_automatico_resultado r ON r.episodio_id=e.id
         WHERE e.id=?1 AND e.proyecto=?2",
        params![episodio_id, proyecto],
    )?
    .pop()
    .context("episodio automatico ausente en el proyecto")?;
    let sellado: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM episodio_automatico_sello WHERE episodio_id=?1)",
        [episodio_id],
        |r| r.get(0),
    )?;
    ensure!(sellado, "episodio automatico sin sello");
    let pasos = rows(
        c,
        "SELECT p.orden,p.instancia_id,p.herramienta,p.tipo_destino,
                i.clave_evento,i.grupo_evidencia,i.timestamp,c.json_original
         FROM episodio_automatico_paso p JOIN instancia i ON i.id=p.instancia_id
         JOIN captura_evento c ON c.instancia_id=i.id
         WHERE p.episodio_id=?1 ORDER BY p.orden",
        [episodio_id],
    )?
    .iter()
    .map(paso_desde_fila)
    .collect::<Result<Vec<_>>>()?;
    let cantidad_pasos = h["cantidad_pasos"].as_i64().context("cantidad ausente")?;
    ensure!(
        pasos.len() as i64 == cantidad_pasos,
        "episodio automatico incompleto"
    );
    let comprobacion = h["comprobacion_instancia_id"].as_i64();
    Ok(Fuente {
        origen: "automatico_lista",
        proyecto: proyecto.into(),
        episodio_id,
        clave_episodio: episodio_id.to_string(),
        tipo_objetivo_json: serde_json::to_string(&json!([
            "automatico_lista",
            h["objetivo"].as_str().context("objetivo ausente")?
        ]))?,
        verificacion: "comprobacion_tecnica",
        normalizador_version: h["normalizador_version"]
            .as_str()
            .context("normalizador ausente")?
            .into(),
        secuencia_original_sha256: h["secuencia_sha256"]
            .as_str()
            .context("hash ausente")?
            .into(),
        resultado: h["resultado"].as_i64(),
        evidencia_resultado_instancia_id: comprobacion,
        evidencia_identidad: match comprobacion {
            Some(id) => format!("automatico_comprobacion:{id}"),
            None => format!("automatico_episodio:{episodio_id}"),
        },
        corregido: false,
        pasos,
    })
}

fn hash_evidencia(fuente: &Fuente) -> Result<String> {
    let pasos: Vec<Value> = fuente
        .pasos
        .iter()
        .map(|p| {
            json!({
                "orden":p.orden,
                "instancia_id":p.instancia_id,
                "clave_evento":p.clave_evento,
                "herramienta":p.herramienta,
                "tipo_destino":p.tipo_destino,
                "grupo_evidencia":p.grupo_evidencia,
                "estado_tecnico":p.estado_tecnico,
                "referencias":p.referencias.iter()
                    .map(|r| json!({"tipo":r.tipo,"clave_evento":r.clave_evento}))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let evidencia = json!({
        "algoritmo":ALGORITMO,
        "origen":fuente.origen,
        "episodio":fuente.clave_episodio,
        "tipo_objetivo_json":fuente.tipo_objetivo_json,
        "verificacion":fuente.verificacion,
        "resultado":fuente.resultado,
        "evidencia_resultado_instancia_id":fuente.evidencia_resultado_instancia_id,
        "secuencia_original_sha256":fuente.secuencia_original_sha256,
        "pasos":pasos
    });
    Ok(digest(serde_json::to_string(&evidencia)?.as_bytes()))
}

fn construir_en_tx(tx: &Connection, fuente: &Fuente) -> Result<i64> {
    let tipo_sha = digest(fuente.tipo_objetivo_json.as_bytes());
    let descartes = calcular_descartes(&fuente.pasos, fuente.evidencia_resultado_instancia_id);
    let excluidos: BTreeSet<i64> = descartes
        .iter()
        .filter(|d| d.aplicada)
        .map(|d| d.instancia_id)
        .collect();
    let cantidad_descartados = excluidos.len();
    let decision = decidir_estado(
        fuente.resultado,
        fuente.corregido,
        fuente.pasos.len(),
        cantidad_descartados,
    );
    let evidencia_sha256 = hash_evidencia(fuente)?;
    let clave = format!("secuencia/{}/{}", fuente.origen, fuente.clave_episodio);
    if let Some(existente) = rows(
        tx,
        "SELECT id,proyecto,tipo_objetivo_json,evidencia_sha256 FROM arandu_secuencia_minima
         WHERE clave=?1 OR (origen=?2 AND episodio_id=?3)",
        params![clave, fuente.origen, fuente.episodio_id],
    )?
    .pop()
    {
        ensure!(
            existente["proyecto"] == fuente.proyecto
                && existente["tipo_objetivo_json"] == fuente.tipo_objetivo_json
                && existente["evidencia_sha256"] == evidencia_sha256,
            "conflicto de identidad de la secuencia derivada"
        );
        return existente["id"].as_i64().context("secuencia sin id");
    }
    let ultima_instancia_id = fuente
        .pasos
        .iter()
        .map(|p| p.instancia_id)
        .max()
        .unwrap_or(0);
    let ultimo_timestamp = fuente.pasos.iter().map(|p| p.timestamp).max().unwrap_or(0);
    tx.execute("INSERT INTO arandu_secuencia_minima(clave,origen,episodio_id,proyecto,tipo_objetivo_json,
            tipo_objetivo_sha256,verificacion,algoritmo,normalizador_version,resultado,estado,motivo,
            evidencia_resultado_instancia_id,evidencia_identidad,cantidad_pasos,cantidad_incluidos,
            cantidad_descartados,ultima_instancia_id,ultimo_timestamp,secuencia_original_sha256,
            evidencia_sha256,creada_en) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
        params![clave,fuente.origen,fuente.episodio_id,fuente.proyecto,fuente.tipo_objetivo_json,tipo_sha,
            fuente.verificacion,ALGORITMO,fuente.normalizador_version,fuente.resultado,decision.estado,decision.motivo,
            fuente.evidencia_resultado_instancia_id,fuente.evidencia_identidad,fuente.pasos.len() as i64,
            (fuente.pasos.len()-cantidad_descartados) as i64,cantidad_descartados as i64,ultima_instancia_id,
            ultimo_timestamp,fuente.secuencia_original_sha256,evidencia_sha256,now()?])?;
    let id = tx.last_insert_rowid();
    for paso in &fuente.pasos {
        let incluido = !excluidos.contains(&paso.instancia_id);
        tx.execute(
            "INSERT INTO arandu_secuencia_minima_paso VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                paso.orden,
                paso.instancia_id,
                &paso.herramienta,
                &paso.tipo_destino,
                &paso.estado_tecnico,
                if incluido { 1i64 } else { 0i64 },
                if incluido {
                    None::<&str>
                } else {
                    Some("descarte_explicito")
                }
            ],
        )?;
    }
    for descarte in &descartes {
        tx.execute(
            "INSERT INTO arandu_secuencia_minima_descarte VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id,
                descarte.instancia_id,
                descarte.evidencia_instancia_id,
                &descarte.tipo_referencia,
                if descarte.aplicada { 1i64 } else { 0i64 },
                descarte.motivo_no_aplicada.as_deref(),
                descarte.orden_descartado,
                descarte.orden_evidencia
            ],
        )?;
    }
    tx.execute(
        "INSERT INTO arandu_secuencia_minima_sello VALUES (?1)",
        [id],
    )?;
    Ok(id)
}

pub(crate) fn leer_id(c: &Connection, id: i64) -> Result<Value> {
    let sellada: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM arandu_secuencia_minima_sello WHERE reduccion_id=?1)",
        [id],
        |r| r.get(0),
    )?;
    ensure!(sellada, "secuencia derivada sin sello");
    let h = rows(c, "SELECT * FROM arandu_secuencia_minima WHERE id=?1", [id])?
        .pop()
        .context("secuencia derivada ausente")?;
    let tipo_objetivo: Value = serde_json::from_str(
        h["tipo_objetivo_json"]
            .as_str()
            .context("tipo de objetivo ausente")?,
    )?;
    let mut pasos = rows(c,
        "SELECT p.orden,p.instancia_id,p.herramienta,p.tipo_destino,p.estado_tecnico,
                p.incluido,p.motivo_exclusion,i.clave_evento,i.grupo_evidencia,c.secuencia,i.timestamp
         FROM arandu_secuencia_minima_paso p JOIN instancia i ON i.id=p.instancia_id
         JOIN captura_evento c ON c.instancia_id=i.id
         WHERE p.reduccion_id=?1 ORDER BY p.orden", [id])?;
    for paso in &mut pasos {
        let incluido = paso["incluido"].as_i64() == Some(1);
        paso["incluido"] = json!(incluido);
        paso["fallo_observado"] = json!(paso["estado_tecnico"] == "error");
    }
    let mut descartes = rows(c,
        "SELECT d.instancia_id,d.evidencia_instancia_id,d.tipo_referencia,d.aplicada,
                d.motivo_no_aplicada,d.orden_descartado,d.orden_evidencia,
                pi.clave_evento AS descartado_clave,ei.clave_evento AS evidencia_clave,
                pd.herramienta AS descartado_herramienta,pd.tipo_destino AS descartado_destino,
                pe.herramienta AS evidencia_herramienta,pe.tipo_destino AS evidencia_destino
         FROM arandu_secuencia_minima_descarte d
         JOIN arandu_secuencia_minima_paso pd ON pd.reduccion_id=d.reduccion_id AND pd.instancia_id=d.instancia_id
         JOIN arandu_secuencia_minima_paso pe ON pe.reduccion_id=d.reduccion_id AND pe.instancia_id=d.evidencia_instancia_id
         JOIN instancia pi ON pi.id=d.instancia_id JOIN instancia ei ON ei.id=d.evidencia_instancia_id
         WHERE d.reduccion_id=?1 ORDER BY d.orden_descartado,d.orden_evidencia,d.tipo_referencia", [id])?;
    for descarte in &mut descartes {
        descarte["aplicada"] = json!(descarte["aplicada"].as_i64() == Some(1));
    }
    let corregido = if h["origen"].as_str() == Some("manual_tarea") {
        c.query_row(
            "SELECT EXISTS(SELECT 1 FROM episodio_resultado er
             JOIN tarea_resultado c ON c.anterior_id=er.tarea_resultado_id
             WHERE er.episodio_id=?1)",
            [h["episodio_id"].as_i64().context("episodio sin id")?],
            |r| r.get::<_, bool>(0),
        )?
    } else {
        false
    };
    let estado_snapshot = h["estado"].as_str().context("estado ausente")?;
    let motivo_snapshot = h["motivo"].as_str().context("motivo ausente")?;
    let es_candidata = estado_snapshot.starts_with("candidata_");
    let (estado_actual, motivo_actual, seleccionable_actual) = if corregido {
        (
            "corregida".to_string(),
            "resultado_corregido_posteriormente".to_string(),
            false,
        )
    } else {
        (
            estado_snapshot.to_string(),
            motivo_snapshot.to_string(),
            es_candidata,
        )
    };
    let resultado_acredita = if seleccionable_actual {
        Some("episodio_fuente")
    } else {
        None
    };
    let fallos_observados = pasos
        .iter()
        .filter(|p| p["estado_tecnico"].as_str() == Some("error"))
        .count();
    let fallos_en_camino = pasos
        .iter()
        .filter(|p| {
            p["fallo_observado"].as_bool() == Some(true) && p["incluido"].as_bool() == Some(true)
        })
        .count();
    let resumen = json!({
        "estado_snapshot":h["estado"].clone(),
        "motivo_snapshot":h["motivo"].clone(),
        "resultado_snapshot":h["resultado"].clone(),
        "cantidad_pasos":h["cantidad_pasos"].clone(),
        "cantidad_incluidos":h["cantidad_incluidos"].clone(),
        "cantidad_descartados":h["cantidad_descartados"].clone(),
        "fallos_observados":fallos_observados,
        "fallos_en_camino":fallos_en_camino,
        "evidencia_resultado_instancia_id":h["evidencia_resultado_instancia_id"].clone(),
        "evidencia_identidad":h["evidencia_identidad"].clone(),
    });
    Ok(json!({
        "secuencia":h,
        "tipo_objetivo":tipo_objetivo,
        "pasos":pasos,
        "descartes":descartes,
        "estado_actual":estado_actual,
        "motivo_actual":motivo_actual,
        "seleccionable_actual":seleccionable_actual,
        "es_reejecucion":false,
        "resultado_acredita":resultado_acredita,
        "corregido_en_vivo":corregido,
        "resumen":resumen,
        "alcance":"recorrido observado del episodio; los descartes exigen referencias tipadas del mismo episodio; no es un minimo causal ni una comparacion entre episodios",
        "no_afirma":[
            "que un paso fallido sea prescindible",
            "que el orden observado sea el unico posible",
            "que una comprobacion tecnica equivalga a un criterio de tarea"
        ]
    }))
}

impl Store {
    pub fn construir_secuencia_manual(&mut self, proyecto: &str, episodio: &str) -> Result<Value> {
        nonempty(proyecto)?;
        nonempty(episodio)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fuente = cargar_manual(&tx, proyecto, episodio)?;
        let id = construir_en_tx(&tx, &fuente)?;
        let salida = leer_id(&tx, id)?;
        tx.commit()?;
        Ok(salida)
    }

    pub fn construir_secuencia_automatica(
        &mut self,
        proyecto: &str,
        episodio_id: i64,
    ) -> Result<Value> {
        nonempty(proyecto)?;
        ensure!(episodio_id > 0, "episodio automatico invalido");
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fuente = cargar_automatico(&tx, proyecto, episodio_id)?;
        let id = construir_en_tx(&tx, &fuente)?;
        let salida = leer_id(&tx, id)?;
        tx.commit()?;
        Ok(salida)
    }

    pub fn leer_secuencia(&mut self, proyecto: &str, clave: &str) -> Result<Value> {
        nonempty(proyecto)?;
        nonempty(clave)?;
        let tx = self.conn.transaction()?;
        let id = rows(
            &tx,
            "SELECT id FROM arandu_secuencia_minima WHERE clave=?1 AND proyecto=?2",
            params![clave, proyecto],
        )?
        .pop()
        .context("secuencia ausente en el proyecto")?["id"]
            .as_i64()
            .context("secuencia sin id")?;
        let salida = leer_id(&tx, id)?;
        tx.commit()?;
        Ok(salida)
    }

    pub fn listar_secuencias(&self, proyecto: &str) -> Result<Value> {
        nonempty(proyecto)?;
        let mut secuencias = rows(&self.conn,
            "SELECT r.id,r.clave,r.origen,r.episodio_id,r.tipo_objetivo_json,r.verificacion,r.algoritmo,r.resultado,
                    r.estado,r.motivo,r.cantidad_pasos,r.cantidad_incluidos,r.cantidad_descartados,
                    r.ultima_instancia_id,r.ultimo_timestamp,r.creada_en
             FROM arandu_secuencia_minima r JOIN arandu_secuencia_minima_sello s ON s.reduccion_id=r.id
             WHERE r.proyecto=?1 ORDER BY r.ultima_instancia_id,r.id", [proyecto])?;
        for secuencia in &mut secuencias {
            let corregido = if secuencia["origen"].as_str() == Some("manual_tarea") {
                self.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM episodio_resultado er
                     JOIN tarea_resultado c ON c.anterior_id=er.tarea_resultado_id
                     WHERE er.episodio_id=?1)",
                    [secuencia["episodio_id"].as_i64().unwrap_or(0)],
                    |r| r.get::<_, bool>(0),
                )?
            } else {
                false
            };
            secuencia["tipo_objetivo"] =
                serde_json::from_str(secuencia["tipo_objetivo_json"].as_str().unwrap_or("[]"))?;
            let candidata = secuencia["estado"]
                .as_str()
                .is_some_and(|estado| estado.starts_with("candidata_"));
            secuencia["corregido_en_vivo"] = json!(corregido);
            secuencia["seleccionable_actual"] = json!(!corregido && candidata);
            secuencia["es_reejecucion"] = json!(false);
        }
        Ok(json!({"proyecto":proyecto,"secuencias":secuencias}))
    }
}
