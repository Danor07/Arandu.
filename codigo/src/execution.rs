//! Enlaces explícitos sobre crudo inmutable. Ningún escritor envía solicitudes.
use crate::{now, Store};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operacion", rename_all = "snake_case", deny_unknown_fields)]
pub enum Record {
    Intento {
        intento: String,
        proyecto: String,
        problema: String,
    },
    Enlazar {
        intento: String,
        evento: String,
    },
    Exponer {
        clave: String,
        intento: String,
        solicitud: String,
        paquete: String,
        posicion: usize,
    },
    Evaluar {
        clave: String,
        intento: String,
        evidencia: String,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    rol: String,
    contenido: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Block {
    paquete: String,
    posicion: usize,
    texto: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    llamada_id: String,
    procedencia: String,
    frontera: String,
    evidencia_envio: Option<String>,
    mensajes: Vec<Message>,
    bloques: Vec<Block>,
    proveedor: Option<String>,
    modelo: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    llamada_id: String,
    procedencia: String,
    solicitud: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evaluation {
    version: u32,
    pub(crate) fuente: String,
    pub(crate) procedencia: String,
    pub(crate) criterio: String,
    pub(crate) criterio_version: String,
    pub(crate) alcance: String,
    pub(crate) objetivo: Option<String>,
    pub(crate) resultado: Option<i64>,
    pub(crate) detalle: Value,
    pub(crate) nota: String,
    pub(crate) anterior: Option<String>,
}
pub(crate) struct Raw {
    pub(crate) id: i64,
    pub(crate) value: Value,
}
pub(crate) fn raw(c: &Connection, key: &str) -> Result<Raw> {
    let (id, text): (i64,String) = c.query_row("SELECT i.id,c.json_original FROM instancia i JOIN captura_evento c ON c.instancia_id=i.id WHERE i.clave_evento=?1",[key],|r| Ok((r.get(0)?,r.get(1)?))).optional()?.with_context(|| format!("referencia pendiente: {key}"))?;
    Ok(Raw {
        id,
        value: serde_json::from_str(&text)?,
    })
}
pub(crate) fn nonempty(s: &str) -> Result<()> {
    ensure!(!s.trim().is_empty(), "identidad/campo vacio");
    Ok(())
}
pub(crate) fn provenance(s: &str) -> Result<()> {
    ensure!(
        matches!(s, "real" | "simulada"),
        "procedencia debe ser real o simulada"
    );
    Ok(())
}
pub(crate) fn project_event(r: &Raw, project: &str, group: &str) -> Result<()> {
    nonempty(project)?;
    ensure!(
        r.value["metadatos"]["proyecto"] == project && r.value["grupo_evidencia"] == group,
        "proyecto/episodio incompatible o desconocido"
    );
    Ok(())
}
fn request(r: &Raw) -> Result<Request> {
    ensure!(
        r.value["origen"] == "herramienta",
        "solicitud requiere origen herramienta/adaptador"
    );
    let q: Request = serde_json::from_value(r.value["metadatos"]["solicitud"].clone())
        .context("contrato solicitud invalido")?;
    ensure!(q.version == 1, "version solicitud invalida");
    provenance(&q.procedencia)?;
    if let Some(p) = r.value["metadatos"].get("procedencia") {
        ensure!(p == &q.procedencia, "procedencia de solicitud incompatible");
    }
    nonempty(&q.llamada_id)?;
    ensure!(
        matches!(
            q.frontera.as_str(),
            "preparada" | "envio_observado" | "incierta"
        ),
        "frontera invalida"
    );
    ensure!(!q.mensajes.is_empty(), "faltan mensajes");
    for m in &q.mensajes {
        nonempty(&m.rol)?;
    }
    Ok(q)
}
fn member(c: &Connection, execution: i64, id: i64) -> Result<()> {
    ensure!(c.query_row("SELECT EXISTS(SELECT 1 FROM ejecucion_evento WHERE ejecucion_id=?1 AND instancia_id=?2)",params![execution,id],|r|r.get::<_,bool>(0))?,"evento no enlazado al intento");
    Ok(())
}
/// Contrato de evidencia externa v1, independiente de `ejecucion`. Lo usan la
/// evaluación de tanda 3 y el resultado de tarea de tanda 5: un solo evaluador.
pub(crate) fn evaluacion_contrato(e: &Raw) -> Result<Evaluation> {
    let v: Evaluation = serde_json::from_value(e.value["metadatos"]["evaluacion"].clone())
        .context("contrato evaluacion invalido")?;
    ensure!(v.version == 1, "version evaluacion invalida");
    provenance(&v.procedencia)?;
    ensure!(
        e.value["metadatos"]["procedencia"] == v.procedencia,
        "procedencia de evidencia incompatible"
    );
    ensure!(
        e.value["metadatos"]["solicitud"].is_null() && e.value["metadatos"]["respuesta"].is_null(),
        "evidencia externa debe ser evento dedicado"
    );
    nonempty(&v.criterio)?;
    nonempty(&v.criterio_version)?;
    ensure!(
        v.resultado.is_none_or(|x| x == 0 || x == 1),
        "resultado invalido"
    );
    ensure!(
        (v.fuente == "usuario_explicito" && e.value["origen"] == "usuario")
            || (v.fuente == "verificador_externo" && e.value["origen"] == "herramienta"),
        "evidencia no externa o fuente incompatible"
    );
    ensure!(
        v.detalle.is_object() && !v.detalle.as_object().unwrap().is_empty(),
        "falta detalle de criterio"
    );
    if v.fuente == "verificador_externo" {
        for key in [
            "nombre",
            "version",
            "entrada",
            "salida",
            "regla",
            "ejecucion",
        ] {
            ensure!(
                v.detalle.get(key).is_some_and(|x| !x.is_null()),
                "falta {key} del verificador"
            );
        }
        ensure!(
            v.detalle["ejecucion"] == "real_local" || v.detalle["ejecucion"] == "simulada",
            "ejecucion del verificador invalida"
        );
        nonempty(
            v.detalle["nombre"]
                .as_str()
                .context("nombre del verificador")?,
        )?;
        nonempty(
            v.detalle["version"]
                .as_str()
                .context("version del verificador")?,
        )?;
        let pointer = v.detalle["regla"]["campo"]
            .as_str()
            .context("regla requiere campo JSON Pointer")?;
        let observed = v.detalle["salida"]
            .pointer(pointer)
            .and_then(Value::as_bool)
            .context("salida del verificador debe contener booleano observado")?;
        ensure!(
            v.detalle["regla"]["si_true"] == 1 && v.detalle["regla"]["si_false"] == 0,
            "regla v1 requiere true=1, false=0"
        );
        ensure!(
            v.resultado == Some(i64::from(observed)),
            "resultado no coincide con regla/salida"
        );
    }
    Ok(v)
}

/// La evidencia declara explicitamente a quien evalua, y el verificador copia su
/// contenido exacto. No se infiere el objetivo por cercania ni por timestamp.
pub(crate) fn evaluacion_objetivo(
    e: &Raw,
    v: &Evaluation,
    clave_evaluada: &str,
    evaluado: &Value,
) -> Result<()> {
    ensure!(
        e.value["referencias"].as_array().is_some_and(|refs| refs
            .iter()
            .any(|r| r["tipo"] == "evalua" && r["clave_evento"] == clave_evaluada)),
        "falta referencia evalua al objetivo"
    );
    if v.fuente == "verificador_externo" {
        ensure!(
            v.detalle["entrada"]["objetivo"] == clave_evaluada
                && v.detalle["entrada"]["contenido"] == evaluado["contenido"],
            "entrada del verificador no coincide con objetivo"
        );
    }
    Ok(())
}

pub(crate) fn receipt(id: i64, date: i64, table: &str) -> Value {
    json!({"entidad":table,"id":id,"registrado_en":date})
}
// Nombres SQL provienen exclusivamente de las variantes internas, nunca del usuario.
pub(crate) fn existing(
    c: &Connection,
    table: &str,
    column: &str,
    key: &str,
    contract: &str,
) -> Result<Option<Value>> {
    let (idcol, datecol) = if table == "ejecucion_evento" {
        ("instancia_id", "enlazado_en")
    } else {
        ("id", "registrado_en")
    };
    existing_cols(c, table, idcol, datecol, column, key, contract)
}

/// Igual que `existing`, con los nombres de columna explicitos. Provienen
/// exclusivamente de variantes internas, nunca de datos del usuario.
pub(crate) fn existing_cols(
    c: &Connection,
    table: &str,
    idcol: &str,
    datecol: &str,
    column: &str,
    key: &str,
    contract: &str,
) -> Result<Option<Value>> {
    let row: Option<(i64, i64, String)> = c
        .query_row(
            &format!("SELECT {idcol},{datecol},contrato_json FROM {table} WHERE {column}=?1"),
            [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    row.map(|(id, date, old)| {
        ensure!(old == contract, "conflicto de identidad: {key}");
        Ok(receipt(id, date, table))
    })
    .transpose()
}

impl Store {
    pub fn record_execution(&mut self, record: &Record) -> Result<Value> {
        let contract = serde_json::to_string(record)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let date = now()?;
        if let Record::Intento {
            intento,
            proyecto,
            problema,
        } = record
        {
            nonempty(intento)?;
            nonempty(proyecto)?;
            if let Some(old) = existing(&tx, "ejecucion", "intento_clave", intento, &contract)? {
                tx.commit()?;
                return Ok(old);
            }
            let p = raw(&tx, problema)?;
            ensure!(p.value["rol"] == "problema", "se requiere rol problema");
            let group = p.value["grupo_evidencia"]
                .as_str()
                .context("episodio ausente")?;
            project_event(&p, proyecto, group)?;
            // El problema puede ser compartido por varios intentos; no se lo asigna
            // automáticamente a la relación exclusiva ejecucion_evento.
            tx.execute("INSERT INTO ejecucion(intento_clave,proyecto,problema_instancia_id,grupo_evidencia,registrado_en,contrato_json) VALUES (?1,?2,?3,?4,?5,?6)",params![intento,proyecto,p.id,group,date,contract])?;
            let result = receipt(tx.last_insert_rowid(), date, "ejecucion");
            tx.commit()?;
            return Ok(result);
        }
        let intento = match record {
            Record::Enlazar { intento, .. }
            | Record::Exponer { intento, .. }
            | Record::Evaluar { intento, .. } => intento,
            _ => unreachable!(),
        };
        let (execution,project,problem,group):(i64,String,i64,String)=tx.query_row("SELECT id,proyecto,problema_instancia_id,grupo_evidencia FROM ejecucion WHERE intento_clave=?1",[intento],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?.context("intento ausente")?;
        let (table, id) = match record {
            Record::Enlazar { evento, .. } => {
                let e = raw(&tx, evento)?;
                if let Some(old) = existing(
                    &tx,
                    "ejecucion_evento",
                    "instancia_id",
                    &e.id.to_string(),
                    &contract,
                )? {
                    tx.commit()?;
                    return Ok(old);
                }
                project_event(&e, &project, &group)?;
                if let Some(usage) = e.value.get("uso").filter(|x| !x.is_null()) {
                    ensure!(
                        usage["intento_clave"] == *intento,
                        "intento de uso incompatible"
                    );
                }
                if let Some(refs) = e.value["referencias"].as_array() {
                    for reference in refs {
                        let target = raw(
                            &tx,
                            reference["clave_evento"]
                                .as_str()
                                .context("referencia invalida")?,
                        )?;
                        project_event(&target, &project, &group)?;
                        if target.id != problem {
                            member(&tx, execution, target.id)?;
                        }
                    }
                }
                if !e.value["metadatos"]["solicitud"].is_null() {
                    let q = request(&e)?;
                    let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM ejecucion_evento ee JOIN instancia i ON i.id=ee.instancia_id WHERE json_extract(i.metadatos_json,'$.solicitud.llamada_id')=?1)",[&q.llamada_id],|r|r.get(0))?;
                    ensure!(
                        !duplicate,
                        "llamada ya enlazada; reusar evento original o nueva identidad de llamada"
                    );
                }
                if !e.value["metadatos"]["respuesta"].is_null() {
                    let r: Response =
                        serde_json::from_value(e.value["metadatos"]["respuesta"].clone())?;
                    ensure!(r.version == 1, "version respuesta invalida");
                    provenance(&r.procedencia)?;
                    if let Some(p) = e.value["metadatos"].get("procedencia") {
                        ensure!(p == &r.procedencia, "procedencia de respuesta incompatible");
                    }
                    let target = raw(&tx, &r.solicitud)?;
                    member(&tx, execution, target.id)?;
                    let q = request(&target)?;
                    ensure!(
                        r.llamada_id == q.llamada_id && r.procedencia == q.procedencia,
                        "llamada/procedencia incompatible"
                    );
                    ensure!(
                        e.value["referencias"].as_array().is_some_and(|refs| {
                            refs.iter().any(|x| {
                                x["tipo"] == "responde_a" && x["clave_evento"] == r.solicitud
                            })
                        }),
                        "falta referencia responde_a"
                    );
                }
                tx.execute(
                    "INSERT INTO ejecucion_evento VALUES (?1,?2,?3,?4)",
                    params![e.id, execution, date, contract],
                )?;
                ("ejecucion_evento", e.id)
            }
            Record::Exponer {
                clave,
                solicitud,
                paquete,
                posicion,
                ..
            } => {
                nonempty(clave)?;
                if let Some(old) = existing(&tx, "exposicion_memoria", "clave", clave, &contract)? {
                    tx.commit()?;
                    return Ok(old);
                }
                let e = raw(&tx, solicitud)?;
                member(&tx, execution, e.id)?;
                let q = request(&e)?;
                ensure!(
                    q.frontera == "envio_observado"
                        && q.evidencia_envio
                            .as_ref()
                            .is_some_and(|s| !s.trim().is_empty()),
                    "falta evidencia explicita de envio observado"
                );
                let (packet,p,text):(i64,i64,String)=tx.query_row("SELECT p.evento_id,r.problema_instancia_id,p.texto_memoria FROM paquete_recuperacion p JOIN evento_recuperacion r ON r.id=p.evento_id WHERE r.clave_evento=?1 AND p.proyecto=?2",params![paquete,project],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?.context("paquete ausente/proyecto incompatible")?;
                ensure!(p == problem, "problema de paquete incompatible");
                ensure!(!text.is_empty(), "paquete vacio no es exposicion");
                let blocks: Vec<_> = q
                    .bloques
                    .iter()
                    .filter(|b| b.paquete == *paquete || b.posicion == *posicion)
                    .collect();
                ensure!(
                    blocks.len() == 1
                        && blocks[0].paquete == *paquete
                        && blocks[0].posicion == *posicion
                        && blocks[0].texto == text,
                    "bloque/posicion/texto incompatible"
                );
                ensure!(
                    q.mensajes
                        .get(*posicion)
                        .is_some_and(|m| m.contenido == text),
                    "mensaje no contiene paquete integro en posicion declarada"
                );
                tx.execute("INSERT INTO exposicion_memoria(clave,ejecucion_id,paquete_id,solicitud_instancia_id,llamada_id,posicion,texto_memoria,procedencia,registrado_en,contrato_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![clave,execution,packet,e.id,q.llamada_id,i64::try_from(*posicion)?,text,q.procedencia,date,contract])?;
                ("exposicion_memoria", tx.last_insert_rowid())
            }
            Record::Evaluar {
                clave, evidencia, ..
            } => {
                nonempty(clave)?;
                if let Some(old) = existing(&tx, "evaluacion_ejecucion", "clave", clave, &contract)?
                {
                    tx.commit()?;
                    return Ok(old);
                }
                let e = raw(&tx, evidencia)?;
                // Enlace y evaluación se confirman juntos; sirve también para evidencia
                // previamente enlazada por el comando explícito.
                project_event(&e, &project, &group)?;
                if let Some(u) = e.value.get("uso").filter(|v| !v.is_null()) {
                    ensure!(
                        u["intento_clave"] == *intento,
                        "intento de evidencia incompatible"
                    );
                }
                let v = evaluacion_contrato(&e)?;
                if let Some(refs) = e.value["referencias"].as_array() {
                    for reference in refs {
                        let t = raw(
                            &tx,
                            reference["clave_evento"]
                                .as_str()
                                .context("referencia invalida")?,
                        )?;
                        project_event(&t, &project, &group)?;
                        if t.id != problem {
                            member(&tx, execution, t.id)?;
                        }
                    }
                }
                let target = if let Some(key) = &v.objetivo {
                    let t = raw(&tx, key)?;
                    member(&tx, execution, t.id)?;
                    let target_prov = t.value["metadatos"]["respuesta"]["procedencia"]
                        .as_str()
                        .or(t.value["metadatos"]["solicitud"]["procedencia"].as_str())
                        .or(t.value["metadatos"]["procedencia"].as_str())
                        .context("procedencia del objetivo desconocida")?;
                    ensure!(
                        target_prov == v.procedencia,
                        "procedencia de evidencia/objetivo incompatible"
                    );
                    Some(t.id)
                } else {
                    None
                };
                ensure!(
                    (v.alcance == "evento" && target.is_some())
                        || (v.alcance == "intento" && target.is_none()),
                    "alcance/objetivo incompatible"
                );
                let evaluated_id = target.unwrap_or(problem);
                let (evaluated_key,evaluated_raw):(String,String) = tx.query_row("SELECT i.clave_evento,c.json_original FROM instancia i JOIN captura_evento c ON c.instancia_id=i.id WHERE i.id=?1",[evaluated_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
                let evaluated: Value = serde_json::from_str(&evaluated_raw)?;
                if target.is_none() {
                    ensure!(
                        evaluated["metadatos"]["procedencia"] == v.procedencia,
                        "procedencia del problema/criterio incompatible"
                    );
                }
                evaluacion_objetivo(&e, &v, &evaluated_key, &evaluated)?;
                let previous = if let Some(key) = &v.anterior {
                    let row:(i64,i64,String,String,String,Option<i64>)=tx.query_row("SELECT id,ejecucion_id,criterio_clave,criterio_version,alcance,objetivo_instancia_id FROM evaluacion_ejecucion WHERE clave=?1",[key],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?.context("evaluacion anterior pendiente")?;
                    ensure!(
                        row.1 == execution
                            && row.2 == v.criterio
                            && row.3 == v.criterio_version
                            && row.4 == v.alcance
                            && row.5 == target,
                        "correccion incompatible"
                    );
                    Some(row.0)
                } else {
                    None
                };
                let linked: Option<i64> = tx
                    .query_row(
                        "SELECT ejecucion_id FROM ejecucion_evento WHERE instancia_id=?1",
                        [e.id],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(id) = linked {
                    ensure!(id == execution, "evidencia pertenece a otro intento");
                } else {
                    let link = serde_json::to_string(&Record::Enlazar {
                        intento: intento.clone(),
                        evento: evidencia.clone(),
                    })?;
                    tx.execute(
                        "INSERT INTO ejecucion_evento VALUES (?1,?2,?3,?4)",
                        params![e.id, execution, date, link],
                    )?;
                }
                tx.execute("INSERT INTO evaluacion_ejecucion(clave,ejecucion_id,fuente_instancia_id,objetivo_instancia_id,criterio_clave,criterio_version,alcance,fuente,resultado,detalle_json,nota,procedencia,anterior_id,registrado_en,contrato_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![clave,execution,e.id,target,v.criterio,v.criterio_version,v.alcance,v.fuente,v.resultado,v.detalle.to_string(),v.nota,v.procedencia,previous,date,contract])?;
                ("evaluacion_ejecucion", tx.last_insert_rowid())
            }
            _ => unreachable!(),
        };
        let output = receipt(id, date, table);
        tx.commit()?;
        Ok(output)
    }
}

// Conversión de filas de auditoría, sin ocultar columnas ni fabricar métricas.
pub(crate) fn rows(c: &Connection, sql: &str, values: impl rusqlite::Params) -> Result<Vec<Value>> {
    let mut statement = c.prepare(sql)?;
    let names: Vec<String> = statement
        .column_names()
        .iter()
        .map(|x| x.to_string())
        .collect();
    let mapped = statement.query_map(values, |row| {
        let mut value = serde_json::Map::new();
        for (index, name) in names.iter().enumerate() {
            use rusqlite::types::ValueRef;
            let item = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => json!(n),
                ValueRef::Real(n) => json!(n),
                ValueRef::Text(s) => Value::String(String::from_utf8_lossy(s).into_owned()),
                ValueRef::Blob(_) => Value::Null,
            };
            value.insert(name.clone(), item);
        }
        Ok(Value::Object(value))
    })?;
    Ok(mapped.collect::<rusqlite::Result<Vec<_>>>()?)
}
impl Store {
    /// Consulta de exposición observada, independiente de la selección del paquete.
    pub fn exposures(
        &self,
        project: &str,
        call: Option<&str>,
        packet: Option<&str>,
    ) -> Result<Value> {
        nonempty(project)?;
        Ok(json!({"proyecto":project,"exposiciones":rows(&self.conn,
            "SELECT x.*,r.clave_evento AS clave_paquete,c.json_original AS solicitud_json_original FROM exposicion_memoria x JOIN ejecucion e ON e.id=x.ejecucion_id JOIN evento_recuperacion r ON r.id=x.paquete_id JOIN captura_evento c ON c.instancia_id=x.solicitud_instancia_id WHERE e.proyecto=?1 AND (?2 IS NULL OR x.llamada_id=?2) AND (?3 IS NULL OR r.clave_evento=?3) ORDER BY x.solicitud_instancia_id,x.posicion",
            params![project,call,packet])?}))
    }
    pub fn unexposed_packets(&self, project: &str) -> Result<Value> {
        nonempty(project)?;
        Ok(
            json!({"proyecto":project,"estado":"sin_exposicion_registrada","paquetes":rows(&self.conn,"SELECT r.clave_evento,p.* FROM paquete_recuperacion p JOIN evento_recuperacion r ON r.id=p.evento_id WHERE p.proyecto=?1 AND NOT EXISTS(SELECT 1 FROM exposicion_memoria e WHERE e.paquete_id=p.evento_id) ORDER BY r.id",[project])?}),
        )
    }
    pub fn execution(&mut self, project: &str, attempt: &str) -> Result<Value> {
        nonempty(project)?;
        let tx = self.conn.transaction()?;
        let execution = rows(
            &tx,
            "SELECT * FROM ejecucion WHERE proyecto=?1 AND intento_clave=?2",
            params![project, attempt],
        )?
        .pop()
        .context("intento ausente/proyecto incompatible")?;
        let id = execution["id"].as_i64().unwrap();
        let problem = rows(&tx,"SELECT i.id,c.json_original FROM instancia i JOIN captura_evento c ON c.instancia_id=i.id WHERE i.id=?1",[execution["problema_instancia_id"].as_i64().unwrap()])?.pop().context("problema ausente")?;
        let group = execution["grupo_evidencia"].as_str().unwrap();
        let mut events=rows(&tx,"SELECT i.id,i.clave_evento,i.timestamp,c.secuencia,c.recibido_en,c.json_original,ee.ejecucion_id,ee.enlazado_en FROM instancia i JOIN captura_evento c ON c.instancia_id=i.id JOIN instancia_proyecto p ON p.instancia_id=i.id LEFT JOIN ejecucion_evento ee ON ee.instancia_id=i.id WHERE p.proyecto=?1 AND i.grupo_evidencia=?2 ORDER BY c.secuencia,i.id",params![project,group])?;
        let mut linked = Vec::new();
        let mut pending = Vec::new();
        let mut calls = Vec::new();
        for event in &mut events {
            let raw: Value = serde_json::from_str(event["json_original"].as_str().unwrap())?;
            let mut references = Vec::new();
            if let Some(refs) = raw["referencias"].as_array() {
                for r in refs {
                    let target=rows(&tx,"SELECT i.id,p.proyecto,i.grupo_evidencia,ee.ejecucion_id FROM instancia i JOIN instancia_proyecto p ON p.instancia_id=i.id LEFT JOIN ejecucion_evento ee ON ee.instancia_id=i.id WHERE i.clave_evento=?1",[r["clave_evento"].as_str().unwrap()])?.pop();
                    references.push(
                        json!({"referencia":r,"disponible":target.is_some(),"destino":target}),
                    );
                }
            }
            event["referencias"] = json!(references);
            event["evento"] = raw.clone();
            if event["ejecucion_id"] == id {
                if let Some(q) = raw["metadatos"].get("solicitud") {
                    let responses=rows(&tx,"SELECT i.id,i.clave_evento FROM ejecucion_evento ee JOIN instancia i ON i.id=ee.instancia_id WHERE ee.ejecucion_id=?1 AND json_extract(i.metadatos_json,'$.respuesta.solicitud')=?2 ORDER BY i.id",params![id,event["clave_evento"].as_str().unwrap()])?;
                    calls.push(json!({"solicitud":event["clave_evento"],"llamada_id":q["llamada_id"],"procedencia":q["procedencia"],"frontera":q["frontera"],"estado_respuesta":if responses.is_empty(){"desconocida"}else{"respuesta_enlazada"},"respuestas":responses}));
                }
                linked.push(event.clone());
            } else if event["ejecucion_id"].is_null() {
                pending.push(event.clone());
            }
        }
        let mut evaluations = rows(
            &tx,
            "SELECT * FROM evaluacion_ejecucion WHERE ejecucion_id=?1 ORDER BY id",
            [id],
        )?;
        let mut groups: std::collections::BTreeMap<String, Value> =
            std::collections::BTreeMap::new();
        for i in 0..evaluations.len() {
            let leaf = !evaluations
                .iter()
                .any(|v| v["anterior_id"] == evaluations[i]["id"]);
            evaluations[i]["vigente"] = json!(leaf);
            if !leaf {
                continue;
            }
            let mut known = &evaluations[i];
            while known["resultado"].is_null() && !known["anterior_id"].is_null() {
                known = evaluations
                    .iter()
                    .find(|v| v["id"] == known["anterior_id"])
                    .context("cadena incompleta")?;
            }
            let row = &evaluations[i];
            let key = json!([
                row["criterio_clave"],
                row["criterio_version"],
                row["alcance"],
                row["objetivo_instancia_id"]
            ])
            .to_string();
            let g=groups.entry(key).or_insert_with(||json!({"criterio":row["criterio_clave"],"version":row["criterio_version"],"alcance":row["alcance"],"objetivo_instancia_id":row["objetivo_instancia_id"],"hojas":[],"resultados_conocidos":[]}));
            g["hojas"].as_array_mut().unwrap().push(row["id"].clone());
            if !known["resultado"].is_null()
                && !g["resultados_conocidos"]
                    .as_array()
                    .unwrap()
                    .contains(&known["resultado"])
            {
                g["resultados_conocidos"]
                    .as_array_mut()
                    .unwrap()
                    .push(known["resultado"].clone());
            }
        }
        for g in groups.values_mut() {
            let n = g["resultados_conocidos"].as_array().unwrap().len();
            g["estado"] = json!(match n {
                0 => "desconocido",
                1 => "resultado_bajo_criterio",
                _ => "conflicto",
            });
        }
        let exposures=rows(&tx,"SELECT e.*,r.clave_evento AS clave_paquete FROM exposicion_memoria e JOIN evento_recuperacion r ON r.id=e.paquete_id WHERE e.ejecucion_id=?1 ORDER BY e.solicitud_instancia_id,e.posicion",[id])?;
        let result = json!({"ejecucion":execution,"problema":problem,"eventos":linked,"eventos_del_episodio_sin_asignar":pending,"llamadas":calls,"exposiciones":exposures,"evaluaciones":evaluations,"criterios":groups.into_values().collect::<Vec<_>>(),"estado_evaluacion":if evaluations.is_empty(){"desconocido"}else{"ver_criterios"},"atribucion_causal":"no_evaluada"});
        tx.commit()?;
        Ok(result)
    }
}
