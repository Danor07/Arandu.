//! Recuperación de una solución observada por tipo de objetivo.
//!
//! No promueve, no cuenta éxitos y no afirma generalización: elige el
//! representante elegible más corto entre reducciones selladas del proyecto.
//! Si no hay candidato, devuelve coincidencias FTS5 como respaldo textual.
use crate::execution::{nonempty, rows};
use crate::retrieval::{buscar_con_corte, SearchRequest};
use crate::secuencia::leer_id;
use crate::{digest, Store};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

pub const ALGORITMO: &str = "solucion-observada-v1";
pub const ORDEN: &str = "cantidad_incluidos ASC, reciente DESC, id ASC";
pub const LIMITE_POR_OMISION: usize = 20;
pub const LIMITE_MAXIMO: usize = 1000;
/// Mismo límite que `SearchRequest::validate`; la derivación no puede superarlo.
const MAX_CONSULTA_BYTES: usize = 16_384;
const MAX_TERMINOS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TipoObjetivo {
    Manual {
        tarea: String,
        definicion_version: i64,
        criterio_id: String,
        criterio_version: String,
    },
    Automatico {
        objetivo: String,
    },
}

fn texto(value: &Value, campo: &str) -> Result<String> {
    let texto = value
        .as_str()
        .with_context(|| format!("{campo} debe ser texto"))?;
    ensure!(!texto.trim().is_empty(), "{campo} no puede estar vacio");
    Ok(texto.to_string())
}

impl TipoObjetivo {
    /// Valida forma y componentes. No normaliza el texto: el objetivo
    /// automático se compara por su texto exacto y el manual por sus cuatro
    /// componentes versionados.
    pub fn desde_json(value: &Value) -> Result<Self> {
        let items = value
            .as_array()
            .context("tipo de objetivo debe ser un arreglo JSON")?;
        let clase = items
            .first()
            .and_then(Value::as_str)
            .context("tipo de objetivo sin clase")?;
        match clase {
            "manual_tarea" => {
                ensure!(
                    items.len() == 5,
                    "tipo manual_tarea exige [clase,tarea,definicion_version,criterio_id,criterio_version]"
                );
                let tarea = texto(&items[1], "tarea")?;
                let definicion_version = items[2]
                    .as_i64()
                    .context("definicion_version debe ser un entero")?;
                ensure!(
                    definicion_version > 0,
                    "definicion_version debe ser positiva"
                );
                let criterio_id = texto(&items[3], "criterio_id")?;
                let criterio_version = texto(&items[4], "criterio_version")?;
                Ok(Self::Manual {
                    tarea,
                    definicion_version,
                    criterio_id,
                    criterio_version,
                })
            }
            "automatico_lista" => {
                ensure!(
                    items.len() == 2,
                    "tipo automatico_lista exige [clase,objetivo]"
                );
                Ok(Self::Automatico {
                    objetivo: texto(&items[1], "objetivo")?,
                })
            }
            otra => bail!("clase de tipo de objetivo no soportada: {otra}"),
        }
    }

    pub fn valor(&self) -> Value {
        match self {
            Self::Manual {
                tarea,
                definicion_version,
                criterio_id,
                criterio_version,
            } => json!([
                "manual_tarea",
                tarea,
                definicion_version,
                criterio_id,
                criterio_version
            ]),
            Self::Automatico { objetivo } => json!(["automatico_lista", objetivo]),
        }
    }

    pub fn canonico(&self) -> Result<String> {
        Ok(serde_json::to_string(&self.valor())?)
    }

    pub fn sha256(&self) -> Result<String> {
        Ok(digest(self.canonico()?.as_bytes()))
    }

    pub fn origen(&self) -> &'static str {
        match self {
            Self::Manual { .. } => "manual_tarea",
            Self::Automatico { .. } => "automatico_lista",
        }
    }

    pub fn verificacion(&self) -> &'static str {
        match self {
            Self::Manual { .. } => "criterio_tarea",
            Self::Automatico { .. } => "comprobacion_tecnica",
        }
    }

    pub fn alcance(&self) -> &'static str {
        match self {
            Self::Manual { .. } => {
                "misma tarea y version de definicion; criterio y version forman parte del tipo"
            }
            Self::Automatico { .. } => {
                "texto exacto del objetivo de lista y comprobacion tecnica; la coincidencia de texto no implica equivalencia causal"
            }
        }
    }
}

