//! Secuencia mínima: recorrido observado, descartes explícitos y lectura inmutable.
use msp::secuencia::{
    calcular_descartes, decidir_estado, PasoObservado, ReferenciaDescarte, REF_DESCARTA,
    REF_REVIERTE,
};
use msp::tasks::{Criterio, TaskRecord};
use msp::Store;
use proptest::prelude::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::process::Command;

fn tarea(s: &mut Store) {
    s.record_task(&TaskRecord::TareaCrear {
        tarea: "t".into(),
        proyecto: "p".into(),
        definicion: "def/1".into(),
        descripcion: "compilar sin errores".into(),
        criterios: vec![Criterio {
            id: "prueba".into(),
            version: "1".into(),
            descripcion: "la suite pasa".into(),
            obligatorio: true,
        }],
    })
    .unwrap();
}

fn marcar(s: &mut Store, tipo: &str) -> i64 {
    s.record_task(&TaskRecord::TareaMarcar {
        clave: format!("marca/{tipo}"),
        tarea: "t".into(),
        tipo: tipo.into(),
        detalle: json!({}),
    })
    .unwrap()["registrado_en"]
        .as_i64()
        .unwrap()
}

fn herramienta(
    s: &mut Store,
    clave: &str,
    grupo: &str,
    secuencia: i64,
    estado: &str,
    referencias: Value,
    payload: Value,
) -> i64 {
    let crudo = json!({
        "version":1,
        "clave_evento":clave,
        "grupo_evidencia":grupo,
        "secuencia":secuencia,
        "timestamp":secuencia,
        "origen":"herramienta",
        "rol":"otro",
        "contenido":"crudo completo no normalizado",
        "metadatos":{"proyecto":"p","tipo":estado,"payload_hook":payload},
        "referencias":referencias
    });
    let id = s.capture(&crudo.to_string()).unwrap().instancia_id;
    s.record_task(&TaskRecord::TareaEnlazar {
        tarea: "t".into(),
        evento: clave.into(),
        evidencia: "asociacion explicita".into(),
    })
    .unwrap();
    id
}

fn resultado(s: &mut Store, clave: &str, valor: i64, anterior: Option<&str>) {
    let evidencia = format!("evidencia/{clave}");
    let evaluacion = json!({
        "version":1,"fuente":"usuario_explicito","procedencia":"real",
        "criterio":"prueba","criterio_version":"1","alcance":"tarea","objetivo":null,
        "resultado":valor,"detalle":{"tarea":"t"},"nota":"fixture","anterior":null
    });
    s.capture(
        &json!({"version":1,"clave_evento":evidencia,"grupo_evidencia":evidencia,
            "secuencia":1,"timestamp":1,"origen":"usuario","rol":"otro",
            "contenido":"verificacion externa explicita",
            "metadatos":{"proyecto":"p","procedencia":"real","evaluacion":evaluacion}})
        .to_string(),
    )
    .unwrap();
    s.record_task(&TaskRecord::TareaResultado {
        clave: clave.into(),
        tarea: "t".into(),
        criterio: "prueba".into(),
        criterio_version: "1".into(),
        evidencia,
        alcance: "tarea".into(),
        objetivo: None,
        anterior: anterior.map(str::to_string),
    })
    .unwrap();
}

fn cerrar(s: &mut Store, clave: &str, valor: i64) {
    resultado(s, clave, valor, None);
    marcar(s, "cierre");
}

fn lista(a: &str, b: &str) -> Value {
    let todos = json!([
        {"content":"arreglar suma","status":a,"activeForm":"arreglando suma"},
        {"content":"documentar","status":b,"activeForm":"documentando"}]);
    json!({"hook_event_name":"PostToolUse","tool_name":"TodoWrite","tool_input":{"todos":todos},
        "tool_response":{"oldTodos":[],"newTodos":todos}})
}

fn bash(comando: &str, exito: bool) -> Value {
    let mut p = json!({"hook_event_name":if exito {"PostToolUse"} else {"PostToolUseFailure"},"tool_name":"Bash",
        "tool_input":{"command":comando},"tool_response":{"stdout":"salida","stderr":"","interrupted":false}});
    if !exito {
        p["error"] = json!("Exit code 1\nfallo de la comprobacion");
        p["is_interrupt"] = json!(false);
        p.as_object_mut().unwrap().remove("tool_response");
    }
    p
}

fn capturar_auto(s: &mut Store, secuencia: i64, payload: Value, referencias: Value) -> i64 {
    s.capture(
        &json!({"version":1,"clave_evento":format!("chat/{secuencia}"),
            "grupo_evidencia":"chat","secuencia":secuencia,"timestamp":secuencia,
            "origen":"herramienta","rol":"otro","contenido":"texto crudo intacto",
            "metadatos":{"proyecto":"p","intervalo_captura":7,"payload_hook":payload},
            "referencias":referencias})
        .to_string(),
    )
    .unwrap()
    .instancia_id
}

