//! Cortes por listas observadas. Contratos y límites: README, «Cortes automáticos».
use crate::episodios::normalizar_herramienta;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Entrada interna. `fin_sesion` representa evidencia del registro de control;
/// no se presenta como un payload de hook que hubiera sido capturado.
#[derive(Debug, Clone)]
pub struct EventoCorte {
    pub chat: String,
    pub payload: Value,
    pub fin_sesion: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Comprobacion {
    pub evento: usize,
    pub resultado: Option<bool>,
}

/// Los índices son locales al chat filtrado, incluidos sus límites de control.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Tramo {
    pub objetivo: String,
    pub fuente: String,
    pub inicio: usize,
    pub fin: Option<usize>,
    pub motivo_fin: Option<String>,
    pub herramientas: Vec<usize>,
    pub comprobacion: Option<Comprobacion>,
}

fn payload(v: &Value) -> &Value {
    v.pointer("/metadatos/payload_hook")
        .filter(|p| p.is_object())
        .unwrap_or(v)
}

fn texto(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.trim().is_empty())
}

/// Reconoce sólo invocaciones simples. Un shell compuesto o --help no acredita
/// una comprobación. La versión de esta regla se persiste con el resultado.
pub fn es_comprobacion(p: &Value) -> bool {
    if p["tool_name"] != "Bash" {
        return false;
    }
    let Some(comando) = p["tool_input"]["command"].as_str() else {
        return false;
    };
    if comando.chars().any(|c| "\n\r;&|<>$`\"'".contains(c)) {
        return false;
    }
    let palabras: Vec<&str> = comando.split_whitespace().collect();
    if palabras.iter().any(|p| {
        matches!(
            p.split('=').next().unwrap_or(""),
            "--help"
                | "-h"
                | "--version"
                | "-V"
                | "--no-run"
                | "--collect-only"
                | "--list-tests"
                | "--list"
                | "--co"
                | "--collectonly"
                | "--dry-run"
                | "--ignore-scripts"
        )
    }) {
        return false;
    }
    let normal = normalizar_herramienta(p);
    match normal.tipo_destino.as_str() {
        "comando:cargo" => palabras.get(1) == Some(&"test"),
        "comando:npm" | "comando:pnpm" | "comando:yarn" => palabras.get(1) == Some(&"test"),
        "comando:pytest" => true,
        "comando:python" | "comando:python3" => palabras.get(1..3) == Some(&["-m", "pytest"]),
        _ => false,
    }
}

pub(crate) fn resultado(p: &Value) -> Option<bool> {
    let respuesta = &p["tool_response"];
    if p["tool_input"]["run_in_background"] == true
        || p["is_interrupt"] == true
        || respuesta["interrupted"] == true
        || !respuesta["backgroundTaskId"].is_null()
        || respuesta["backgroundedByUser"] == true
        || !respuesta["timedOutAfterMs"].is_null()
    {
        return None;
    }
    match p["hook_event_name"].as_str() {
        Some("PostToolUse") if respuesta["interrupted"] == false => Some(true),
        Some("PostToolUseFailure") => Some(false),
        _ => None,
    }
}