struct Problema {
    instancia_id: i64,
    grupo_evidencia: String,
    /// Tiempo de fuente declarado por el evento; corta eventos observados.
    timestamp: i64,
    /// Tiempo local de recepción; compara registros locales de cierre/resultado.
    recibido_en: i64,
    contenido: String,
}

struct Candidato {
    id: i64,
    clave: String,
    origen: String,
    episodio_id: i64,
    cantidad_incluidos: i64,
    reciente: i64,
}

fn cargar_problema(c: &Connection, proyecto: &str, instancia_id: i64) -> Result<Problema> {
    let h = rows(
        c,
        "SELECT i.id,i.grupo_evidencia,i.timestamp,i.contenido,i.rol,c.recibido_en
         FROM instancia i JOIN instancia_proyecto p ON p.instancia_id=i.id
         JOIN captura_evento c ON c.instancia_id=i.id
         WHERE i.id=?1 AND p.proyecto=?2",
        params![instancia_id, proyecto],
    )?
    .pop()
    .context("evento consultante ausente en el proyecto")?;
    ensure!(
        h["rol"].as_str() == Some("problema"),
        "el evento consultante debe tener rol problema"
    );
    Ok(Problema {
        instancia_id: h["id"].as_i64().context("problema sin id")?,
        grupo_evidencia: h["grupo_evidencia"]
            .as_str()
            .context("problema sin grupo")
            .map(str::to_string)?,
        timestamp: h["timestamp"].as_i64().context("problema sin timestamp")?,
        recibido_en: h["recibido_en"]
            .as_i64()
            .context("problema sin recibido_en")?,
        contenido: h["contenido"]
            .as_str()
            .context("problema sin contenido")?
            .to_string(),
    })
}

fn candidatos(c: &Connection, proyecto: &str, tipo: &TipoObjetivo) -> Result<Vec<Candidato>> {
    let canonico = tipo.canonico()?;
    let sha = tipo.sha256()?;
    let filas = rows(
        c,
        "SELECT r.id,r.clave,r.origen,r.episodio_id,r.cantidad_incluidos,
                COALESCE(ev.timestamp,r.ultimo_timestamp) AS reciente
         FROM arandu_secuencia_minima r
         JOIN arandu_secuencia_minima_sello s ON s.reduccion_id=r.id
         LEFT JOIN instancia ev ON ev.id = CASE r.origen
             WHEN 'manual_tarea' THEN (
                 SELECT tr.fuente_instancia_id FROM episodio_resultado er
                 JOIN tarea_resultado tr ON tr.id=er.tarea_resultado_id
                 WHERE er.episodio_id=r.episodio_id)
             WHEN 'automatico_lista' THEN (
                 SELECT ar.comprobacion_instancia_id FROM episodio_automatico_resultado ar
                 WHERE ar.episodio_id=r.episodio_id)
         END
         WHERE r.proyecto=?1 AND r.tipo_objetivo_sha256=?2 AND r.tipo_objetivo_json=?3
           AND r.origen=?4 AND r.verificacion=?5
           AND r.estado IN ('candidata_sin_descartes','candidata_con_descartes')
           AND r.resultado=1
         ORDER BY r.cantidad_incluidos ASC,reciente DESC,r.id ASC",
        params![proyecto, sha, canonico, tipo.origen(), tipo.verificacion()],
    )?;
    filas
        .into_iter()
        .map(|f| {
            Ok(Candidato {
                id: f["id"].as_i64().context("secuencia sin id")?,
                clave: f["clave"].as_str().context("secuencia sin clave")?.into(),
                origen: f["origen"].as_str().context("secuencia sin origen")?.into(),
                episodio_id: f["episodio_id"]
                    .as_i64()
                    .context("secuencia sin episodio")?,
                cantidad_incluidos: f["cantidad_incluidos"]
                    .as_i64()
                    .context("secuencia sin cantidad")?,
                reciente: f["reciente"].as_i64().unwrap_or(0),
            })
        })
        .collect()
}