fn paso(orden: i64, grupo: &str, estado: &str, referencias: &[(&str, &str)]) -> PasoObservado {
    PasoObservado {
        orden,
        instancia_id: orden,
        clave_evento: format!("e{orden}"),
        herramienta: "Read".into(),
        tipo_destino: "archivo:rs".into(),
        grupo_evidencia: grupo.into(),
        timestamp: orden,
        estado_tecnico: estado.into(),
        referencias: referencias
            .iter()
            .map(|(tipo, clave)| ReferenciaDescarte {
                tipo: (*tipo).into(),
                clave_evento: (*clave).into(),
            })
            .collect(),
    }
}

fn motivo(descartes: &[msp::secuencia::DescarteCalculado], indice: usize) -> &str {
    descartes[indice]
        .motivo_no_aplicada
        .as_deref()
        .expect("descarte sin motivo")
}

#[test]
fn decision_pura_no_confunde_candidata_con_reejecucion() {
    assert_eq!(decidir_estado(None, false, 3, 0).estado, "no_candidata");
    assert_eq!(decidir_estado(None, false, 3, 0).motivo, "sin_resultado");
    assert_eq!(
        decidir_estado(Some(0), false, 3, 0).motivo,
        "resultado_fallido"
    );
    assert_eq!(
        decidir_estado(Some(1), true, 3, 0).motivo,
        "resultado_corregido"
    );
    assert_eq!(
        decidir_estado(Some(1), false, 0, 0).motivo,
        "sin_pasos_observados"
    );
    let sin = decidir_estado(Some(1), false, 3, 0);
    assert_eq!(sin.estado, "candidata_sin_descartes");
    assert!(sin.acredita_episodio_fuente);
    let con = decidir_estado(Some(1), false, 3, 1);
    assert_eq!(con.estado, "candidata_con_descartes");
    assert!(con.acredita_episodio_fuente);
    assert!(!con.motivo.contains("reejecut"));
}

#[test]
fn descarte_aplicado_exige_evidencia_ok_posterior_y_del_mismo_grupo() {
    let aplicado = calcular_descartes(
        &[
            paso(1, "g", "ok", &[]),
            paso(2, "g", "ok", &[("descarta_paso", "e1")]),
        ],
        None,
    );
    assert_eq!(aplicado.len(), 1);
    assert!(aplicado[0].aplicada);
    assert_eq!(aplicado[0].motivo_no_aplicada, None);
    assert_eq!(aplicado[0].tipo_referencia, REF_DESCARTA);
    assert_eq!(
        (aplicado[0].orden_descartado, aplicado[0].orden_evidencia),
        (1, 2)
    );

    let revierte = calcular_descartes(
        &[
            paso(1, "g", "ok", &[]),
            paso(2, "g", "ok", &[(REF_REVIERTE, "e1")]),
        ],
        None,
    );
    assert!(revierte[0].aplicada);
    assert_eq!(revierte[0].tipo_referencia, REF_REVIERTE);

    for (estado, esperado) in [
        ("error", "evidencia_fallida_o_desconocida"),
        ("desconocido", "evidencia_fallida_o_desconocida"),
    ] {
        let d = calcular_descartes(
            &[
                paso(1, "g", "ok", &[]),
                paso(2, "g", estado, &[("descarta_paso", "e1")]),
            ],
            None,
        );
        assert!(!d[0].aplicada);
        assert_eq!(motivo(&d, 0), esperado);
    }

    let otro_grupo = calcular_descartes(
        &[
            paso(1, "a", "ok", &[]),
            paso(2, "b", "ok", &[("descarta_paso", "e1")]),
        ],
        None,
    );
    assert!(!otro_grupo[0].aplicada);
    assert_eq!(motivo(&otro_grupo, 0), "grupo_evidencia_distinto");
}

#[test]
fn referencias_invalidas_se_conservan_no_aplicadas() {
    let adelante = calcular_descartes(
        &[
            paso(1, "g", "ok", &[("descarta_paso", "e2")]),
            paso(2, "g", "ok", &[]),
        ],
        None,
    );
    assert!(!adelante[0].aplicada);
    assert_eq!(motivo(&adelante, 0), "evidencia_no_posterior");

    let mismo = calcular_descartes(&[paso(1, "g", "ok", &[("descarta_paso", "e1")])], None);
    assert!(!mismo[0].aplicada);
    assert_eq!(motivo(&mismo, 0), "referencia_al_mismo_paso");

    let protegido = calcular_descartes(
        &[
            paso(1, "g", "ok", &[]),
            paso(2, "g", "ok", &[("descarta_paso", "e1")]),
        ],
        Some(1),
    );
    assert!(!protegido[0].aplicada);
    assert_eq!(motivo(&protegido, 0), "comprobacion_del_resultado");

    let evidencia_descartada = calcular_descartes(
        &[
            paso(1, "g", "ok", &[]),
            paso(2, "g", "ok", &[("descarta_paso", "e1")]),
            paso(3, "g", "ok", &[("descarta_paso", "e2")]),
        ],
        None,
    );
    let de_b = evidencia_descartada
        .iter()
        .find(|d| d.evidencia_instancia_id == 2)
        .unwrap();
    assert!(!de_b.aplicada);
    assert_eq!(
        de_b.motivo_no_aplicada.as_deref(),
        Some("evidencia_descartada")
    );

    let ya_descartado = calcular_descartes(
        &[
            paso(1, "g", "ok", &[]),
            paso(2, "g", "ok", &[("descarta_paso", "e1")]),
            paso(3, "g", "ok", &[("descarta_paso", "e1")]),
        ],
        None,
    );
    let redundante = ya_descartado
        .iter()
        .find(|d| d.evidencia_instancia_id == 2)
        .unwrap();
    assert!(!redundante.aplicada);
    assert_eq!(
        redundante.motivo_no_aplicada.as_deref(),
        Some("paso_ya_descartado")
    );
}