/// Una lista válida da (objetivo activo, objetivos completados). Las formas no
/// documentadas se conservan en el crudo y no se interpretan como transiciones.
type CambioLista = (Option<(String, String)>, BTreeSet<String>);
fn lista(p: &Value, tareas: &mut BTreeMap<String, String>) -> Option<CambioLista> {
    let entrada = &p["tool_input"];
    match p["tool_name"].as_str()? {
        "TodoWrite" => {
            let todos = entrada["todos"].as_array()?;
            let mut vistos = BTreeSet::new();
            let mut completados = BTreeSet::new();
            let mut activo = None;
            for t in todos {
                let contenido = texto(&t["content"])?;
                let clave = format!("todo:{contenido}");
                if !vistos.insert(clave.clone()) {
                    return None;
                }
                match t["status"].as_str()? {
                    "in_progress" => {
                        if activo.is_some() {
                            return None;
                        }
                        activo = Some((clave, contenido.into()));
                    }
                    "completed" => {
                        completados.insert(clave);
                    }
                    "pending" => {}
                    _ => return None,
                }
            }
            Some((activo, completados))
        }
        "TaskCreate" => {
            let t = &p["tool_response"]["task"];
            tareas.insert(texto(&t["id"])?.into(), texto(&t["subject"])?.into());
            None
        }
        "TaskUpdate" => {
            let id = texto(&entrada["taskId"])?;
            let respuesta = &p["tool_response"];
            if respuesta["success"] != true || respuesta["taskId"] != id {
                return None;
            }
            if let Some(subject) = texto(&entrada["subject"]) {
                tareas.insert(id.into(), subject.into());
            }
            let clave = format!("task:{id}");
            match entrada["status"].as_str()? {
                "in_progress" => Some((Some((clave, tareas.get(id)?.clone())), BTreeSet::new())),
                "completed" => Some((None, BTreeSet::from([clave]))),
                _ => None,
            }
        }
        _ => None,
    }
}

fn cerrar(tramos: &mut Vec<Tramo>, actual: &mut Option<(String, Tramo)>, fin: usize, motivo: &str) {
    if let Some((_, mut tramo)) = actual.take() {
        tramo.fin = Some(fin);
        tramo.motivo_fin = Some(motivo.into());
        tramos.push(tramo);
    }
}

/// Total y determinista. No asigna una observación a más de un tramo. Un ID de
/// llamada ya observado no abre otro tramo ni agrega otro paso al reaparecer.
pub fn detectar_tramos(chat: &str, eventos: &[EventoCorte]) -> Vec<Tramo> {
    let mut tramos = Vec::new();
    let mut actual: Option<(String, Tramo)> = None;
    let mut tareas = BTreeMap::new();
    let mut llamadas = BTreeSet::new();
    for (indice, evento) in eventos.iter().filter(|e| e.chat == chat).enumerate() {
        let p = payload(&evento.payload);
        if texto(&p["agent_id"]).is_some() {
            continue;
        }
        if evento.fin_sesion || p["hook_event_name"] == "SessionEnd" {
            cerrar(&mut tramos, &mut actual, indice, "sesion_terminada");
            continue;
        }
        let hook = p["hook_event_name"].as_str().unwrap_or("");
        if !matches!(hook, "PostToolUse" | "PostToolUseFailure") {
            continue;
        }
        if let Some(id) = texto(&p["tool_use_id"]) {
            if !llamadas.insert(id.to_string()) {
                continue;
            }
        }
        let herramienta = p["tool_name"].as_str().unwrap_or("");
        if matches!(
            herramienta,
            "TodoWrite" | "TaskCreate" | "TaskUpdate" | "TaskGet" | "TaskList"
        ) {
            if hook == "PostToolUse" {
                if let Some((siguiente, completados)) = lista(p, &mut tareas) {
                    if actual
                        .as_ref()
                        .is_some_and(|(id, _)| completados.contains(id))
                    {
                        cerrar(&mut tramos, &mut actual, indice, "completado");
                    }
                    if let Some((id, objetivo)) = siguiente {
                        if actual.as_ref().is_none_or(|(anterior, _)| *anterior != id) {
                            cerrar(&mut tramos, &mut actual, indice, "siguiente");
                            actual = Some((
                                id,
                                Tramo {
                                    objetivo,
                                    fuente: herramienta.into(),
                                    inicio: indice,
                                    fin: None,
                                    motivo_fin: None,
                                    herramientas: Vec::new(),
                                    comprobacion: None,
                                },
                            ));
                        }
                    }
                }
            }
            continue;
        }
        if let Some((_, tramo)) = &mut actual {
            tramo.herramientas.push(indice);
            if es_comprobacion(p) {
                tramo.comprobacion = Some(Comprobacion {
                    evento: indice,
                    resultado: resultado(p),
                });
            }
        }
    }
    if let Some((_, tramo)) = actual {
        tramos.push(tramo);
    }
    tramos
}