fn fuera_del_corte(evento: &Value, problema: &Problema, clase: &str) -> Result<Option<String>> {
    let id = evento["id"]
        .as_i64()
        .or_else(|| evento["evidencia_id"].as_i64())
        .context("evento usado sin id")?;
    let timestamp = evento["timestamp"]
        .as_i64()
        .or_else(|| evento["evidencia_timestamp"].as_i64())
        .context("evento usado sin timestamp")?;
    let grupo = evento["grupo_evidencia"]
        .as_str()
        .or_else(|| evento["evidencia_grupo"].as_str())
        .context("evento usado sin grupo")?;
    if grupo == problema.grupo_evidencia {
        return Ok(Some(format!("{clase}_del_grupo_del_problema")));
    }
    if id >= problema.instancia_id {
        return Ok(Some(format!("{clase}_fuera_del_corte_por_id")));
    }
    if timestamp > problema.timestamp {
        return Ok(Some(format!("{clase}_fuera_del_corte_por_tiempo")));
    }
    Ok(None)
}

fn corte_instancia(
    c: &Connection,
    instancia_id: i64,
    problema: &Problema,
    clase: &str,
) -> Result<Option<String>> {
    let evento = rows(
        c,
        "SELECT id,timestamp,grupo_evidencia FROM instancia WHERE id=?1",
        [instancia_id],
    )?
    .pop()
    .context("evento del episodio ausente")?;
    fuera_del_corte(&evento, problema, clase)
}

/// La evidencia manual ya está protegida por los triggers de 007; esta
/// comprobación en vivo excluye una corrección posterior sin reescribir la
/// fotografía de la secuencia.
fn evaluar_manual(
    c: &Connection,
    candidato: &Candidato,
    problema: &Problema,
) -> Result<Option<String>> {
    let corregido: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM episodio_resultado er
         JOIN tarea_resultado posterior ON posterior.anterior_id=er.tarea_resultado_id
         WHERE er.episodio_id=?1)",
        [candidato.episodio_id],
        |r| r.get(0),
    )?;
    if corregido {
        return Ok(Some("resultado_corregido".into()));
    }
    let h = rows(
        c,
        "SELECT er.resultado,tr.registrado_en,tr.fuente_instancia_id,
                i.id AS evidencia_id,i.timestamp AS evidencia_timestamp,
                i.grupo_evidencia AS evidencia_grupo,
                cierre.registrado_en AS cierre_registrado,def.registrado_en AS definicion_registrada,
                def.definicion_version AS definicion_version
         FROM episodio_resultado er
         JOIN tarea_resultado tr ON tr.id=er.tarea_resultado_id
         JOIN instancia i ON i.id=tr.fuente_instancia_id
         JOIN episodio e ON e.id=er.episodio_id
         JOIN tarea_evento cierre ON cierre.id=e.cierre_evento_id
         JOIN tarea_evento def ON def.id=e.definicion_evento_id
         WHERE er.episodio_id=?1",
        [candidato.episodio_id],
    )?
    .pop()
    .context("episodio manual sin resultado")?;
    if h["resultado"].as_i64() != Some(1) {
        return Ok(Some("resultado_no_exitoso".into()));
    }
    if h["registrado_en"]
        .as_i64()
        .context("resultado sin registro")?
        > problema.recibido_en
    {
        return Ok(Some("resultado_no_disponible".into()));
    }
    if h["cierre_registrado"]
        .as_i64()
        .context("cierre sin registro")?
        > problema.recibido_en
    {
        return Ok(Some("cierre_no_disponible".into()));
    }
    if h["definicion_registrada"]
        .as_i64()
        .context("definicion sin registro")?
        > problema.recibido_en
    {
        return Ok(Some("definicion_no_disponible".into()));
    }
    if let Some(motivo) = fuera_del_corte(&h, problema, "evidencia")? {
        return Ok(Some(motivo));
    }
    let definicion_version = h["definicion_version"]
        .as_i64()
        .context("episodio sin version de definicion")?;
    let pasos = rows(
        c,
        "SELECT i.id,i.timestamp,i.grupo_evidencia,
                v.vinculado_en,v.definicion_version AS vinculo_definicion_version
         FROM episodio_paso ep
         JOIN instancia i ON i.id=ep.instancia_id
         JOIN tarea_vinculo v ON v.instancia_id=ep.instancia_id
         WHERE ep.episodio_id=?1 ORDER BY ep.orden",
        [candidato.episodio_id],
    )?;
    if pasos.is_empty() {
        return Ok(Some("sin_pasos".into()));
    }
    for paso in pasos {
        if let Some(motivo) = fuera_del_corte(&paso, problema, "paso")? {
            return Ok(Some(motivo));
        }
        if paso["vinculo_definicion_version"].as_i64() != Some(definicion_version) {
            return Ok(Some("vinculo_de_otra_definicion".into()));
        }
        if paso["vinculado_en"]
            .as_i64()
            .context("vinculo sin registro")?
            > problema.recibido_en
        {
            return Ok(Some("vinculo_no_disponible".into()));
        }
    }
    Ok(None)
}