#[test]
fn manual_sin_descartes_conserva_todo_incluso_un_fallo_aislado() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    tarea(&mut s);
    marcar(&mut s, "inicio");
    let a = herramienta(
        &mut s,
        "a",
        "g",
        1,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    let b = herramienta(
        &mut s,
        "b",
        "g",
        2,
        "herramienta-error",
        json!([]),
        json!({"tool_name":"Bash","tool_input":{"command":"false"}}),
    );
    let c = herramienta(
        &mut s,
        "c",
        "g",
        3,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"b.rs"}}),
    );
    cerrar(&mut s, "verificado", 1);
    s.construir_episodio("ep", "p", "t", "verificado").unwrap();

    let secuencia = s.construir_secuencia_manual("p", "ep").unwrap();
    assert_eq!(secuencia["secuencia"]["estado"], "candidata_sin_descartes");
    assert_eq!(
        secuencia["secuencia"]["motivo"],
        "recorrido_observado; el resultado acredita el episodio fuente"
    );
    assert_eq!(
        secuencia["tipo_objetivo"],
        json!(["manual_tarea", "t", 1, "prueba", "1"])
    );
    assert_eq!(secuencia["secuencia"]["verificacion"], "criterio_tarea");
    assert_eq!(secuencia["secuencia"]["cantidad_pasos"], 3);
    assert_eq!(secuencia["secuencia"]["cantidad_incluidos"], 3);
    assert_eq!(secuencia["secuencia"]["cantidad_descartados"], 0);
    assert_eq!(secuencia["resumen"]["fallos_observados"], 1);
    assert_eq!(secuencia["resumen"]["fallos_en_camino"], 1);
    assert_eq!(secuencia["seleccionable_actual"], true);
    assert_eq!(secuencia["es_reejecucion"], false);
    assert_eq!(secuencia["resultado_acredita"], "episodio_fuente");
    assert_eq!(secuencia["descartes"], json!([]));
    let pasos = secuencia["pasos"].as_array().unwrap();
    assert_eq!(pasos.len(), 3);
    for (id, incluido) in [(a, true), (b, true), (c, true)] {
        let p = pasos
            .iter()
            .find(|p| p["instancia_id"].as_i64() == Some(id))
            .unwrap();
        assert_eq!(p["incluido"], incluido);
    }
    let fallo = pasos
        .iter()
        .find(|p| p["instancia_id"].as_i64() == Some(b))
        .unwrap();
    assert_eq!(fallo["estado_tecnico"], "error");
    assert_eq!(fallo["fallo_observado"], true);

    assert_eq!(s.construir_secuencia_manual("p", "ep").unwrap(), secuencia);
    let clave = secuencia["secuencia"]["clave"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(s.leer_secuencia("p", &clave).unwrap(), secuencia);
    assert!(s.construir_secuencia_manual("otro", "ep").is_err());
    assert!(s.leer_secuencia("otro", &clave).is_err());

    let c = Connection::open(&path).unwrap();
    let crudo: String = c
        .query_row(
            "SELECT json_original FROM captura_evento WHERE instancia_id=?1",
            [b],
            |r| r.get(0),
        )
        .unwrap();
    assert!(crudo.contains("false"));
}

