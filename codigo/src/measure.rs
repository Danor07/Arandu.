//! Instantáneas consistentes de una memoria y preparación de corridas aisladas.
//! Nada de esto lanza Claude, compra llamadas ni ejecuta comandos del manifiesto.
use crate::control::{normalizar_raiz, Registry, ADAPTADOR_VERSION, HOST_PROBADO};
use crate::{digest, now};
use anyhow::{ensure, Context, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const SNAPSHOT_VERSION: u32 = 1;

fn sha256_archivo(path: &Path) -> Result<String> {
    Ok(digest(&std::fs::read(path)?))
}

pub fn ruta_manifiesto(destino: &Path) -> PathBuf {
    let mut nombre = destino.as_os_str().to_os_string();
    nombre.push(".manifiesto.json");
    PathBuf::from(nombre)
}

/// Corte leído **de la copia**, no del original.
fn corte(conn: &Connection) -> Result<Value> {
    let (instancias, maximo, ultimo): (i64, Option<i64>, Option<i64>) = conn.query_row(
        "SELECT count(*),max(id),max(timestamp) FROM instancia",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let paquetes: i64 = conn.query_row("SELECT count(*) FROM paquete_recuperacion", [], |r| {
        r.get(0)
    })?;
    let entregas: i64 = conn.query_row("SELECT count(*) FROM entrega_contexto_host", [], |r| {
        r.get(0)
    })?;
    let tareas: i64 = conn.query_row("SELECT count(*) FROM tarea", [], |r| r.get(0))?;
    let mut stmt = conn.prepare("SELECT DISTINCT proyecto FROM instancia_proyecto p JOIN instancia i ON i.id=p.instancia_id WHERE proyecto IS NOT NULL ORDER BY proyecto")?;
    let proyectos: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(json!({
        "instancias": instancias,
        "instancia_id_maxima": maximo,
        "timestamp_fuente_maximo": ultimo,
        "paquetes": paquetes,
        "entregas_contexto": entregas,
        "tareas": tareas,
        "proyectos_con_antecedentes": proyectos,
        "obtenido_de": "la copia ya cerrada, no la base viva"
    }))
}

fn migraciones(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT version,sha256 FROM schema_migrations ORDER BY version")?;
    let filas = stmt.query_map([], |r| {
        Ok(json!({"version":r.get::<_,i64>(0)?,"sha256":r.get::<_,String>(1)?}))
    })?;
    Ok(filas.collect::<rusqlite::Result<_>>()?)
}

/// Copia consistente con `VACUUM INTO`, no copiando el archivo de una base WAL
/// activa. Una caída deja un `.parcial` identificable, nunca un snapshot final
/// aparentemente válido.
pub fn crear_snapshot(
    registry: &Registry,
    memoria_id: &str,
    destino: &Path,
    nota: &str,
) -> Result<Value> {
    ensure!(
        !destino.exists(),
        "el destino ya existe; un snapshot no sobrescribe: {}",
        destino.display()
    );
    let manifiesto = ruta_manifiesto(destino);
    ensure!(
        !manifiesto.exists(),
        "ya existe un manifiesto para ese destino"
    );
    if let Some(padre) = destino.parent() {
        std::fs::create_dir_all(padre)?;
    }
    let mut parcial = destino.as_os_str().to_os_string();
    parcial.push(".parcial");
    let parcial = PathBuf::from(parcial);
    let _ = std::fs::remove_file(&parcial);

    let store = registry.open_memory(memoria_id)?;
    let nombre = registry.memory_name(memoria_id)?;
    store.vacuum_into(&parcial)?;
    drop(store);

    // La copia se valida cerrada y sin tocar el original.
    let copia = Connection::open(&parcial)?;
    let integridad: String = copia.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(integridad == "ok", "la copia no pasa integrity_check");
    copia.execute_batch("PRAGMA foreign_key_check")?;
    let identidad: Option<String> = copia
        .query_row("SELECT memoria_id FROM memoria_identidad", [], |r| r.get(0))
        .ok();
    ensure!(
        identidad.as_deref() == Some(memoria_id),
        "la copia no conserva la identidad sellada del origen"
    );
    let corte = corte(&copia)?;
    let migraciones = migraciones(&copia)?;
    drop(copia);

    let hash = sha256_archivo(&parcial)?;
    let bytes = std::fs::metadata(&parcial)?.len();
    let snapshot_id = digest(format!("{memoria_id}/{hash}/{bytes}").as_bytes())[..32].to_string();
    // Recién acá el snapshot pasa a existir con su nombre final.
    std::fs::rename(&parcial, destino)?;
    let documento = json!({
        "version": SNAPSHOT_VERSION,
        "snapshot_id": snapshot_id,
        "memoria_origen": {"memoria_id": memoria_id, "nombre": nombre},
        "archivo": destino.file_name().map(|n| n.to_string_lossy().into_owned()),
        "sha256": hash,
        "bytes": bytes,
        "migraciones": migraciones,
        "corte": corte,
        "configuracion": {"adaptador": ADAPTADOR_VERSION, "host_probado": HOST_PROBADO,
                          "tecnica": "VACUUM INTO"},
        "procedencia": "copia local de una memoria de este equipo",
        "nota": nota,
        "creado_en": now()?,
        "limite": "no incluye el registro operativo; no es restaurable como instalacion en otro equipo"
    });
    std::fs::write(&manifiesto, serde_json::to_string_pretty(&documento)?)?;
    Ok(documento)
}

/// Comprueba hash e integridad antes de usar una copia. No migra ni escribe.
pub fn validar_snapshot(destino: &Path) -> Result<Value> {
    let manifiesto = ruta_manifiesto(destino);
    let documento: Value = serde_json::from_slice(
        &std::fs::read(&manifiesto)
            .with_context(|| format!("falta el manifiesto {}", manifiesto.display()))?,
    )?;
    ensure!(
        documento["version"] == SNAPSHOT_VERSION,
        "version de manifiesto no soportada"
    );
    let hash = sha256_archivo(destino)?;
    ensure!(
        documento["sha256"] == hash,
        "el archivo del snapshot no coincide con su hash: posible corrupcion"
    );
    let conn = Connection::open(destino)?;
    let integridad: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(integridad == "ok", "el snapshot no pasa integrity_check");
    Ok(json!({"snapshot_id":documento["snapshot_id"],"sha256":hash,
        "integridad":"ok","corte":documento["corte"],"valido":true}))
}

// --------------------------------------------------------------------- casos

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condicion {
    pub nombre: String,
    pub modo: String,
    /// Ruta al snapshot que usa esta condición.
    pub snapshot: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Caso {
    pub id: String,
    pub prompt: String,
    pub criterios: Vec<Value>,
    /// Commit y artefacto reproducible; sin dato se declara, no se completa.
    pub estado_inicial_codigo: Value,
    pub configuracion: Value,
    /// `true` si el caso o su solución ya está en el snapshot.
    pub exposicion_previa: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Casos {
    pub version: u32,
    /// Raíz de trabajo donde se ejecutará cada corrida.
    pub raiz_trabajo: String,
    pub repeticiones: u32,
    pub condiciones: Vec<Condicion>,
    pub casos: Vec<Caso>,
}

/// Prepara caso × condición × repetición en raíces aisladas. No abre sesiones
/// históricas, no reanuda trabajo pendiente copiado y no toca el snapshot.
pub fn preparar_corridas(casos: &Path, salida: &Path) -> Result<Value> {
    let manifiesto: Casos =
        serde_json::from_slice(&std::fs::read(casos)?).context("manifiesto de casos invalido")?;
    ensure!(manifiesto.version == 1, "version de casos no soportada");
    ensure!(
        manifiesto.repeticiones >= 1,
        "se exige al menos una repeticion"
    );
    ensure!(!manifiesto.casos.is_empty(), "no hay casos");
    ensure!(!manifiesto.condiciones.is_empty(), "no hay condiciones");
    ensure!(
        !salida.exists() || salida.read_dir()?.next().is_none(),
        "el directorio de salida ya tiene contenido; una preparacion no sobrescribe"
    );
    let base = casos.parent().unwrap_or(Path::new("."));
    let (clave_trabajo, _) = normalizar_raiz(Path::new(&manifiesto.raiz_trabajo))
        .context("raiz_trabajo inexistente: crear la copia de trabajo antes de preparar")?;

    let mut corridas = Vec::new();
    for caso in &manifiesto.casos {
        for condicion in &manifiesto.condiciones {
            crate::control::modo_valido(&condicion.modo)?;
            let origen = base.join(&condicion.snapshot);
            let validacion = validar_snapshot(&origen)
                .with_context(|| format!("snapshot invalido para {}", condicion.nombre))?;
            let documento: Value =
                serde_json::from_slice(&std::fs::read(ruta_manifiesto(&origen))?)?;
            let memoria_id = documento["memoria_origen"]["memoria_id"]
                .as_str()
                .context("manifiesto sin memoria de origen")?
                .to_string();
            // Los antecedentes del snapshot se filtran por proyecto: si la raíz
            // de trabajo normaliza distinto, no son elegibles y se declara.
            let proyectos: Vec<String> = documento["corte"]["proyectos_con_antecedentes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let elegibles = proyectos.contains(&clave_trabajo);
            let desde = documento["corte"]["instancia_id_maxima"]
                .as_i64()
                .unwrap_or(0);

            for repeticion in 1..=manifiesto.repeticiones {
                let dir = salida
                    .join(&caso.id)
                    .join(&condicion.nombre)
                    .join(format!("r{repeticion}"));
                std::fs::create_dir_all(dir.join("datos").join("memorias"))?;
                let copia = dir
                    .join("datos")
                    .join("memorias")
                    .join(format!("{memoria_id}.sqlite"));
                // Copia de trabajo: conserva el linaje sellado dentro de un
                // registro aislado; la corrida tiene su propia identidad.
                std::fs::copy(&origen, &copia)?;
                let mut registro = Registry::open(dir.join("datos"))?;
                registro.adopt_memory(&memoria_id, "copia de trabajo de un snapshot")?;
                registro.register_project(Path::new(&manifiesto.raiz_trabajo))?;
                let corrida_id =
                    digest(format!("{}/{}/{repeticion}", caso.id, condicion.nombre).as_bytes())
                        [..24]
                        .to_string();
                let documento_corrida = json!({
                    "version": 1,
                    "corrida_id": corrida_id,
                    "caso": caso,
                    "condicion": condicion,
                    "repeticion": repeticion,
                    "snapshot": {"archivo": origen.to_string_lossy(),
                                 "snapshot_id": documento["snapshot_id"],
                                 "sha256": validacion["sha256"],
                                 "corte": documento["corte"]},
                    "memoria_id": memoria_id,
                    "raiz_datos": dir.join("datos").to_string_lossy(),
                    "raiz_trabajo": manifiesto.raiz_trabajo,
                    "proyecto_normalizado": clave_trabajo,
                    "antecedentes_elegibles": elegibles,
                    "motivo_no_elegible": if elegibles { Value::Null } else {
                        json!("la raiz de trabajo normaliza distinto que el proyecto de los antecedentes; esta condicion no puede recuperar nada")
                    },
                    "solo_eventos_desde_instancia_id": desde,
                    "instrucciones": [
                        format!("cd {:?}", manifiesto.raiz_trabajo),
                        format!("$env:MSP_RAIZ = {:?}", dir.join("datos")),
                        "claude --plugin-dir <ruta del plugin>   (chat NUEVO, una sola sesion)",
                        match condicion.modo.as_str() {
                            "solo_registro" => format!("msp:: registrar {memoria_id}"),
                            _ => format!("msp:: usar {memoria_id}"),
                        },
                        "Fijar el modo ANTES del primer prompt ordinario; cambiarlo despues marca la corrida no comparable.",
                        format!("Prompt del caso: {}", caso.prompt)
                    ],
                    "limites": [
                        "esta herramienta no lanza Claude ni ejecuta comandos del manifiesto",
                        "los eventos de esta corrida quedan en la copia y no alimentan el snapshot ni otra corrida",
                        "la exclusion de episodio impide que la propia corrida se recupere a si misma",
                        "BM25 usa estadisticas del indice local: el ranking no es una constante congelada"
                    ],
                    "preparada_en": now()?
                });
                std::fs::write(
                    dir.join("corrida.json"),
                    serde_json::to_string_pretty(&documento_corrida)?,
                )?;
                corridas.push(documento_corrida);
            }
        }
    }
    let indice = json!({"version":1,"preparadas":corridas.len(),"salida":salida.to_string_lossy(),
        "corridas":corridas,"preparado_en":now()?});
    std::fs::write(
        salida.join("indice.json"),
        serde_json::to_string_pretty(&indice)?,
    )?;
    Ok(indice)
}

/// Tabla descriptiva por caso/condición/repetición. Calidad primero, esfuerzo
/// observado después, y todo lo que difiere marcado como no comparable.
pub fn tabla_comparativa(salida: &Path) -> Result<Value> {
    let indice: Value = serde_json::from_slice(
        &std::fs::read(salida.join("indice.json"))
            .context("falta indice.json de la preparacion")?,
    )?;
    let mut filas = Vec::new();
    let mut criterios_por_caso: std::collections::BTreeMap<String, Value> = Default::default();
    let mut codigo_por_caso: std::collections::BTreeMap<String, Value> = Default::default();
    for corrida in indice["corridas"].as_array().cloned().unwrap_or_default() {
        let caso = corrida["caso"]["id"].as_str().unwrap_or("").to_string();
        let condicion = corrida["condicion"]["nombre"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let dir = salida
            .join(&caso)
            .join(&condicion)
            .join(format!("r{}", corrida["repeticion"]));
        let informe: Option<Value> = std::fs::read(dir.join("informe.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());

        // Parámetros que deben coincidir entre condiciones del mismo caso.
        let criterios = corrida["caso"]["criterios"].clone();
        let codigo = corrida["caso"]["estado_inicial_codigo"].clone();
        let criterio_distinto = criterios_por_caso
            .get(&caso)
            .is_some_and(|c| *c != criterios);
        let codigo_distinto = codigo_por_caso.get(&caso).is_some_and(|c| *c != codigo);
        criterios_por_caso.entry(caso.clone()).or_insert(criterios);
        codigo_por_caso.entry(caso.clone()).or_insert(codigo);

        let mut motivos: Vec<String> = Vec::new();
        if informe.is_none() {
            motivos.push("corrida pendiente: falta informe.json".into());
        }
        if corrida["antecedentes_elegibles"] == false && condicion != "sin_recuperacion" {
            motivos.push("los antecedentes del snapshot no son elegibles en esta raiz".into());
        }
        if criterio_distinto {
            motivos.push("criterio distinto al de otra condicion del mismo caso".into());
        }
        if codigo_distinto {
            motivos.push("estado inicial del codigo distinto al de otra condicion".into());
        }
        if corrida["caso"]["configuracion"]["modelo"].is_null() {
            motivos.push("modelo/configuracion desconocidos: declarado, no supuesto".into());
        }
        if corrida["caso"]["exposicion_previa"] == true {
            motivos.push("exposicion previa: el caso o su solucion ya estan en el snapshot".into());
        }
        if let Some(i) = &informe {
            let modos = i["modos_observados"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(1);
            if modos > 1 {
                motivos.push("el modo cambio a mitad del chat".into());
            }
        }
        let calidad = informe.as_ref().map(|i| i["calidad"].clone());
        filas.push(json!({
            "caso": caso, "condicion": condicion, "repeticion": corrida["repeticion"],
            "estado": if informe.is_some() { "con_informe" } else { "pendiente" },
            "comparable": motivos.is_empty(),
            "motivos_no_comparable": motivos,
            "calidad": calidad,
            "esfuerzo_observado": informe.as_ref().map(|i| json!({
                "operaciones_de_herramienta": i["herramientas"]["operaciones"].as_array().map(|a| a.len()),
                "cobertura_herramientas": i["herramientas"]["cobertura"],
                "paquetes_preparados": i["memoria"]["paquetes_preparados"],
                "entregas_completadas": i["memoria"]["entregas_escritura_completada"],
                "entregas_inciertas": i["memoria"]["entregas_inciertas"],
                "ventana_observada_total": i["tiempo"]["ventana_observada_total"],
                "tokens": i["tokens"]
            })),
            "antecedentes_elegibles": corrida["antecedentes_elegibles"],
            "snapshot_id": corrida["snapshot"]["snapshot_id"],
            "exposicion_previa": corrida["caso"]["exposicion_previa"]
        }));
    }
    Ok(json!({
        "version": 1,
        "filas": filas,
        "pendientes": filas.iter().filter(|f| f["estado"] == "pendiente").count(),
        "lectura": "tabla descriptiva: una observacion no demuestra mejora general ni significacion estadistica",
        "nota": "una condicion faltante queda pendiente; nada se completa por suposicion",
        "generada_en": now()?
    }))
}