fn evaluar_automatico(
    c: &Connection,
    candidato: &Candidato,
    problema: &Problema,
) -> Result<Option<String>> {
    let h = rows(
        c,
        "SELECT e.chat,e.inicio_instancia_id,e.fin_instancia_id,e.cierre_control_en,
                r.resultado,r.comprobacion_instancia_id
         FROM episodio_automatico e
         LEFT JOIN episodio_automatico_resultado r ON r.episodio_id=e.id
         WHERE e.id=?1",
        [candidato.episodio_id],
    )?
    .pop()
    .context("episodio automatico ausente")?;
    if h["resultado"].as_i64() != Some(1) {
        return Ok(Some("resultado_no_exitoso".into()));
    }
    if h["chat"].as_str() == Some(problema.grupo_evidencia.as_str()) {
        return Ok(Some("chat_del_grupo_del_problema".into()));
    }
    let inicio = h["inicio_instancia_id"]
        .as_i64()
        .context("episodio automatico sin inicio")?;
    if let Some(motivo) = corte_instancia(c, inicio, problema, "inicio")? {
        return Ok(Some(motivo));
    }
    if let Some(fin) = h["fin_instancia_id"].as_i64() {
        if let Some(motivo) = corte_instancia(c, fin, problema, "fin")? {
            return Ok(Some(motivo));
        }
    } else if let Some(cierre) = h["cierre_control_en"].as_i64() {
        if cierre > problema.recibido_en {
            return Ok(Some("cierre_control_no_disponible".into()));
        }
    } else {
        return Ok(Some("limite_no_disponible".into()));
    }
    let comprobacion = h["comprobacion_instancia_id"]
        .as_i64()
        .context("episodio automatico sin comprobacion")?;
    if let Some(motivo) = corte_instancia(c, comprobacion, problema, "evidencia")? {
        return Ok(Some(motivo));
    }
    let pasos = rows(
        c,
        "SELECT i.id,i.timestamp,i.grupo_evidencia FROM episodio_automatico_paso ep
         JOIN instancia i ON i.id=ep.instancia_id WHERE ep.episodio_id=?1",
        [candidato.episodio_id],
    )?;
    if pasos.is_empty() {
        return Ok(Some("sin_pasos".into()));
    }
    for paso in pasos {
        if let Some(motivo) = fuera_del_corte(&paso, problema, "paso")? {
            return Ok(Some(motivo));
        }
    }
    Ok(None)
}

fn evaluar(c: &Connection, candidato: &Candidato, problema: &Problema) -> Result<Option<String>> {
    match candidato.origen.as_str() {
        "manual_tarea" => evaluar_manual(c, candidato, problema),
        "automatico_lista" => evaluar_automatico(c, candidato, problema),
        otro => bail!("origen de secuencia desconocido: {otro}"),
    }
}

/// Deriva términos literales del contenido del problema, con el mismo límite
/// que la consulta FTS. No interpreta el texto ni inventa términos. Conserva
/// el término original porque `literal_query` separa antes de pasar a
/// minúsculas; bajar antes podría introducir marcas combinantes y multiplicar
/// los tokens. Ordena y deduplica por la forma en minúsculas. Se detiene antes
/// de superar 16384 bytes y nunca corta un término ni un carácter UTF-8.
fn derivar_consulta(contenido: &str) -> String {
    let mut terminos: Vec<(String, String)> = contenido
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| (s.to_lowercase(), s.to_string()))
        .collect();
    terminos.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    terminos.dedup_by(|a, b| a.0 == b.0);
    let mut salida = String::new();
    let mut incluidos = 0;
    for (_, termino) in terminos {
        if incluidos == MAX_TERMINOS {
            break;
        }
        let separador = usize::from(!salida.is_empty());
        if salida.len() + separador + termino.len() > MAX_CONSULTA_BYTES {
            continue;
        }
        if separador == 1 {
            salida.push(' ');
        }
        salida.push_str(&termino);
        incluidos += 1;
    }
    salida
}