#[test]
fn descarte_aplicado_aparta_el_paso_y_la_lectura_conserva_ambos() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    tarea(&mut s);
    marcar(&mut s, "inicio");
    let a = herramienta(
        &mut s,
        "a",
        "g",
        1,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    let b = herramienta(
        &mut s,
        "b",
        "g",
        2,
        "herramienta-ok",
        json!([{"tipo":"descarta_paso","clave_evento":"a"}]),
        json!({"tool_name":"Read","tool_input":{"file_path":"b.rs"}}),
    );
    cerrar(&mut s, "verificado", 1);
    s.construir_episodio("ep", "p", "t", "verificado").unwrap();

    let secuencia = s.construir_secuencia_manual("p", "ep").unwrap();
    assert_eq!(secuencia["secuencia"]["estado"], "candidata_con_descartes");
    assert_eq!(secuencia["secuencia"]["cantidad_incluidos"], 1);
    assert_eq!(secuencia["secuencia"]["cantidad_descartados"], 1);
    let pasos = secuencia["pasos"].as_array().unwrap();
    let excluido = pasos
        .iter()
        .find(|p| p["instancia_id"].as_i64() == Some(a))
        .unwrap();
    let incluido = pasos
        .iter()
        .find(|p| p["instancia_id"].as_i64() == Some(b))
        .unwrap();
    assert_eq!(excluido["incluido"], false);
    assert_eq!(excluido["motivo_exclusion"], "descarte_explicito");
    assert_eq!(incluido["incluido"], true);
    let descarte = &secuencia["descartes"][0];
    assert_eq!(descarte["aplicada"], true);
    assert_eq!(descarte["tipo_referencia"], "descarta_paso");
    assert_eq!(descarte["descartado_clave"], "a");
    assert_eq!(descarte["evidencia_clave"], "b");

    let clave = secuencia["secuencia"]["clave"].as_str().unwrap();
    let releida = s.leer_secuencia("p", clave).unwrap();
    assert_eq!(releida, secuencia);
    assert_eq!(releida["descartes"][0]["descartado_clave"], "a");
    assert_eq!(releida["descartes"][0]["evidencia_clave"], "b");

    let listado = s.listar_secuencias("p").unwrap();
    assert_eq!(listado["secuencias"].as_array().unwrap().len(), 1);
    assert_eq!(listado["secuencias"][0]["seleccionable_actual"], true);
    assert_eq!(listado["secuencias"][0]["es_reejecucion"], false);
}

#[test]
fn referencia_no_aplicada_por_fallo_se_conserva_y_el_paso_sigue() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    tarea(&mut s);
    marcar(&mut s, "inicio");
    let a = herramienta(
        &mut s,
        "a",
        "g",
        1,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    let b = herramienta(
        &mut s,
        "b",
        "g",
        2,
        "herramienta-error",
        json!([{"tipo":"descarta_paso","clave_evento":"a"}]),
        json!({"tool_name":"Bash","tool_input":{"command":"false"}}),
    );
    cerrar(&mut s, "verificado", 1);
    s.construir_episodio("ep", "p", "t", "verificado").unwrap();

    let secuencia = s.construir_secuencia_manual("p", "ep").unwrap();
    assert_eq!(secuencia["secuencia"]["estado"], "candidata_sin_descartes");
    assert_eq!(secuencia["secuencia"]["cantidad_descartados"], 0);
    assert_eq!(secuencia["descartes"][0]["aplicada"], false);
    assert_eq!(
        secuencia["descartes"][0]["motivo_no_aplicada"],
        "evidencia_fallida_o_desconocida"
    );
    for id in [a, b] {
        let p = secuencia["pasos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["instancia_id"].as_i64() == Some(id))
            .unwrap();
        assert_eq!(p["incluido"], true);
    }
}

#[test]
fn referencias_se_deduplican_por_par_exacto_y_descartan_tipos_ajenos() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    tarea(&mut s);
    marcar(&mut s, "inicio");
    herramienta(
        &mut s,
        "a",
        "g",
        1,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    herramienta(
        &mut s,
        "b",
        "g",
        2,
        "herramienta-ok",
        json!([
            {"tipo":"descarta_paso","clave_evento":"a"},
            {"tipo":"descarta_paso","clave_evento":"a"},
            {"tipo":"revierte_paso","clave_evento":"a"},
            {"tipo":"descarta_paso","clave_evento":"c"},
            {"tipo":"tipo_desconocido","clave_evento":"a"}
        ]),
        json!({"tool_name":"Read","tool_input":{"file_path":"b.rs"}}),
    );
    herramienta(
        &mut s,
        "c",
        "g",
        3,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"c.rs"}}),
    );
    cerrar(&mut s, "verificado", 1);
    s.construir_episodio("ep", "p", "t", "verificado").unwrap();

    let secuencia = s.construir_secuencia_manual("p", "ep").unwrap();
    let descartes = secuencia["descartes"].as_array().unwrap();
    assert_eq!(
        descartes.len(),
        3,
        "el par identico se deduplica y el tipo ajeno se ignora"
    );
    assert_eq!(
        descartes
            .iter()
            .filter(|d| d["descartado_clave"] == "a" && d["tipo_referencia"] == "descarta_paso")
            .count(),
        1
    );
    assert!(descartes.iter().any(|d| {
        d["descartado_clave"] == "a"
            && d["tipo_referencia"] == "revierte_paso"
            && d["motivo_no_aplicada"] == "paso_ya_descartado"
    }));
    assert!(descartes.iter().any(
        |d| d["descartado_clave"] == "c" && d["motivo_no_aplicada"] == "evidencia_no_posterior"
    ));
    assert!(!descartes
        .iter()
        .any(|d| d["tipo_referencia"] == "tipo_desconocido"));
}

