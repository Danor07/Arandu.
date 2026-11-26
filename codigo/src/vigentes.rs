//! Agrupación literal, competencia entre caminos y archivo. Reglas y límites en README.
use crate::adapter::consulta_literal;
use crate::competencia::{
    alerta, neto, rota, Competencia, COMPETENCIA_VERSION, F_FALLOS, K_VENTAJA, VENTANA,
};
use crate::execution::{nonempty, rows};
use crate::retrieval::Cutoff;
use crate::{digest, now, Store};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const TIPO_VERSION: &str = "objetivo-terminos-v1";
pub const VIGENTE_VERSION: &str = COMPETENCIA_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TipoObjetivo {
    pub clave: String,
    pub proyecto: String,
    pub regla_version: &'static str,
    pub terminos: Vec<String>,
}

/// El límite de consulta_literal se aplica una sola vez al texto original.
pub fn tipo_objetivo(proyecto: &str, objetivo: &str) -> Result<Option<TipoObjetivo>> {
    nonempty(proyecto)?;
    let consulta = consulta_literal(objetivo);
    let mut terminos: Vec<_> = consulta.split_whitespace().map(str::to_string).collect();
    terminos.sort();
    if terminos.is_empty() {
        return Ok(None);
    }
    let clave = digest(serde_json::to_string(&(TIPO_VERSION, proyecto, &terminos))?.as_bytes());
    Ok(Some(TipoObjetivo {
        clave,
        proyecto: proyecto.into(),
        regla_version: TIPO_VERSION,
        terminos,
    }))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Camino {
    pub hash_minima: String,
    pub exitos: usize,
    /// Sólo los atribuidos: episodios fallidos con la secuencia completa de un éxito de este camino.
    pub fallos: usize,
    /// Éxitos menos fallos atribuidos en sus últimas `VENTANA` verificaciones; es lo que compite.
    pub neto: i64,
    /// `vigente`, `archivado` o `candidato`.
    pub estado: &'static str,
    pub sucesor: Option<String>,
    pub primera_verificacion: i64,
    pub ultima_verificacion: i64,
    pub derivacion_id: i64,
    /// Resultados atribuidos, en orden. Alimenta la alerta; no se serializa.
    #[serde(skip)]
    pub serie: Vec<bool>,
    /// Lugar de cada resultado propio entre todas las verificaciones atribuidas del tipo.
    #[serde(skip)]
    pub posiciones: Vec<usize>,
}

fn indexar(c: &Connection, proyecto: &str) -> Result<()> {
    let fuentes=rows(c,"SELECT f.* FROM objetivo_episodio_fuente f LEFT JOIN episodio_tipo_objetivo t
        ON t.origen=f.origen AND t.episodio_id=f.episodio_id WHERE f.proyecto=?1 AND t.tipo_clave IS NULL",[proyecto])?;
    for f in fuentes {
        let Some(tipo) = tipo_objetivo(
            proyecto,
            f["objetivo"].as_str().context("objetivo sin texto")?,
        )?
        else {
            continue;
        };
        c.execute(
            "INSERT OR IGNORE INTO tipo_objetivo VALUES (?1,?2,?3,?4)",
            params![
                tipo.clave,
                proyecto,
                TIPO_VERSION,
                serde_json::to_string(&tipo.terminos)?
            ],
        )?;
        c.execute(
            "INSERT INTO episodio_tipo_objetivo VALUES (?1,?2,?3)",
            params![f["origen"].as_str(), f["episodio_id"].as_i64(), tipo.clave],
        )?;
    }
    Ok(())
}

fn observaciones(
    c: &Connection,
    tipo: &TipoObjetivo,
    corte: Option<&Cutoff>,
) -> Result<Vec<Value>> {
    let mut fuentes=rows(c,"SELECT f.*,CASE f.origen WHEN 'manual_tarea' THEN (SELECT secuencia_sha256 FROM episodio WHERE id=f.episodio_id)
            ELSE (SELECT secuencia_sha256 FROM episodio_automatico WHERE id=f.episodio_id) END AS secuencia_sha256
        FROM objetivo_episodio_fuente f JOIN episodio_tipo_objetivo t
        ON t.origen=f.origen AND t.episodio_id=f.episodio_id WHERE t.tipo_clave=?1
        AND NOT EXISTS(SELECT 1 FROM tarea_resultado correccion JOIN instancia ci ON ci.id=correccion.fuente_instancia_id
            WHERE correccion.anterior_id=f.tarea_resultado_id
              AND (?2 IS NULL OR (correccion.registrado_en<=?3 AND ci.id<?2 AND ci.timestamp<=?3)))
        AND NOT EXISTS(SELECT 1 FROM evaluacion_ejecucion correccion JOIN instancia ci ON ci.id=correccion.fuente_instancia_id
            WHERE correccion.anterior_id=f.evaluacion_ejecucion_id
              AND (?2 IS NULL OR (correccion.registrado_en<=?3 AND ci.id<?2 AND ci.timestamp<=?3)))
        AND (?2 IS NULL OR (f.disponible_en<=?3
            AND NOT EXISTS(SELECT 1 FROM objetivo_episodio_captura cap JOIN instancia i ON i.id=cap.instancia_id
                WHERE cap.origen=f.origen AND cap.episodio_id=f.episodio_id
                AND (i.id>=?2 OR i.timestamp>?3 OR i.grupo_evidencia=?4))))
        ORDER BY (f.hash_minima IS NULL),f.disponible_en,f.origen COLLATE BINARY,f.episodio_id",
        params![tipo.clave,corte.map(|c|c.problem_id),corte.map(|c|c.timestamp),corte.map(|c|c.episode.as_str())])?;
    for f in &mut fuentes {
        if corte.is_some_and(|c| {
            f["minima_disponible_en"]
                .as_i64()
                .is_some_and(|t| t > c.timestamp)
        }) {
            f["hash_minima"] = Value::Null;
            f["derivacion_id"] = Value::Null;
        }
    }
    fuentes.sort_by(|a, b| {
        a["hash_minima"]
            .is_null()
            .cmp(&b["hash_minima"].is_null())
            .then(
                a["disponible_en"]
                    .as_i64()
                    .cmp(&b["disponible_en"].as_i64()),
            )
            .then(a["origen"].as_str().cmp(&b["origen"].as_str()))
            .then(a["episodio_id"].as_i64().cmp(&b["episodio_id"].as_i64()))
    });
    let mut vistas = BTreeSet::new();
    let mut salida = Vec::new();
    for f in fuentes {
        let identidad = if let Some(id) = f["evidencia_id"].as_i64() {
            format!("verificacion/{id}")
        } else {
            format!("sin_verificacion/{}/{}", f["origen"], f["episodio_id"])
        };
        if vistas.insert(identidad) {
            salida.push(f);
        }
    }
    // Fallos tardíos (SZZ): cada arreglo con un vínculo confirmado cuenta como un fallo
    // del camino del episodio señalado, desde la fecha de la confirmación.
    salida.extend(rows(c,"SELECT 'fallo_tardio_szz' AS tipo_registro,0 AS resultado,min(d.registrado_en) AS fecha,min(v.id) AS evidencia_id,
            v.origen,v.episodio_id,f.hash_minima AS camino_szz,v.arreglo_commit
        FROM szz_vinculo v JOIN szz_decision d ON d.vinculo_id=v.id AND d.decision='confirmado'
        JOIN episodio_tipo_objetivo t ON t.origen=v.origen AND t.episodio_id=v.episodio_id
        JOIN objetivo_episodio_fuente f ON f.origen=v.origen AND f.episodio_id=v.episodio_id
        WHERE t.tipo_clave=?1 AND f.hash_minima IS NOT NULL AND (?2 IS NULL OR d.registrado_en<=?2)
        GROUP BY v.arreglo_commit,v.origen,v.episodio_id,f.hash_minima ORDER BY 4",
        params![tipo.clave,corte.map(|c|c.timestamp)])?);
    Ok(salida)
}

/// Pliega las verificaciones dadas, en orden de fecha, y devuelve los caminos con su estado.
pub fn competir(observadas: &[Value]) -> Result<Vec<Camino>> {
    let mut caminos: BTreeMap<String, Camino> = BTreeMap::new();
    // Secuencia completa de cada episodio exitoso → su camino. Éxito y fallo se
    // atribuyen con la misma vara: la secuencia completa del episodio.
    let mut completas: BTreeMap<&str, &str> = BTreeMap::new();
    for f in observadas {
        if f["resultado"] != 1 {
            continue;
        }
        if let (Some(hash), Some(derivacion)) =
            (f["hash_minima"].as_str(), f["derivacion_id"].as_i64())
        {
            if let Some(completa) = f["secuencia_sha256"].as_str() {
                completas.insert(completa, hash);
            }
            let fecha = f["fecha"].as_i64().context("verificacion sin fecha")?;
            let camino = caminos.entry(hash.into()).or_insert(Camino {
                hash_minima: hash.into(),
                exitos: 0,
                fallos: 0,
                neto: 0,
                estado: "candidato",
                sucesor: None,
                primera_verificacion: fecha,
                ultima_verificacion: fecha,
                derivacion_id: derivacion,
                serie: Vec::new(),
                posiciones: Vec::new(),
            });
            camino.exitos += 1;
            camino.primera_verificacion = camino.primera_verificacion.min(fecha);
            if fecha > camino.ultima_verificacion
                || (fecha == camino.ultima_verificacion && derivacion < camino.derivacion_id)
            {
                camino.ultima_verificacion = fecha;
                camino.derivacion_id = derivacion;
            }
        }
    }
    // Una fila por verificación atribuida a un camino. Restar evidencia más
    // adelante (fallos tardíos) es agregar otra fila con exito=false.
    let mut filas: Vec<(i64, i64, &str, bool)> = Vec::new();
    for f in observadas {
        let exito = f["resultado"] == 1;
        let hash = if let Some(camino) = f["camino_szz"].as_str() {
            Some(camino)
        } else if exito {
            f["hash_minima"].as_str()
        } else if f["resultado"] == 0 {
            f["secuencia_sha256"]
                .as_str()
                .and_then(|s| completas.get(s).copied())
        } else {
            None
        };
        if let Some(hash) = hash.filter(|h| caminos.contains_key(*h)) {
            filas.push((
                f["fecha"].as_i64().context("verificacion sin fecha")?,
                f["evidencia_id"]
                    .as_i64()
                    .context("verificacion sin evidencia")?,
                hash,
                exito,
            ));
        }
    }
    filas.sort();
    let mut competencia = Competencia::default();
    for (posicion, (_, _, hash, exito)) in filas.into_iter().enumerate() {
        competencia.observar(hash, exito, K_VENTAJA, F_FALLOS);
        if let Some(camino) = caminos.get_mut(hash) {
            camino.serie.push(exito);
            camino.posiciones.push(posicion);
        }
    }
    for camino in caminos.values_mut() {
        camino.fallos = camino.serie.iter().filter(|e| !**e).count();
        camino.neto = neto(&camino.serie[camino.serie.len().saturating_sub(VENTANA)..]);
        camino.sucesor = competencia.archivados.get(&camino.hash_minima).cloned();
        camino.estado = if competencia.vigente.as_ref() == Some(&camino.hash_minima) {
            "vigente"
        } else if camino.sucesor.is_some() {
            "archivado"
        } else {
            "candidato"
        };
    }
    Ok(caminos.into_values().collect())
}

const INSERTAR_ESTADO: &str = "INSERT INTO camino_estado(tipo_clave,hash_minima,estado,sucesor_hash,regla_version,evidencia_json,registrado_en) VALUES (?1,?2,?3,?4,?5,?6,?7)";

/// Lleva el estado actual al historial. Los vínculos del camino que se archiva
/// (entregas selladas que lo ofrecieron) se migran antes de su fila; nada se borra.
fn materializar(c: &Connection, tipo: &TipoObjetivo, caminos: &[Camino]) -> Result<()> {
    let mut previos: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    for p in rows(
        c,
        "SELECT hash_minima,estado,sucesor_hash FROM camino_estado_actual WHERE tipo_clave=?1",
        [&tipo.clave],
    )? {
        previos.insert(
            p["hash_minima"].as_str().context("estado sin hash")?.into(),
            (
                p["estado"].as_str().context("estado sin valor")?.into(),
                p["sucesor_hash"].as_str().map(str::to_string),
            ),
        );
    }
    let ahora = now()?;
    for camino in caminos {
        if let Some(sucesor) = &camino.sucesor {
            let destino = caminos
                .iter()
                .find(|x| &x.hash_minima == sucesor)
                .context("sucesor sin camino")?;
            c.execute(
                "INSERT OR IGNORE INTO camino_vinculo_migrado SELECT v.evento_id,?3,?4,?5 FROM recuperacion_vigente v
                 JOIN camino_derivacion d ON d.tipo_clave=v.tipo_clave AND d.derivacion_id=v.derivacion_id
                 WHERE v.tipo_clave=?1 AND d.hash_minima<>?3 AND (d.hash_minima=?2
                    OR EXISTS(SELECT 1 FROM camino_vinculo_migrado g WHERE g.evento_id=v.evento_id AND g.sucesor_hash=?2))",
                params![tipo.clave, camino.hash_minima, sucesor, destino.derivacion_id, ahora],
            )?;
        }
        let previo = previos
            .remove(&camino.hash_minima)
            .unwrap_or(("candidato".into(), None));
        if (previo.0.as_str(), &previo.1) != (camino.estado, &camino.sucesor) {
            c.execute(
                INSERTAR_ESTADO,
                params![
                    tipo.clave,
                    camino.hash_minima,
                    camino.estado,
                    camino.sucesor,
                    VIGENTE_VERSION,
                    serde_json::to_string(camino)?,
                    ahora
                ],
            )?;
        }
    }
    // Un camino que perdió todos sus éxitos por correcciones deja de competir: vuelve a candidato.
    for (hash, (estado, _)) in previos {
        if estado != "candidato" {
            c.execute(
                INSERTAR_ESTADO,
                params![
                    tipo.clave,
                    hash,
                    "candidato",
                    None::<String>,
                    VIGENTE_VERSION,
                    r#"{"motivo":"sin exitos vigentes"}"#,
                    ahora
                ],
            )?;
        }
    }
    Ok(())
}

fn resumen(
    c: &Connection,
    tipo: &TipoObjetivo,
    corte: Option<&Cutoff>,
    detalle: bool,
) -> Result<Value> {
    let observadas = observaciones(c, tipo, corte)?;
    let caminos = competir(&observadas)?;
    if corte.is_none() {
        materializar(c, tipo, &caminos)?;
    }
    let mut vigente = Value::Null;
    if let Some(camino) = caminos.iter().find(|x| x.estado == "vigente") {
        let pasos = crate::urna::pasos(c, camino.derivacion_id)?;
        let referencia = observadas
            .iter()
            .find(|f| f["derivacion_id"] == camino.derivacion_id)
            .context("camino sin evidencia")?;
        // Rota: sigue vigente porque no hay otra mejor, pero no se le ofrece al host.
        vigente = json!({"camino":camino,"pasos":pasos,"alerta":alerta(&camino.serie),"en_falla":rota(&camino.serie, F_FALLOS),"referencia":{
            "origen":referencia["origen"],"episodio_id":referencia["episodio_id"],"evidencia_id":referencia["evidencia_id"],
            "criterio_id":referencia["criterio_id"],"criterio_version":referencia["criterio_version"]}});
    }
    let fechas: Vec<_> = observadas
        .iter()
        .filter_map(|f| f["fecha"].as_i64())
        .collect();
    let exitos = observadas.iter().filter(|f| f["resultado"] == 1).count();
    let fallos = observadas.iter().filter(|f| f["resultado"] == 0).count();
    let sin_minima = observadas
        .iter()
        .filter(|f| f["resultado"] == 1 && f["hash_minima"].is_null())
        .count();
    let ultimo_fallo = observadas
        .iter()
        .filter(|f| f["resultado"] == 0)
        .filter_map(|f| f["fecha"].as_i64())
        .max();
    let mut salida = json!({"tipo":tipo,"regla_vigente":VIGENTE_VERSION,"provisional":true,"vigente":vigente,
        "evidencia_tipo":{"exitos":exitos,"fallos":fallos,"fallos_atribuidos":caminos.iter().map(|x|x.fallos).sum::<usize>(),
            "fallos_tardios":observadas.iter().filter(|f| !f["camino_szz"].is_null()).count(),"sin_verificacion":observadas.len()-exitos-fallos,"exitos_sin_minima":sin_minima,"ultimo_fallo":ultimo_fallo,
            "primera_fecha":fechas.iter().min(),"ultima_fecha":fechas.iter().max()},
        "alcance":"coincidencia literal de objetivos; verificaciones observadas, no ensayos independientes ni garantia de calidad",
        "fallos":"se atribuyen a un camino solo si la secuencia completa del episodio fallido es la de un episodio exitoso de ese camino; el resto queda en el tipo"});
    if detalle {
        salida["caminos"] = json!(caminos);
        salida["urna"] = crate::urna::observada(c, tipo, &observadas, &caminos, corte)?;
        salida["observaciones"] = json!(observadas);
        salida["archivo"] = json!({
            "historial":rows(c,"SELECT * FROM camino_estado WHERE tipo_clave=?1 ORDER BY id",[&tipo.clave])?,
            "vinculos_migrados":rows(c,"SELECT g.* FROM camino_vinculo_migrado g JOIN recuperacion_vigente v ON v.evento_id=g.evento_id
                WHERE v.tipo_clave=?1 ORDER BY g.evento_id,g.sucesor_hash",[&tipo.clave])?});
    }
    Ok(salida)
}

pub(crate) fn recuperar(
    c: &Connection,
    proyecto: &str,
    objetivo: &str,
    corte: &Cutoff,
) -> Result<Option<Value>> {
    let Some(tipo) = tipo_objetivo(proyecto, objetivo)? else {
        return Ok(None);
    };
    indexar(c, proyecto)?;
    let mut valor = resumen(c, &tipo, Some(corte), false)?;
    if valor["vigente"].is_null() || valor["vigente"]["en_falla"] == true {
        return Ok(None);
    }
    valor["tipo_registro"] = json!("solucion_vigente_provisional");
    Ok(Some(valor))
}

/// Después de sellar una entrega: el archivo refleja el estado de hoy aunque lo
/// entregado se haya decidido con el corte del pedido, y una entrega que ofreció
/// un camino ya archivado recibe su vínculo en la misma transacción.
pub(crate) fn actualizar_archivo(c: &Connection, proyecto: &str, objetivo: &str) -> Result<()> {
    if let Some(tipo) = tipo_objetivo(proyecto, objetivo)? {
        materializar(c, &tipo, &competir(&observaciones(c, &tipo, None)?)?)?;
    }
    Ok(())
}

impl Store {
    pub fn consultar_vigente(&mut self, proyecto: &str, objetivo: &str) -> Result<Value> {
        nonempty(proyecto)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        indexar(&tx, proyecto)?;
        let salida = match tipo_objetivo(proyecto, objetivo)? {
            Some(tipo) => resumen(&tx, &tipo, None, true)?,
            None => json!({"tipo":null,"vigente":null,"motivo":"objetivo sin terminos"}),
        };
        tx.commit()?;
        Ok(salida)
    }

    pub fn listar_tipos_objetivo(&mut self, proyecto: &str) -> Result<Value> {
        nonempty(proyecto)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        indexar(&tx, proyecto)?;
        let tipos = rows(
            &tx,
            "SELECT * FROM tipo_objetivo WHERE proyecto=?1 ORDER BY clave",
            [proyecto],
        )?;
        let mut salida = Vec::new();
        for t in tipos {
            let tipo = TipoObjetivo {
                clave: t["clave"].as_str().context("tipo sin clave")?.into(),
                proyecto: proyecto.into(),
                regla_version: TIPO_VERSION,
                terminos: serde_json::from_str(
                    t["terminos_json"].as_str().context("tipo sin terminos")?,
                )?,
            };
            salida.push(resumen(&tx, &tipo, None, true)?);
        }
        tx.commit()?;
        Ok(
            json!({"proyecto":proyecto,"regla_tipo":TIPO_VERSION,"regla_vigente":VIGENTE_VERSION,"tipos":salida}),
        )
    }
}