impl Store {
    /// Consulta explícita de solo lectura. No crea eventos de recuperación ni
    /// paquetes, y no modifica el índice FTS.
    pub fn recuperar_solucion(
        &mut self,
        proyecto: &str,
        problema_id: i64,
        tipo_json: &Value,
        consulta: Option<&str>,
        limite: usize,
    ) -> Result<Value> {
        nonempty(proyecto)?;
        ensure!(problema_id > 0, "evento consultante invalido");
        ensure!(
            limite > 0 && limite <= LIMITE_MAXIMO,
            "limite debe estar entre 1 y {LIMITE_MAXIMO}"
        );
        let tipo = TipoObjetivo::desde_json(tipo_json)?;
        let tx = self.conn.transaction()?;
        let problema = cargar_problema(&tx, proyecto, problema_id)?;
        let tipo_sha = tipo.sha256()?;
        let mut excluidos = Vec::new();
        let mut elegido = None;
        for candidato in candidatos(&tx, proyecto, &tipo)? {
            match evaluar(&tx, &candidato, &problema)? {
                None => {
                    let lectura = leer_id(&tx, candidato.id)?;
                    elegido = Some((candidato, lectura));
                    break;
                }
                Some(motivo) => excluidos.push(json!({
                    "clave":candidato.clave,
                    "motivo":motivo
                })),
            }
        }
        let salida = if let Some((candidato, lectura)) = elegido {
            let seleccion = json!({
                "secuencia_id":candidato.id,
                "cantidad_incluidos":candidato.cantidad_incluidos,
                "reciente":candidato.reciente
            });
            json!({
                "version":1,
                "algoritmo":ALGORITMO,
                "proyecto":proyecto,
                "problema":{
                    "instancia_id":problema.instancia_id,
                    "grupo_evidencia":problema.grupo_evidencia,
                    "timestamp":problema.timestamp,
                    "recibido_en":problema.recibido_en
                },
                "tipo_objetivo":tipo.valor(),
                "tipo_objetivo_sha256":tipo_sha,
                "tipo_alcance":tipo.alcance(),
                "estado":"candidata_encontrada",
                "candidato":lectura,
                "candidato_clave":candidato.clave,
                "seleccion":seleccion,
                "orden":ORDEN,
                "vigente":"representante elegible mas corto observado; competencia, promocion y archivo quedan para el punto 6",
                "excluidos":excluidos,
                "respaldo_textual":Value::Null,
                "no_afirma":[
                    "que el camino derivado haya sido reejecutado",
                    "que la repeticion del tipo demuestre generalizacion",
                    "que un conteo de coincidencias sea un conteo de exitos"
                ]
            })
        } else {
            let consulta = match consulta {
                Some(texto) if !texto.trim().is_empty() => texto.to_string(),
                _ => derivar_consulta(&problema.contenido),
            };
            let resultados = if consulta.trim().is_empty() {
                Vec::new()
            } else {
                buscar_con_corte(
                    &tx,
                    &SearchRequest {
                        proyecto: proyecto.into(),
                        consulta: consulta.clone(),
                        origen: None,
                        rol: None,
                        limite,
                    },
                    problema_id,
                    &problema.grupo_evidencia,
                    problema.timestamp,
                )?
            };
            json!({
                "version":1,
                "algoritmo":ALGORITMO,
                "proyecto":proyecto,
                "problema":{
                    "instancia_id":problema.instancia_id,
                    "grupo_evidencia":problema.grupo_evidencia,
                    "timestamp":problema.timestamp,
                    "recibido_en":problema.recibido_en
                },
                "tipo_objetivo":tipo.valor(),
                "tipo_objetivo_sha256":tipo_sha,
                "tipo_alcance":tipo.alcance(),
                "estado":"sin_candidato",
                "candidato":Value::Null,
                "orden":ORDEN,
                "vigente":Value::Null,
                "excluidos":excluidos,
                "respaldo_textual":{
                    "tipo":"fts5",
                    "consulta":consulta,
                    "corte":{
                        "instancia_id_exclusiva":problema.instancia_id,
                        "timestamp_maximo":problema.timestamp,
                        "grupo_excluido":problema.grupo_evidencia
                    },
                    "resultados":resultados,
                    "nota":"respaldo textual; no implica equivalencia de tipo de objetivo ni causalidad"
                },
                "no_afirma":[
                    "que el camino derivado haya sido reejecutado",
                    "que la repeticion del tipo demuestre generalizacion",
                    "que un conteo de coincidencias sea un conteo de exitos"
                ]
            })
        };
        tx.commit()?;
        Ok(salida)
    }
}