#[test]
fn correccion_posterior_preserva_snapshot_y_expone_estado_actual() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    tarea(&mut s);
    marcar(&mut s, "inicio");
    herramienta(
        &mut s,
        "a",
        "g",
        1,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    cerrar(&mut s, "verificado", 1);
    s.construir_episodio("ep", "p", "t", "verificado").unwrap();
    let secuencia = s.construir_secuencia_manual("p", "ep").unwrap();
    let clave = secuencia["secuencia"]["clave"]
        .as_str()
        .unwrap()
        .to_string();

    resultado(&mut s, "corregido", 0, Some("verificado"));
    let releida = s.leer_secuencia("p", &clave).unwrap();
    assert_eq!(releida["secuencia"]["estado"], "candidata_sin_descartes");
    assert_eq!(releida["secuencia"]["resultado"], 1);
    assert_eq!(releida["corregido_en_vivo"], true);
    assert_eq!(releida["estado_actual"], "corregida");
    assert_eq!(
        releida["motivo_actual"],
        "resultado_corregido_posteriormente"
    );
    assert_eq!(releida["seleccionable_actual"], false);
    assert_eq!(releida["es_reejecucion"], false);
    assert_eq!(releida["resultado_acredita"], Value::Null);

    let listado = s.listar_secuencias("p").unwrap();
    assert_eq!(listado["secuencias"][0]["corregido_en_vivo"], true);
    assert_eq!(listado["secuencias"][0]["seleccionable_actual"], false);
}

#[test]
fn resultado_fallido_no_es_candidata() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    tarea(&mut s);
    marcar(&mut s, "inicio");
    herramienta(
        &mut s,
        "a",
        "g",
        1,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    cerrar(&mut s, "fallido", 0);
    s.construir_episodio("ep", "p", "t", "fallido").unwrap();

    let secuencia = s.construir_secuencia_manual("p", "ep").unwrap();
    assert_eq!(secuencia["secuencia"]["estado"], "no_candidata");
    assert_eq!(secuencia["secuencia"]["motivo"], "resultado_fallido");
    assert_eq!(secuencia["secuencia"]["resultado"], 0);
    assert_eq!(secuencia["seleccionable_actual"], false);
    assert_eq!(secuencia["resultado_acredita"], Value::Null);
}

#[test]
fn manual_sin_resultado_no_se_deriva() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    tarea(&mut s);
    marcar(&mut s, "inicio");
    herramienta(
        &mut s,
        "a",
        "g",
        1,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    cerrar(&mut s, "verificado", 1);
    s.construir_episodio("ep", "p", "t", "verificado").unwrap();

    let c = Connection::open(&path).unwrap();
    c.execute_batch("DROP TRIGGER episodio_resultado_delete; DELETE FROM episodio_resultado;")
        .unwrap();
    drop(c);
    let error = s
        .construir_secuencia_manual("p", "ep")
        .unwrap_err()
        .to_string();
    assert!(error.contains("sin resultado"), "{error}");
}

