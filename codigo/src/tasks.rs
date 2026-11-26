//! Tareas explícitas con criterios versionados, resultados externos e informe.
//! Una tarea agrupa observaciones de varios turnos y chats: eso significa
//! «capturado durante esta tarea», nunca causalidad. No hay juez LLM.
use crate::execution::{
    evaluacion_contrato, evaluacion_objetivo, existing_cols, nonempty, project_event, raw, receipt,
    rows,
};
use crate::{now, Store};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Un criterio de aceptación: identidad, versión y si es obligatorio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Criterio {
    pub id: String,
    pub version: String,
    pub descripcion: String,
    pub obligatorio: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "operacion", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskRecord {
    TareaCrear {
        tarea: String,
        proyecto: String,
        definicion: String,
        descripcion: String,
        criterios: Vec<Criterio>,
    },
    /// Revisar agrega una versión; los resultados anteriores no cambian.
    TareaRevisar {
        tarea: String,
        definicion: String,
        descripcion: String,
        criterios: Vec<Criterio>,
    },
    /// Marcador explícito del usuario. El modelo no clasifica estos hechos.
    TareaMarcar {
        clave: String,
        tarea: String,
        tipo: String,
        detalle: Value,
    },
    /// Repara una asociación anterior sin tocar el crudo ni suponer cercanía.
    TareaEnlazar {
        tarea: String,
        evento: String,
        evidencia: String,
    },
    /// Resultado externo con evidencia dedicada, contrato de tanda 3.
    TareaResultado {
        clave: String,
        tarea: String,
        criterio: String,
        criterio_version: String,
        evidencia: String,
        alcance: String,
        objetivo: Option<String>,
        anterior: Option<String>,
    },
    /// Referencia una `evaluacion_ejecucion` ya registrada: es la misma
    /// observación, no un voto adicional.
    TareaResultadoDeEjecucion {
        clave: String,
        tarea: String,
        criterio: String,
        criterio_version: String,
        evaluacion: String,
        anterior: Option<String>,
    },
}

const MARCADORES: &[&str] = &[
    "inicio",
    "pausa",
    "reanudacion",
    "cierre",
    "ajuste",
    "cambio_requisitos",
];

fn criterios(lista: &[Criterio]) -> Result<()> {
    ensure!(
        !lista.is_empty(),
        "una definicion exige al menos un criterio"
    );
    let mut vistos = BTreeSet::new();
    for c in lista {
        nonempty(&c.id)?;
        nonempty(&c.version)?;
        nonempty(&c.descripcion)?;
        ensure!(vistos.insert(&c.id), "criterio repetido: {}", c.id);
    }
    Ok(())
}

/// Tarea, proyecto y versión vigente de su definición.
struct Tarea {
    id: i64,
    proyecto: String,
    version: i64,
}

fn tarea(c: &Connection, clave: &str) -> Result<Tarea> {
    let (id, proyecto): (i64, String) = c
        .query_row(
            "SELECT id,proyecto FROM tarea WHERE clave=?1",
            [clave],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .with_context(|| format!("tarea desconocida: {clave}"))?;
    let version: i64 = c.query_row(
        "SELECT coalesce(max(definicion_version),0) FROM tarea_evento WHERE tarea_id=?1 AND tipo='definicion'",
        [id],
        |r| r.get(0),
    )?;
    ensure!(version > 0, "tarea sin definicion: {clave}");
    Ok(Tarea {
        id,
        proyecto,
        version,
    })
}

/// El criterio citado debe existir en la definición vigente con esa versión.
fn criterio_vigente(c: &Connection, t: &Tarea, id: &str, version: &str) -> Result<Criterio> {
    let texto: String = c.query_row(
        "SELECT criterios_json FROM tarea_evento WHERE tarea_id=?1 AND tipo='definicion' AND definicion_version=?2",
        params![t.id, t.version],
        |r| r.get(0),
    )?;
    let lista: Vec<Criterio> = serde_json::from_str(&texto)?;
    lista
        .into_iter()
        .find(|c| c.id == id && c.version == version)
        .with_context(|| format!("criterio {id}/{version} ausente de la definicion vigente"))
}

/// Instantánea completa de descripción y criterios que abre una versión.
struct Definicion<'a> {
    tarea_id: i64,
    clave: &'a str,
    version: i64,
    descripcion: &'a str,
    criterios: &'a [Criterio],
    contrato: &'a str,
    date: i64,
}

fn definicion(tx: &rusqlite::Transaction<'_>, d: Definicion<'_>) -> Result<i64> {
    let Definicion {
        tarea_id,
        clave,
        version,
        descripcion,
        criterios: lista,
        contrato,
        date,
    } = d;
    nonempty(clave)?;
    nonempty(descripcion)?;
    criterios(lista)?;
    tx.execute(
        "INSERT INTO tarea_evento(clave,tarea_id,tipo,definicion_version,descripcion,criterios_json,detalle_json,fuente,registrado_en,contrato_json)
         VALUES (?1,?2,'definicion',?3,?4,?5,?6,'usuario_explicito',?7,?8)",
        params![
            clave,
            tarea_id,
            version,
            descripcion,
            serde_json::to_string(lista)?,
            json!({ "version": version }).to_string(),
            date,
            contrato
        ],
    )?;
    Ok(tx.last_insert_rowid())
}

impl Store {
    /// Escritor único de tareas. Confirma sólo después del commit y la igualdad
    /// del contrato hace idempotente cualquier reintento.
    pub fn record_task(&mut self, record: &TaskRecord) -> Result<Value> {
        let contrato = serde_json::to_string(record)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let date = now()?;
        let salida = match record {
            TaskRecord::TareaCrear {
                tarea: clave,
                proyecto,
                definicion: def,
                descripcion,
                criterios: lista,
            } => {
                nonempty(clave)?;
                nonempty(proyecto)?;
                if let Some(old) =
                    existing_cols(&tx, "tarea", "id", "creada_en", "clave", clave, &contrato)?
                {
                    tx.commit()?;
                    return Ok(old);
                }
                tx.execute(
                    "INSERT INTO tarea(clave,proyecto,creada_en,contrato_json) VALUES (?1,?2,?3,?4)",
                    params![clave, proyecto, date, contrato],
                )?;
                let id = tx.last_insert_rowid();
                definicion(
                    &tx,
                    Definicion {
                        tarea_id: id,
                        clave: def,
                        version: 1,
                        descripcion,
                        criterios: lista,
                        contrato: &contrato,
                        date,
                    },
                )?;
                receipt(id, date, "tarea")
            }
            TaskRecord::TareaRevisar {
                tarea: clave,
                definicion: def,
                descripcion,
                criterios: lista,
            } => {
                if let Some(old) = existing_cols(
                    &tx,
                    "tarea_evento",
                    "id",
                    "registrado_en",
                    "clave",
                    def,
                    &contrato,
                )? {
                    tx.commit()?;
                    return Ok(old);
                }
                let t = tarea(&tx, clave)?;
                let id = definicion(
                    &tx,
                    Definicion {
                        tarea_id: t.id,
                        clave: def,
                        version: t.version + 1,
                        descripcion,
                        criterios: lista,
                        contrato: &contrato,
                        date,
                    },
                )?;
                receipt(id, date, "tarea_evento")
            }
            TaskRecord::TareaMarcar {
                clave,
                tarea: clave_tarea,
                tipo,
                detalle,
            } => {
                nonempty(clave)?;
                ensure!(
                    MARCADORES.contains(&tipo.as_str()),
                    "marcador invalido: {tipo}"
                );
                ensure!(detalle.is_object(), "detalle debe ser un objeto");
                if let Some(old) = existing_cols(
                    &tx,
                    "tarea_evento",
                    "id",
                    "registrado_en",
                    "clave",
                    clave,
                    &contrato,
                )? {
                    tx.commit()?;
                    return Ok(old);
                }
                let t = tarea(&tx, clave_tarea)?;
                tx.execute(
                    "INSERT INTO tarea_evento(clave,tarea_id,tipo,definicion_version,detalle_json,fuente,registrado_en,contrato_json)
                     VALUES (?1,?2,?3,?4,?5,'usuario_explicito',?6,?7)",
                    params![clave, t.id, tipo, t.version, detalle.to_string(), date, contrato],
                )?;
                receipt(tx.last_insert_rowid(), date, "tarea_evento")
            }
            TaskRecord::TareaEnlazar {
                tarea: clave_tarea,
                evento,
                evidencia,
            } => {
                nonempty(evidencia)?;
                let e = raw(&tx, evento)?;
                if let Some(old) = existing_cols(
                    &tx,
                    "tarea_vinculo",
                    "instancia_id",
                    "vinculado_en",
                    "instancia_id",
                    &e.id.to_string(),
                    &contrato,
                )? {
                    tx.commit()?;
                    return Ok(old);
                }
                let t = tarea(&tx, clave_tarea)?;
                let grupo = e.value["grupo_evidencia"]
                    .as_str()
                    .context("episodio ausente")?;
                project_event(&e, &t.proyecto, grupo)?;
                tx.execute(
                    "INSERT INTO tarea_vinculo(instancia_id,tarea_id,definicion_version,metodo,evidencia,vinculado_en,contrato_json)
                     VALUES (?1,?2,?3,'enlace_manual',?4,?5,?6)",
                    params![e.id, t.id, t.version, evidencia, date, contrato],
                )?;
                receipt(e.id, date, "tarea_vinculo")
            }
            TaskRecord::TareaResultado {
                clave,
                tarea: clave_tarea,
                criterio,
                criterio_version,
                evidencia,
                alcance,
                objetivo,
                anterior,
            } => {
                nonempty(clave)?;
                if let Some(old) = existing_cols(
                    &tx,
                    "tarea_resultado",
                    "id",
                    "registrado_en",
                    "clave",
                    clave,
                    &contrato,
                )? {
                    tx.commit()?;
                    return Ok(old);
                }
                let t = tarea(&tx, clave_tarea)?;
                criterio_vigente(&tx, &t, criterio, criterio_version)?;
                let e = raw(&tx, evidencia)?;
                let grupo = e.value["grupo_evidencia"]
                    .as_str()
                    .context("episodio ausente")?;
                project_event(&e, &t.proyecto, grupo)?;
                // Mismo contrato de evidencia externa de tanda 3, sin duplicarlo.
                let v = evaluacion_contrato(&e)?;
                ensure!(
                    v.criterio == *criterio && v.criterio_version == *criterio_version,
                    "la evidencia declara otro criterio"
                );
                ensure!(v.alcance == *alcance, "la evidencia declara otro alcance");
                let objetivo_id = match alcance.as_str() {
                    "tarea" => {
                        ensure!(
                            objetivo.is_none() && v.objetivo.is_none(),
                            "alcance tarea no lleva objetivo"
                        );
                        // La evidencia nombra la tarea que evalúa: no se infiere.
                        ensure!(
                            v.detalle["tarea"] == *clave_tarea,
                            "la evidencia no declara detalle.tarea con esta tarea"
                        );
                        None
                    }
                    "evento" => {
                        let key = objetivo
                            .as_deref()
                            .context("alcance evento exige objetivo")?;
                        ensure!(
                            v.objetivo.as_deref() == Some(key),
                            "objetivo incompatible con la evidencia"
                        );
                        let target = raw(&tx, key)?;
                        vinculado(&tx, t.id, target.id)?;
                        evaluacion_objetivo(&e, &v, key, &target.value)?;
                        Some(target.id)
                    }
                    otro => anyhow::bail!("alcance invalido: {otro}"),
                };
                let previo = correccion(
                    &tx,
                    anterior.as_deref(),
                    &t,
                    criterio,
                    criterio_version,
                    alcance,
                    objetivo_id,
                )?;
                tx.execute(
                    "INSERT INTO tarea_resultado(clave,tarea_id,definicion_version,criterio_id,criterio_version,
                       fuente_instancia_id,objetivo_instancia_id,alcance,fuente,resultado,detalle_json,
                       evaluacion_ejecucion_id,anterior_id,registrado_en,contrato_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,?12,?13,?14)",
                    params![clave, t.id, t.version, criterio, criterio_version, e.id, objetivo_id,
                            alcance, v.fuente, v.resultado, v.detalle.to_string(), previo, date, contrato],
                )?;
                receipt(tx.last_insert_rowid(), date, "tarea_resultado")
            }
            TaskRecord::TareaResultadoDeEjecucion {
                clave,
                tarea: clave_tarea,
                criterio,
                criterio_version,
                evaluacion,
                anterior,
            } => {
                nonempty(clave)?;
                if let Some(old) = existing_cols(
                    &tx,
                    "tarea_resultado",
                    "id",
                    "registrado_en",
                    "clave",
                    clave,
                    &contrato,
                )? {
                    tx.commit()?;
                    return Ok(old);
                }
                let t = tarea(&tx, clave_tarea)?;
                criterio_vigente(&tx, &t, criterio, criterio_version)?;
                let (eval_id, fuente_id, objetivo_id, alcance_eval, fuente, resultado, detalle, proyecto):
                    (i64, i64, Option<i64>, String, String, Option<i64>, String, String) = tx
                    .query_row(
                        "SELECT v.id,v.fuente_instancia_id,v.objetivo_instancia_id,v.alcance,v.fuente,
                                v.resultado,v.detalle_json,e.proyecto
                         FROM evaluacion_ejecucion v JOIN ejecucion e ON e.id=v.ejecucion_id WHERE v.clave=?1",
                        [evaluacion],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
                    )
                    .optional()?
                    .with_context(|| format!("evaluacion de ejecucion ausente: {evaluacion}"))?;
                ensure!(
                    proyecto == t.proyecto,
                    "la evaluacion pertenece a otro proyecto"
                );
                // La evidencia de esa evaluación tiene que pertenecer a la tarea.
                vinculado(&tx, t.id, fuente_id)?;
                // `alcance` de tanda 3 es intento/evento; acá se traduce sin votar otra vez.
                let alcance = if objetivo_id.is_some() {
                    "evento"
                } else {
                    "tarea"
                };
                ensure!(
                    (alcance_eval == "evento") == objetivo_id.is_some(),
                    "alcance/objetivo incoherentes en la evaluacion referida"
                );
                if let Some(id) = objetivo_id {
                    vinculado(&tx, t.id, id)?;
                }
                let previo = correccion(
                    &tx,
                    anterior.as_deref(),
                    &t,
                    criterio,
                    criterio_version,
                    alcance,
                    objetivo_id,
                )?;
                tx.execute(
                    "INSERT INTO tarea_resultado(clave,tarea_id,definicion_version,criterio_id,criterio_version,
                       fuente_instancia_id,objetivo_instancia_id,alcance,fuente,resultado,detalle_json,
                       evaluacion_ejecucion_id,anterior_id,registrado_en,contrato_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                    params![clave, t.id, t.version, criterio, criterio_version, fuente_id, objetivo_id,
                            alcance, fuente, resultado, detalle, eval_id, previo, date, contrato],
                )?;
                receipt(tx.last_insert_rowid(), date, "tarea_resultado")
            }
        };
        tx.commit()?;
        Ok(salida)
    }

    /// Asociación por selección congelada. La llama el adaptador **después** del
    /// commit del crudo: si falla, el evento queda capturado y pendiente.
    pub fn link_task_selection(
        &mut self,
        instancia_id: i64,
        clave_tarea: &str,
        version: i64,
        sesion: &str,
        intervalo: i64,
        revision: i64,
    ) -> Result<Value> {
        let contrato = json!({"operacion":"seleccion_explicita","tarea":clave_tarea,
            "sesion":sesion,"intervalo":intervalo,"revision_seleccion":revision})
        .to_string();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let date = now()?;
        if let Some(old) = existing_cols(
            &tx,
            "tarea_vinculo",
            "instancia_id",
            "vinculado_en",
            "instancia_id",
            &instancia_id.to_string(),
            &contrato,
        )? {
            tx.commit()?;
            return Ok(old);
        }
        let t = tarea(&tx, clave_tarea)?;
        ensure!(
            t.version == version,
            "la definicion cambio entre la seleccion congelada y el enlace"
        );
        tx.execute(
            "INSERT INTO tarea_vinculo(instancia_id,tarea_id,definicion_version,sesion_clave,intervalo_id,
               revision_seleccion,metodo,evidencia,vinculado_en,contrato_json)
             VALUES (?1,?2,?3,?4,?5,?6,'seleccion_explicita',?7,?8,?9)",
            params![
                instancia_id,
                t.id,
                version,
                sesion,
                intervalo,
                revision,
                "seleccion explicita de tarea vigente al admitir el callback",
                date,
                contrato
            ],
        )?;
        tx.commit()?;
        Ok(receipt(instancia_id, date, "tarea_vinculo"))
    }

    pub fn list_tasks(&self, project: &str) -> Result<Value> {
        nonempty(project)?;
        Ok(json!({"proyecto":project,"tareas":rows(&self.conn,
            "SELECT t.clave,t.creada_en,
                    (SELECT max(definicion_version) FROM tarea_evento d WHERE d.tarea_id=t.id AND d.tipo='definicion') AS definicion_version,
                    (SELECT count(*) FROM tarea_vinculo v WHERE v.tarea_id=t.id) AS eventos_vinculados,
                    (SELECT count(*) FROM tarea_resultado r WHERE r.tarea_id=t.id) AS resultados,
                    (SELECT count(*) FROM tarea_evento c WHERE c.tarea_id=t.id AND c.tipo='cierre') AS cierres
             FROM tarea t WHERE t.proyecto=?1 ORDER BY t.id", [project])?}))
    }
}

fn vinculado(c: &Connection, tarea_id: i64, instancia_id: i64) -> Result<()> {
    let propio: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM tarea_vinculo WHERE tarea_id=?1 AND instancia_id=?2)",
        params![tarea_id, instancia_id],
        |r| r.get(0),
    )?;
    ensure!(
        propio,
        "el evento no esta vinculado a esta tarea; enlazarlo explicitamente primero"
    );
    Ok(())
}

/// Una corrección conserva criterio, versión, alcance y objetivo del antecedente
/// y no puede bifurcarse: el padre admite un solo sucesor.
fn correccion(
    c: &Connection,
    anterior: Option<&str>,
    t: &Tarea,
    criterio: &str,
    version: &str,
    alcance: &str,
    objetivo: Option<i64>,
) -> Result<Option<i64>> {
    let Some(clave) = anterior else {
        return Ok(None);
    };
    let fila: (i64, i64, String, String, String, Option<i64>) = c
        .query_row(
            "SELECT id,tarea_id,criterio_id,criterio_version,alcance,objetivo_instancia_id
             FROM tarea_resultado WHERE clave=?1",
            [clave],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?
        .context("resultado anterior ausente")?;
    ensure!(
        fila.1 == t.id
            && fila.2 == criterio
            && fila.3 == version
            && fila.4 == alcance
            && fila.5 == objetivo,
        "correccion incompatible con el resultado anterior"
    );
    Ok(Some(fila.0))
}

/// Une intervalos solapados y deja visible cualquier intervalo abierto. La
/// etiqueta es «ventana observada», no tiempo de razonamiento ni trabajo humano.
pub(crate) fn ventanas(eventos: &[(String, i64)]) -> (Vec<(i64, i64)>, Option<i64>) {
    let mut abiertos = 0i32;
    let mut inicio = 0i64;
    let mut cerrados = Vec::new();
    for (tipo, tiempo) in eventos {
        match tipo.as_str() {
            "inicio" | "reanudacion" => {
                if abiertos == 0 {
                    inicio = *tiempo;
                }
                abiertos += 1;
            }
            "pausa" | "cierre" if abiertos > 0 => {
                abiertos -= 1;
                if abiertos == 0 {
                    cerrados.push((inicio, *tiempo));
                }
            }
            _ => {}
        }
    }
    (cerrados, (abiertos > 0).then_some(inicio))
}

impl Store {
    /// Informe por tarea desde una instantánea transaccional. Cada cifra declara
    /// unidad, alcance y método; lo desconocido nunca vale cero.
    pub fn task_report(&mut self, project: &str, clave: &str) -> Result<Value> {
        self.task_report_desde(project, clave, 0)
    }

    /// `desde` acota el informe a los eventos posteriores a esa instancia: lo
    /// usa una corrida para no sumar métricas históricas de la copia.
    pub fn task_report_desde(&mut self, project: &str, clave: &str, desde: i64) -> Result<Value> {
        nonempty(project)?;
        let tx = self.conn.transaction()?;
        let corte = now()?;
        let t = tarea(&tx, clave)?;
        ensure!(t.proyecto == project, "la tarea pertenece a otro proyecto");

        let definiciones = rows(&tx,
            "SELECT clave,definicion_version,descripcion,criterios_json,registrado_en FROM tarea_evento
             WHERE tarea_id=?1 AND tipo='definicion' ORDER BY definicion_version", [t.id])?;
        let marcadores = rows(&tx,
            "SELECT clave,tipo,definicion_version,detalle_json,fuente,registrado_en FROM tarea_evento
             WHERE tarea_id=?1 AND tipo<>'definicion' ORDER BY id", [t.id])?;
        let vigente: Vec<Criterio> = serde_json::from_str(
            definiciones
                .last()
                .and_then(|d| d["criterios_json"].as_str())
                .context("definicion vigente ausente")?,
        )?;

        // Eventos vinculados y, por separado, los que declararon esta tarea al
        // capturarse pero cuyo enlace no llegó a confirmarse.
        let vinculados = rows(&tx,
            "SELECT v.instancia_id,v.metodo,v.definicion_version,v.sesion_clave,v.intervalo_id,v.revision_seleccion,
                    i.clave_evento,i.origen,i.rol,i.timestamp,i.metadatos_json
             FROM tarea_vinculo v JOIN instancia i ON i.id=v.instancia_id
             WHERE v.tarea_id=?1 AND v.instancia_id>?2 ORDER BY v.instancia_id", params![t.id, desde])?;
        let pendientes = rows(
            &tx,
            "SELECT i.id AS instancia_id,i.clave_evento,i.origen,i.rol,i.timestamp
             FROM instancia i LEFT JOIN tarea_vinculo v ON v.instancia_id=i.id
             WHERE v.instancia_id IS NULL
               AND json_extract(i.metadatos_json,'$.tarea.clave')=?1 AND i.id>?2 ORDER BY i.id",
            params![clave, desde],
        )?;

        let mut sesiones: BTreeSet<String> = BTreeSet::new();
        let mut herramientas: BTreeMap<String, Value> = BTreeMap::new();
        let mut sin_identidad = 0i64;
        let mut identidad_por_hash = 0i64;
        let mut modos: BTreeSet<String> = BTreeSet::new();
        let ids: Vec<i64> = vinculados
            .iter()
            .filter_map(|v| v["instancia_id"].as_i64())
            .collect();
        for v in &vinculados {
            if let Some(s) = v["sesion_clave"].as_str() {
                sesiones.insert(s.to_string());
            }
            let meta: Value = v["metadatos_json"]
                .as_str()
                .and_then(|m| serde_json::from_str(m).ok())
                .unwrap_or(Value::Null);
            if meta["identidad_fuente"] == "hash_payload" {
                identidad_por_hash += 1;
            }
            if let Some(m) = meta["modo"].as_str() {
                modos.insert(m.to_string());
            }
            let tipo = meta["tipo"].as_str().unwrap_or("");
            if tipo == "herramienta-ok" || tipo == "herramienta-error" {
                match meta["tool_use_id"].as_str() {
                    // Observación terminal por ID de operación; el replay no suma.
                    Some(id) => {
                        let fila = herramientas.entry(id.to_string()).or_insert_with(
                            || json!({"tool_use_id":id,"observaciones":0,"exito_tecnico":0,"error":0}),
                        );
                        fila["observaciones"] =
                            json!(fila["observaciones"].as_i64().unwrap_or(0) + 1);
                        let campo = if tipo == "herramienta-ok" {
                            "exito_tecnico"
                        } else {
                            "error"
                        };
                        fila[campo] = json!(fila[campo].as_i64().unwrap_or(0) + 1);
                    }
                    None => sin_identidad += 1,
                }
            }
        }

        // Resultados por criterio: hojas vigentes, conflictos y desconocidos.
        let mut resultados = rows(&tx,
            "SELECT id,clave,criterio_id,criterio_version,alcance,objetivo_instancia_id,fuente,resultado,
                    detalle_json,evaluacion_ejecucion_id,anterior_id,definicion_version,registrado_en
             FROM tarea_resultado WHERE tarea_id=?1 AND fuente_instancia_id>?2 ORDER BY id", params![t.id, desde])?;
        let mut por_criterio: BTreeMap<String, Value> = BTreeMap::new();
        for i in 0..resultados.len() {
            let hoja = !resultados
                .iter()
                .any(|x| x["anterior_id"] == resultados[i]["id"]);
            resultados[i]["vigente"] = json!(hoja);
            if !hoja {
                continue;
            }
            // Una corrección NULL no crea un resultado conocido: se conserva el
            // último conocido de su cadena, declarando que viene del antecedente.
            let mut conocido = &resultados[i];
            let mut heredado = false;
            while conocido["resultado"].is_null() && !conocido["anterior_id"].is_null() {
                let previo = resultados
                    .iter()
                    .find(|x| x["id"] == conocido["anterior_id"])
                    .context("cadena de correccion incompleta")?;
                conocido = previo;
                heredado = true;
            }
            let fila = &resultados[i];
            let clave_criterio = format!(
                "{}/{}",
                fila["criterio_id"].as_str().unwrap_or(""),
                fila["criterio_version"].as_str().unwrap_or("")
            );
            let entrada = por_criterio.entry(clave_criterio).or_insert_with(|| {
                json!({"criterio":fila["criterio_id"],"version":fila["criterio_version"],
                       "hojas":[],"resultados_conocidos":[],"evidencias":[]})
            });
            entrada["hojas"].as_array_mut().unwrap().push(
                json!({"id":fila["id"],"clave":fila["clave"],"resultado":fila["resultado"],
                    "resultado_conocido_de_la_cadena":conocido["resultado"],
                    "heredado_de_correccion_anterior":heredado,
                    "alcance":fila["alcance"],"fuente":fila["fuente"],
                    "evaluacion_ejecucion_id":fila["evaluacion_ejecucion_id"]}),
            );
            entrada["evidencias"]
                .as_array_mut()
                .unwrap()
                .push(fila["clave"].clone());
            if !conocido["resultado"].is_null() {
                let valor = conocido["resultado"].clone();
                let lista = entrada["resultados_conocidos"].as_array_mut().unwrap();
                if !lista.contains(&valor) {
                    lista.push(valor);
                }
            }
        }
        for entrada in por_criterio.values_mut() {
            let n = entrada["resultados_conocidos"].as_array().unwrap().len();
            entrada["estado"] = json!(match n {
                0 => "desconocido",
                1 => "resultado_bajo_criterio",
                _ => "conflicto",
            });
        }

        // Cobertura de criterios obligatorios. Sin evidencia suficiente no se
        // afirma «todos verificados»; desconocido no es cero.
        let mut cumplidos = Vec::new();
        let mut fallidos = Vec::new();
        let mut desconocidos = Vec::new();
        let mut conflictos = Vec::new();
        for c in &vigente {
            let clave_criterio = format!("{}/{}", c.id, c.version);
            match por_criterio.get(&clave_criterio) {
                None => desconocidos.push(clave_criterio),
                Some(e) => match e["estado"].as_str().unwrap_or("desconocido") {
                    "conflicto" => conflictos.push(clave_criterio),
                    "resultado_bajo_criterio" => {
                        if e["resultados_conocidos"][0] == 1 {
                            cumplidos.push(clave_criterio)
                        } else {
                            fallidos.push(clave_criterio)
                        }
                    }
                    _ => desconocidos.push(clave_criterio),
                },
            }
        }
        let obligatorios: BTreeSet<String> = vigente
            .iter()
            .filter(|c| c.obligatorio)
            .map(|c| format!("{}/{}", c.id, c.version))
            .collect();
        let obligatorios_verificados =
            !obligatorios.is_empty() && obligatorios.iter().all(|k| cumplidos.contains(k));

        // Memoria: entregas de los problemas vinculados a esta tarea.
        let mut entregas = Vec::new();
        for id in &ids {
            entregas.extend(rows(&tx,
                "SELECT e.id,e.clave,e.bytes_salida,e.presupuesto_integracion_bytes,e.sesion_clave,
                        (SELECT tipo FROM entrega_contexto_evento h WHERE h.entrega_id=e.id ORDER BY h.id DESC LIMIT 1) AS ultimo_evento
                 FROM entrega_contexto_host e WHERE e.problema_instancia_id=?1 ORDER BY e.id", [id])?);
        }
        let completadas = entregas
            .iter()
            .filter(|e| e["ultimo_evento"] == "escritura_completada")
            .count();
        let inciertas = entregas.len() - completadas;
        let paquetes: i64 = tx.query_row(
            "SELECT count(*) FROM paquete_recuperacion p JOIN evento_recuperacion r ON r.id=p.evento_id
             JOIN tarea_vinculo v ON v.instancia_id=r.problema_instancia_id WHERE v.tarea_id=?1 AND v.instancia_id>?2",
            params![t.id, desde],
            |r| r.get(0),
        )?;

        let ejecuciones = rows(&tx,
            "SELECT DISTINCT e.intento_clave,e.id FROM ejecucion e JOIN ejecucion_evento ee ON ee.ejecucion_id=e.id
             JOIN tarea_vinculo v ON v.instancia_id=ee.instancia_id WHERE v.tarea_id=?1 AND v.instancia_id>?2 ORDER BY e.id", params![t.id, desde])?;

        let cronologia: Vec<(String, i64)> = marcadores
            .iter()
            .map(|m| {
                (
                    m["tipo"].as_str().unwrap_or("").to_string(),
                    m["registrado_en"].as_i64().unwrap_or(0),
                )
            })
            .collect();
        let (cerradas, abierta) = ventanas(&cronologia);
        let total: i64 = cerradas.iter().map(|(a, b)| b - a).sum();
        tx.commit()?;

        Ok(json!({
            "version": 1,
            "tarea": clave,
            "proyecto": project,
            "corte_microsegundos_utc": corte,
            "definicion_vigente": t.version,
            "definiciones": definiciones,
            "criterios_vigentes": vigente,
            "marcadores_explicitos": marcadores,
            "sesiones": sesiones,
            "desde_instancia_id": desde,
            "modos_observados": modos,
            "intentos_correlacionados": ejecuciones,
            "eventos_vinculados": vinculados.len(),
            "eventos_pendientes_de_enlace": pendientes,
            "resultados": resultados,
            "por_criterio": por_criterio.into_values().collect::<Vec<_>>(),
            "calidad": {
                "unidad": "criterios de la definicion vigente",
                "alcance": "esta tarea",
                "metodo": "hojas vigentes por criterio; sin votacion ni gana-la-ultima",
                "cumplidos": cumplidos, "fallidos": fallidos,
                "desconocidos": desconocidos, "conflictos": conflictos,
                "obligatorios": obligatorios,
                "todos_los_obligatorios_verificados": obligatorios_verificados,
                "nota": "desconocido no es cero; cerrar la tarea no confirma calidad"
            },
            "herramientas": {
                "unidad": "observaciones terminales por ID de operacion del host",
                "alcance": "eventos vinculados a esta tarea",
                "metodo": "deduplicado por tool_use_id; un replay no suma",
                "operaciones": herramientas.into_values().collect::<Vec<_>>(),
                "observaciones_sin_identidad_de_operacion": sin_identidad,
                "cobertura": if sin_identidad > 0 { "parcial" } else { "por_id_de_operacion" },
                "nota": "no son llamadas al modelo"
            },
            "memoria": {
                "unidad": "bytes UTF-8 exactos por entrega",
                "alcance": "problemas vinculados a esta tarea",
                "paquetes_preparados": paquetes,
                "entregas": entregas,
                "entregas_escritura_completada": completadas,
                "entregas_inciertas": inciertas,
                "nota": "escribir al canal del host no es recepcion remota ni aplicacion causal"
            },
            "tiempo": {
                "unidad": "microsegundos UTC",
                "alcance": "ventanas explicitas marcadas por el usuario",
                "metodo": "union de intervalos solapados entre sesiones",
                "ventanas_cerradas": cerradas.iter().map(|(a,b)| json!({"desde":a,"hasta":b})).collect::<Vec<_>>(),
                "ventana_observada_total": total,
                "intervalo_abierto_desde": abierta,
                "nota": "ventana observada; no es tiempo de razonamiento ni trabajo humano activo"
            },
            "tokens": {
                "valor": Value::Null,
                "motivo": "ningun hook de la version soportada expone conteo de tokens; no se estima a partir de bytes"
            },
            "cobertura_de_identidad": {
                "eventos_con_identidad_derivada_del_payload": identidad_por_hash,
                "limite": "sin ID de fuente, dos callbacks identicos no se distinguen de un replay",
                "conteos_exactos": identidad_por_hash == 0
            },
            "nota_global": "instrumentacion descriptiva: ninguna cifra demuestra aprendizaje, causalidad ni mejora"
        }))
    }
}

/// Markdown legible a partir del mismo JSON, sin recalcular nada.
pub fn informe_markdown(informe: &Value) -> String {
    let mut s = String::new();
    let lista = |v: &Value| -> String {
        let items: Vec<String> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|x| x.as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();
        if items.is_empty() {
            "ninguno".to_string()
        } else {
            items.join(", ")
        }
    };
    s.push_str(&format!(
        "# Tarea {}\n\n",
        informe["tarea"].as_str().unwrap_or("")
    ));
    s.push_str(&format!(
        "- Proyecto: `{}`\n- Definicion vigente: v{}\n- Corte: {} us UTC\n- Sesiones: {}\n- Eventos vinculados: {} (pendientes de enlace: {})\n\n",
        informe["proyecto"].as_str().unwrap_or(""),
        informe["definicion_vigente"],
        informe["corte_microsegundos_utc"],
        informe["sesiones"].as_array().map(|a| a.len()).unwrap_or(0),
        informe["eventos_vinculados"],
        informe["eventos_pendientes_de_enlace"].as_array().map(|a| a.len()).unwrap_or(0)
    ));
    let q = &informe["calidad"];
    s.push_str("## Calidad\n\n");
    s.push_str(&format!(
        "| Estado | Criterios |\n|---|---|\n| Cumplidos | {} |\n| Fallidos | {} |\n| Desconocidos | {} |\n| En conflicto | {} |\n\n",
        lista(&q["cumplidos"]), lista(&q["fallidos"]), lista(&q["desconocidos"]), lista(&q["conflictos"])
    ));
    s.push_str(&format!(
        "Todos los obligatorios verificados: **{}**. {}\n\n",
        q["todos_los_obligatorios_verificados"],
        q["nota"].as_str().unwrap_or("")
    ));
    let h = &informe["herramientas"];
    s.push_str("## Esfuerzo observado\n\n");
    s.push_str(&format!(
        "- Operaciones de herramienta por ID: {} (cobertura: {}, sin identidad: {})\n",
        h["operaciones"].as_array().map(|a| a.len()).unwrap_or(0),
        h["cobertura"].as_str().unwrap_or(""),
        h["observaciones_sin_identidad_de_operacion"]
    ));
    let m = &informe["memoria"];
    s.push_str(&format!(
        "- Paquetes preparados: {}; entregas completadas: {}; inciertas: {}\n",
        m["paquetes_preparados"], m["entregas_escritura_completada"], m["entregas_inciertas"]
    ));
    let t = &informe["tiempo"];
    s.push_str(&format!(
        "- Ventana observada total: {} us; intervalo abierto: {}\n",
        t["ventana_observada_total"],
        if t["intervalo_abierto_desde"].is_null() {
            "no".into()
        } else {
            t["intervalo_abierto_desde"].to_string()
        }
    ));
    s.push_str(&format!(
        "- Tokens: {} ({})\n\n",
        informe["tokens"]["valor"],
        informe["tokens"]["motivo"].as_str().unwrap_or("")
    ));
    s.push_str("## Limites\n\n");
    s.push_str(&format!(
        "- {}\n",
        informe["cobertura_de_identidad"]["limite"]
            .as_str()
            .unwrap_or("")
    ));
    s.push_str(&format!("- {}\n", m["nota"].as_str().unwrap_or("")));
    s.push_str(&format!("- {}\n", t["nota"].as_str().unwrap_or("")));
    s.push_str(&format!(
        "- {}\n",
        informe["nota_global"].as_str().unwrap_or("")
    ));
    s
}