#[test]
fn automatico_verificado_descarta_solo_con_referencia_posterior() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar_auto(&mut s, 1, lista("in_progress", "pending"), json!([]));
    let lectura = capturar_auto(
        &mut s,
        2,
        json!({"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
        json!([]),
    );
    let eco = capturar_auto(
        &mut s,
        3,
        json!({"hook_event_name":"PostToolUse","tool_name":"Bash","tool_input":{"command":"echo ok"},
            "tool_response":{"stdout":"ok","interrupted":false}}),
        json!([{"tipo":"descarta_paso","clave_evento":"chat/2"}]),
    );
    let prueba = capturar_auto(&mut s, 4, bash("cargo test", true), json!([]));
    capturar_auto(&mut s, 5, lista("completed", "pending"), json!([]));
    let episodios = s.procesar_episodios_automaticos("p", "chat", &[]).unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();

    let secuencia = s.construir_secuencia_automatica("p", id).unwrap();
    assert_eq!(secuencia["secuencia"]["estado"], "candidata_con_descartes");
    assert_eq!(
        secuencia["secuencia"]["verificacion"],
        "comprobacion_tecnica"
    );
    assert_eq!(
        secuencia["tipo_objetivo"],
        json!(["automatico_lista", "arreglar suma"])
    );
    assert_eq!(
        secuencia["resumen"]["evidencia_resultado_instancia_id"],
        prueba
    );
    assert_eq!(secuencia["secuencia"]["cantidad_descartados"], 1);
    let pasos = secuencia["pasos"].as_array().unwrap();
    let excluido = pasos
        .iter()
        .find(|p| p["instancia_id"].as_i64() == Some(lectura))
        .unwrap();
    let evidencia = pasos
        .iter()
        .find(|p| p["instancia_id"].as_i64() == Some(eco))
        .unwrap();
    let check = pasos
        .iter()
        .find(|p| p["instancia_id"].as_i64() == Some(prueba))
        .unwrap();
    assert_eq!(excluido["incluido"], false);
    assert_eq!(evidencia["incluido"], true);
    assert_eq!(check["incluido"], true);
    assert_eq!(secuencia["descartes"][0]["descartado_clave"], "chat/2");
    assert_eq!(secuencia["descartes"][0]["evidencia_clave"], "chat/3");
    assert_eq!(secuencia["seleccionable_actual"], true);

    assert_eq!(
        s.construir_secuencia_automatica("p", id).unwrap(),
        secuencia
    );
    let clave = secuencia["secuencia"]["clave"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(s.leer_secuencia("p", &clave).unwrap(), secuencia);
    assert!(s.construir_secuencia_automatica("otro", id).is_err());
}

#[test]
fn automatico_protege_la_comprobacion_que_acredita_el_resultado() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar_auto(&mut s, 1, lista("in_progress", "pending"), json!([]));
    let prueba = capturar_auto(&mut s, 2, bash("cargo test", true), json!([]));
    capturar_auto(
        &mut s,
        3,
        json!({"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
        json!([{"tipo":"descarta_paso","clave_evento":"chat/2"}]),
    );
    capturar_auto(&mut s, 4, lista("completed", "pending"), json!([]));
    let episodios = s.procesar_episodios_automaticos("p", "chat", &[]).unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();

    let secuencia = s.construir_secuencia_automatica("p", id).unwrap();
    assert_eq!(secuencia["secuencia"]["estado"], "candidata_sin_descartes");
    assert_eq!(secuencia["descartes"][0]["aplicada"], false);
    assert_eq!(
        secuencia["descartes"][0]["motivo_no_aplicada"],
        "comprobacion_del_resultado"
    );
    let check = secuencia["pasos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["instancia_id"].as_i64() == Some(prueba))
        .unwrap();
    assert_eq!(check["incluido"], true);
}

#[test]
fn automatico_fallido_conserva_el_fallo_y_no_es_candidata() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar_auto(&mut s, 1, lista("in_progress", "pending"), json!([]));
    let fallo = capturar_auto(&mut s, 2, bash("cargo test", false), json!([]));
    capturar_auto(&mut s, 3, lista("completed", "pending"), json!([]));
    let episodios = s.procesar_episodios_automaticos("p", "chat", &[]).unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();

    let secuencia = s.construir_secuencia_automatica("p", id).unwrap();
    assert_eq!(secuencia["secuencia"]["estado"], "no_candidata");
    assert_eq!(secuencia["secuencia"]["motivo"], "resultado_fallido");
    assert_eq!(secuencia["secuencia"]["resultado"], 0);
    assert_eq!(secuencia["seleccionable_actual"], false);
    assert_eq!(secuencia["resumen"]["fallos_observados"], 1);
    assert_eq!(secuencia["resumen"]["fallos_en_camino"], 1);
    let paso = secuencia["pasos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["instancia_id"].as_i64() == Some(fallo))
        .unwrap();
    assert_eq!(paso["incluido"], true);
    assert_eq!(paso["fallo_observado"], true);
}

#[test]
fn automatico_sin_verificacion_queda_sin_resultado_y_no_es_candidata() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar_auto(&mut s, 1, lista("in_progress", "pending"), json!([]));
    capturar_auto(
        &mut s,
        2,
        json!({"hook_event_name":"PostToolUse","tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
        json!([]),
    );
    capturar_auto(&mut s, 3, lista("completed", "pending"), json!([]));
    let episodios = s.procesar_episodios_automaticos("p", "chat", &[]).unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();

    let secuencia = s.construir_secuencia_automatica("p", id).unwrap();
    assert_eq!(secuencia["secuencia"]["estado"], "no_candidata");
    assert_eq!(secuencia["secuencia"]["motivo"], "sin_resultado");
    assert_eq!(secuencia["secuencia"]["resultado"], Value::Null);
    assert_eq!(secuencia["descartes"], json!([]));
}

#[test]
fn automatico_sin_sello_no_se_deriva() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar_auto(&mut s, 1, lista("in_progress", "pending"), json!([]));
    capturar_auto(&mut s, 2, bash("cargo test", true), json!([]));
    capturar_auto(&mut s, 3, lista("completed", "pending"), json!([]));
    let episodios = s.procesar_episodios_automaticos("p", "chat", &[]).unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();

    let c = Connection::open(&path).unwrap();
    c.execute_batch(
        "DROP TRIGGER episodio_automatico_sello_delete; DELETE FROM episodio_automatico_sello;",
    )
    .unwrap();
    drop(c);
    let error = s
        .construir_secuencia_automatica("p", id)
        .unwrap_err()
        .to_string();
    assert!(error.contains("sin sello"), "{error}");
}

#[test]
fn migrar_010_a_011_preserva_hashes_y_datos_y_coexiste() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,sha256 TEXT NOT NULL,aplicado_en INTEGER NOT NULL);").unwrap();
    let migraciones = [
        include_str!("../migrations/001_erd.sql"),
        include_str!("../migrations/002_captura_tecnica.sql"),
        include_str!("../migrations/003_recuperacion_textual.sql"),
        include_str!("../migrations/004_ejecucion_evaluacion.sql"),
        include_str!("../migrations/005_entrega_contexto_host.sql"),
        include_str!("../migrations/006_tareas.sql"),
        include_str!("../migrations/007_episodios.sql"),
        include_str!("../migrations/008_cortes_automaticos.sql"),
        include_str!("../migrations/009_secuencias_minimas.sql"),
        include_str!("../migrations/010_objetivos_y_vigente.sql"),
    ];
    for (i, sql) in migraciones.iter().enumerate() {
        c.execute_batch(sql).unwrap();
        c.execute(
            "INSERT INTO schema_migrations VALUES (?1,?2,0)",
            params![
                i as i64 + 1,
                format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
            ],
        )
        .unwrap();
    }
    c.execute("INSERT INTO tarea VALUES (19,'anterior','p',22,'{}')", [])
        .unwrap();
    let crudo = json!({"version":1,"clave_evento":"legado","grupo_evidencia":"g",
        "secuencia":1,"timestamp":2,"origen":"herramienta","rol":"otro",
        "contenido":"crudo anterior","metadatos":{"proyecto":"p","tipo":"herramienta-ok"}})
    .to_string();
    c.execute("INSERT INTO instancia(id,clave_evento,grupo_evidencia,origen,rol,contenido,metadatos_json,timestamp) VALUES (1,'legado','g','herramienta','otro','crudo anterior','{\"proyecto\":\"p\"}',2)",[]).unwrap();
    c.execute("INSERT INTO captura_evento VALUES (1,'g',1,?1,2)", [&crudo])
        .unwrap();
    // Dato propio de la migración 010 de main: debe sobrevivir a la 011.
    c.execute(
        "INSERT INTO tipo_objetivo VALUES (?1,'p','objetivo-terminos-v1','[\"x\"]')",
        ["a".repeat(64)],
    )
    .unwrap();

    let s = Store::open(&path).unwrap();
    assert_eq!(s.list_tasks("p").unwrap()["tareas"][0]["clave"], "anterior");
    assert_eq!(
        c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    // Las tablas de Arandu usan prefijo y conviven con las de main.
    for tabla in [
        "secuencia_minima",
        "secuencia_minima_paso",
        "secuencia_minima_sello",
        "arandu_secuencia_minima",
        "arandu_secuencia_minima_paso",
        "arandu_secuencia_minima_descarte",
        "arandu_secuencia_minima_sello",
    ] {
        assert_eq!(
            c.query_row(
                &format!(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='{tabla}'"
                ),
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            1,
            "falta la tabla {tabla}"
        );
    }
    let guardado: String = c
        .query_row(
            "SELECT json_original FROM captura_evento WHERE instancia_id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(guardado, crudo);
    assert_eq!(
        c.query_row(
            "SELECT terminos_json FROM tipo_objetivo WHERE proyecto='p'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "[\"x\"]"
    );
    for (i, sql) in migraciones.iter().enumerate() {
        assert_eq!(
            c.query_row(
                "SELECT sha256 FROM schema_migrations WHERE version=?1",
                [i as i64 + 1],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
        );
    }
    assert_eq!(
        c.query_row(
            "SELECT sha256 FROM schema_migrations WHERE version=11",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        format!(
            "{:x}",
            Sha256::digest(
                include_str!("../migrations/011_arandu_secuencias.sql")
                    .replace("\r\n", "\n")
                    .as_bytes()
            )
        )
    );
    drop(s);
    Store::open(&path).unwrap();
    c.execute(
        "UPDATE schema_migrations SET sha256='alterado' WHERE version=11",
        [],
    )
    .unwrap();
    assert!(Store::open(&path).is_err());
}

#[test]
fn cli_construye_relee_y_lista_la_secuencia() {
    let d = tempfile::tempdir().unwrap();
    let datos = d.path().join("datos");
    let mut r = msp::control::Registry::open(&datos).unwrap();
    let memoria = r.create_memory("cli").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut s = r.open_memory(&memoria).unwrap();
    tarea(&mut s);
    marcar(&mut s, "inicio");
    herramienta(
        &mut s,
        "a",
        "g",
        1,
        "herramienta-ok",
        json!([]),
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    cerrar(&mut s, "verificado", 1);
    s.construir_episodio("ep", "p", "t", "verificado").unwrap();
    drop(s);
    drop(r);

    let bin = env!("CARGO_BIN_EXE_msp");
    let corre = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .env("MSP_RAIZ", &datos)
            .output()
            .unwrap()
    };
    let salida = corre(&[
        "--control",
        "arandu-secuencia-construir",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
        "--episodio",
        "ep",
    ]);
    assert!(
        salida.status.success(),
        "{}",
        String::from_utf8_lossy(&salida.stderr)
    );
    let construida: Value = serde_json::from_slice(&salida.stdout).unwrap();
    assert_eq!(construida["secuencia"]["estado"], "candidata_sin_descartes");
    let clave = construida["secuencia"]["clave"]
        .as_str()
        .unwrap()
        .to_string();

    let repetida = corre(&[
        "--control",
        "arandu-secuencia-construir",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
        "--episodio",
        "ep",
    ]);
    assert!(repetida.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&repetida.stdout).unwrap(),
        construida
    );

    let leida = corre(&[
        "--control",
        "arandu-secuencia-leer",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
        "--secuencia",
        &clave,
    ]);
    assert!(
        leida.status.success(),
        "{}",
        String::from_utf8_lossy(&leida.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&leida.stdout).unwrap(),
        construida
    );

    let listado = corre(&[
        "--control",
        "arandu-secuencia-listar",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
    ]);
    assert!(listado.status.success());
    let listado: Value = serde_json::from_slice(&listado.stdout).unwrap();
    assert_eq!(listado["secuencias"].as_array().unwrap().len(), 1);
    assert_eq!(listado["secuencias"][0]["clave"], clave);

    let ambiguo = corre(&[
        "--control",
        "arandu-secuencia-construir",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
        "--episodio",
        "ep",
        "--automatico",
        "1",
    ]);
    assert!(!ambiguo.status.success());
    assert!(String::from_utf8_lossy(&ambiguo.stderr).contains("exactamente uno"));
}

fn pasos_arbitrarios() -> impl Strategy<Value = Vec<PasoObservado>> {
    prop::collection::vec(
        (
            prop::sample::select(vec!["g0", "g1", "g2"]),
            prop::sample::select(vec!["ok", "error", "desconocido"]),
            prop::collection::vec(
                (
                    prop::sample::select(vec![REF_DESCARTA, REF_REVIERTE]),
                    prop::sample::select(vec!["e1", "e2", "e3", "e4", "e5", "e6", "e7", "e8"]),
                ),
                0..3,
            ),
        ),
        0..8,
    )
    .prop_map(|items| {
        items
            .into_iter()
            .enumerate()
            .map(|(indice, (grupo, estado, referencias))| {
                let orden = indice as i64 + 1;
                PasoObservado {
                    orden,
                    instancia_id: orden,
                    clave_evento: format!("e{orden}"),
                    herramienta: "Read".into(),
                    tipo_destino: "archivo:rs".into(),
                    grupo_evidencia: grupo.into(),
                    timestamp: orden,
                    estado_tecnico: estado.into(),
                    referencias: referencias
                        .into_iter()
                        .map(|(tipo, clave)| ReferenciaDescarte {
                            tipo: tipo.into(),
                            clave_evento: clave.into(),
                        })
                        .collect(),
                }
            })
            .collect()
    })
}

proptest! {
    #[test]
    fn los_descartes_aplicados_respetan_evidencia_grupo_y_orden(
        pasos in pasos_arbitrarios(),
        protegido in prop::option::of(0usize..8),
    ) {
        let evidencia_resultado = protegido.and_then(|i| pasos.get(i).map(|p| p.instancia_id));
        let primero = calcular_descartes(&pasos, evidencia_resultado);
        prop_assert_eq!(&primero, &calcular_descartes(&pasos, evidencia_resultado));
        for descarte in &primero {
            let evidencia = pasos.iter().find(|p| p.instancia_id == descarte.evidencia_instancia_id).unwrap();
            let destino = pasos.iter().find(|p| p.instancia_id == descarte.instancia_id).unwrap();
            if descarte.aplicada {
                prop_assert_eq!(evidencia.estado_tecnico.as_str(), "ok");
                prop_assert_eq!(&evidencia.grupo_evidencia, &destino.grupo_evidencia);
                prop_assert!(descarte.orden_evidencia > descarte.orden_descartado);
                prop_assert_ne!(Some(destino.instancia_id), evidencia_resultado);
            } else {
                prop_assert!(descarte.motivo_no_aplicada.as_deref().is_some_and(|m| !m.is_empty()));
            }
        }
        prop_assert!(primero.iter().filter(|d| d.aplicada).count() <= primero.len());
    }

    #[test]
    fn sin_referencias_no_hay_descartes(grupos in prop::collection::vec(prop::sample::select(vec!["g0","g1"]), 0..8)) {
        let pasos: Vec<_> = grupos.into_iter().enumerate().map(|(i, grupo)| {
            let orden = i as i64 + 1;
            PasoObservado {
                orden,
                instancia_id: orden,
                clave_evento: format!("e{orden}"),
                herramienta: "Read".into(),
                tipo_destino: "archivo:rs".into(),
                grupo_evidencia: grupo.into(),
                timestamp: orden,
                estado_tecnico: "ok".into(),
                referencias: vec![],
            }
        }).collect();
        prop_assert!(calcular_descartes(&pasos, Some(1)).is_empty());
    }
}
