//! Recuperación de una solución observada por tipo de objetivo.
use msp::episodios_automaticos::CierreChat;
use msp::soluciones::TipoObjetivo;
use msp::tasks::{Criterio, TaskRecord};
use msp::Store;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

fn crear_tarea(s: &mut Store, tarea: &str, criterio: &str, version: &str) {
    s.record_task(&TaskRecord::TareaCrear {
        tarea: tarea.into(),
        proyecto: "p".into(),
        definicion: format!("def/{tarea}/{version}"),
        descripcion: format!("tarea {tarea}"),
        criterios: vec![Criterio {
            id: criterio.into(),
            version: version.into(),
            descripcion: "la comprobacion pasa".into(),
            obligatorio: true,
        }],
    })
    .unwrap();
}

fn revisar_tarea(s: &mut Store, tarea: &str, criterio: &str, version: &str) {
    s.record_task(&TaskRecord::TareaRevisar {
        tarea: tarea.into(),
        definicion: format!("def/{tarea}/{version}"),
        descripcion: format!("tarea {tarea}"),
        criterios: vec![Criterio {
            id: criterio.into(),
            version: version.into(),
            descripcion: "la comprobacion pasa".into(),
            obligatorio: true,
        }],
    })
    .unwrap();
}

fn marcar(s: &mut Store, clave: &str, tarea: &str, tipo: &str) -> i64 {
    s.record_task(&TaskRecord::TareaMarcar {
        clave: clave.into(),
        tarea: tarea.into(),
        tipo: tipo.into(),
        detalle: json!({}),
    })
    .unwrap()["registrado_en"]
        .as_i64()
        .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn herramienta_sin_enlazar(
    s: &mut Store,
    clave: &str,
    grupo: &str,
    secuencia: i64,
    timestamp: i64,
    exito: bool,
    payload: Value,
) -> i64 {
    let crudo = json!({
        "version":1,
        "clave_evento":clave,
        "grupo_evidencia":grupo,
        "secuencia":secuencia,
        "timestamp":timestamp,
        "origen":"herramienta",
        "rol":"otro",
        "contenido":format!("crudo {clave}"),
        "metadatos":{
            "proyecto":"p",
            "tipo":if exito {"herramienta-ok"} else {"herramienta-error"},
            "payload_hook":payload
        },
        "referencias":[]
    });
    s.capture(&crudo.to_string()).unwrap().instancia_id
}

fn enlazar(s: &mut Store, tarea: &str, clave: &str) {
    s.record_task(&TaskRecord::TareaEnlazar {
        tarea: tarea.into(),
        evento: clave.into(),
        evidencia: "asociacion explicita".into(),
    })
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn herramienta(
    s: &mut Store,
    tarea: &str,
    clave: &str,
    grupo: &str,
    secuencia: i64,
    timestamp: i64,
    exito: bool,
    payload: Value,
) -> i64 {
    let id = herramienta_sin_enlazar(s, clave, grupo, secuencia, timestamp, exito, payload);
    enlazar(s, tarea, clave);
    id
}

fn problema(
    s: &mut Store,
    proyecto: &str,
    clave: &str,
    grupo: &str,
    secuencia: i64,
    timestamp: i64,
    contenido: &str,
) -> i64 {
    s.capture(
        &json!({
            "version":1,
            "clave_evento":clave,
            "grupo_evidencia":grupo,
            "secuencia":secuencia,
            "timestamp":timestamp,
            "origen":"usuario",
            "rol":"problema",
            "contenido":contenido,
            "metadatos":{"proyecto":proyecto}
        })
        .to_string(),
    )
    .unwrap()
    .instancia_id
}

fn evento_textual(
    s: &mut Store,
    clave: &str,
    grupo: &str,
    secuencia: i64,
    timestamp: i64,
    contenido: &str,
) -> i64 {
    s.capture(
        &json!({
            "version":1,
            "clave_evento":clave,
            "grupo_evidencia":grupo,
            "secuencia":secuencia,
            "timestamp":timestamp,
            "origen":"usuario",
            "rol":"otro",
            "contenido":contenido,
            "metadatos":{"proyecto":"p"}
        })
        .to_string(),
    )
    .unwrap()
    .instancia_id
}

#[allow(clippy::too_many_arguments)]
fn resultado(
    s: &mut Store,
    tarea: &str,
    criterio: &str,
    version: &str,
    clave: &str,
    timestamp: i64,
    valor: i64,
    anterior: Option<&str>,
) -> i64 {
    let evidencia = format!("evidencia/{clave}");
    let evaluacion = json!({
        "version":1,
        "fuente":"usuario_explicito",
        "procedencia":"real",
        "criterio":criterio,
        "criterio_version":version,
        "alcance":"tarea",
        "objetivo":null,
        "resultado":valor,
        "detalle":{"tarea":tarea},
        "nota":"fixture",
        "anterior":anterior
    });
    let id = s
        .capture(
            &json!({
                "version":1,
                "clave_evento":evidencia,
                "grupo_evidencia":evidencia,
                "secuencia":1,
                "timestamp":timestamp,
                "origen":"usuario",
                "rol":"otro",
                "contenido":"verificacion externa explicita",
                "metadatos":{
                    "proyecto":"p",
                    "procedencia":"real",
                    "evaluacion":evaluacion
                }
            })
            .to_string(),
        )
        .unwrap()
        .instancia_id;
    s.record_task(&TaskRecord::TareaResultado {
        clave: clave.into(),
        tarea: tarea.into(),
        criterio: criterio.into(),
        criterio_version: version.into(),
        evidencia,
        alcance: "tarea".into(),
        objetivo: None,
        anterior: anterior.map(str::to_string),
    })
    .unwrap();
    id
}

fn construir(s: &mut Store, episodio: &str, tarea: &str, resultado_clave: &str) -> Value {
    s.construir_episodio(episodio, "p", tarea, resultado_clave)
        .unwrap();
    s.construir_secuencia_manual("p", episodio).unwrap()
}

fn tipo_manual(tarea: &str, version: i64, criterio: &str, criterio_version: &str) -> Value {
    json!(["manual_tarea", tarea, version, criterio, criterio_version])
}

fn lista(a: &str, b: &str) -> Value {
    let todos = json!([
        {"content":"arreglar suma","status":a,"activeForm":"arreglando suma"},
        {"content":"documentar","status":b,"activeForm":"documentando"}
    ]);
    json!({
        "hook_event_name":"PostToolUse",
        "tool_name":"TodoWrite",
        "tool_input":{"todos":todos},
        "tool_response":{"oldTodos":[],"newTodos":todos}
    })
}

fn bash(comando: &str, exito: bool) -> Value {
    let mut payload = json!({
        "hook_event_name":if exito {"PostToolUse"} else {"PostToolUseFailure"},
        "tool_name":"Bash",
        "tool_input":{"command":comando},
        "tool_response":{"stdout":"salida","stderr":"","interrupted":false}
    });
    if !exito {
        payload["error"] = json!("Exit code 1\nfallo de la comprobacion");
        payload["is_interrupt"] = json!(false);
        payload.as_object_mut().unwrap().remove("tool_response");
    }
    payload
}

fn capturar_auto(
    s: &mut Store,
    clave: &str,
    grupo: &str,
    secuencia: i64,
    timestamp: i64,
    intervalo: i64,
    payload: Value,
) -> i64 {
    s.capture(
        &json!({
            "version":1,
            "clave_evento":clave,
            "grupo_evidencia":grupo,
            "secuencia":secuencia,
            "timestamp":timestamp,
            "origen":"herramienta",
            "rol":"otro",
            "contenido":format!("texto crudo intacto {clave}"),
            "metadatos":{
                "proyecto":"p",
                "intervalo_captura":intervalo,
                "payload_hook":payload
            },
            "referencias":[]
        })
        .to_string(),
    )
    .unwrap()
    .instancia_id
}

fn insertar_cierre_futuro(path: &Path, tarea: &str, registrado_en: i64) {
    let c = Connection::open(path).unwrap();
    c.execute(
        "INSERT INTO tarea_evento(clave,tarea_id,tipo,definicion_version,detalle_json,fuente,registrado_en,contrato_json)
         SELECT ?1,id,'cierre',1,'{}','usuario_explicito',?2,'{}' FROM tarea WHERE clave=?3",
        params![format!("marca/{tarea}/cierre/futuro"), registrado_en, tarea],
    )
    .unwrap();
}

/// Fixture de borde: crea una tarea, definición, cierre, vínculo y resultado
/// con relojes locales elegidos. `definicion_offset` permite adelantar sólo la
/// definición respecto de `recibido_en` sin tocar las tablas append-only.
fn manual_con_tiempos(
    path: &Path,
    problema_timestamp: i64,
    definicion_offset: i64,
) -> (Store, i64, i64) {
    let mut s = Store::open(path).unwrap();
    let paso = herramienta_sin_enlazar(
        &mut s,
        "paso",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    let evidencia = s
        .capture(
            &json!({
                "version":1,
                "clave_evento":"evidencia/resultado",
                "grupo_evidencia":"evidencia",
                "secuencia":1,
                "timestamp":2,
                "origen":"usuario",
                "rol":"otro",
                "contenido":"verificacion externa explicita",
                "metadatos":{"proyecto":"p"}
            })
            .to_string(),
        )
        .unwrap()
        .instancia_id;
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        problema_timestamp,
        "necesito una solucion",
    );
    let recibido_en: i64 = Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT recibido_en FROM captura_evento WHERE instancia_id=?1",
            [problema_id],
            |r| r.get(0),
        )
        .unwrap();
    {
        let c = Connection::open(path).unwrap();
        c.execute(
            "INSERT INTO tarea(clave,proyecto,creada_en,contrato_json) VALUES ('t','p',?1,'{}')",
            [recibido_en],
        )
        .unwrap();
        let tarea_id = c.last_insert_rowid();
        c.execute(
            "INSERT INTO tarea_evento(clave,tarea_id,tipo,definicion_version,descripcion,criterios_json,detalle_json,registrado_en,contrato_json)
             VALUES ('t/definicion/1',?1,'definicion',1,'objetivo',?2,'{}',?3,'{}')",
            params![
                tarea_id,
                r#"[{"id":"prueba","version":"1","descripcion":"la comprobacion pasa","obligatorio":true}]"#,
                recibido_en + definicion_offset
            ],
        )
        .unwrap();
        c.execute(
            "INSERT INTO tarea_evento(clave,tarea_id,tipo,definicion_version,detalle_json,fuente,registrado_en,contrato_json)
             VALUES ('t/cierre/1',?1,'cierre',1,'{}','usuario_explicito',?2,'{}')",
            params![tarea_id, recibido_en],
        )
        .unwrap();
        c.execute(
            "INSERT INTO tarea_vinculo(instancia_id,tarea_id,definicion_version,metodo,evidencia,vinculado_en,contrato_json)
             VALUES (?1,?2,1,'enlace_manual','fixture',?3,'{}')",
            params![paso, tarea_id, recibido_en],
        )
        .unwrap();
        c.execute(
            "INSERT INTO tarea_resultado(clave,tarea_id,definicion_version,criterio_id,criterio_version,fuente_instancia_id,alcance,fuente,resultado,detalle_json,registrado_en,contrato_json)
             VALUES ('res',?1,1,'prueba','1',?2,'tarea','usuario_explicito',1,'{}',?3,'{}')",
            params![tarea_id, evidencia, recibido_en],
        )
        .unwrap();
    }
    s.construir_episodio("ep", "p", "t", "res").unwrap();
    s.construir_secuencia_manual("p", "ep").unwrap();
    (s, problema_id, recibido_en)
}

fn tiempos_locales(path: &Path) -> (i64, i64, i64, i64) {
    let c = Connection::open(path).unwrap();
    let definicion = c
        .query_row(
            "SELECT registrado_en FROM tarea_evento WHERE clave='t/definicion/1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let cierre = c
        .query_row(
            "SELECT registrado_en FROM tarea_evento WHERE clave='t/cierre/1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let resultado = c
        .query_row(
            "SELECT registrado_en FROM tarea_resultado WHERE clave='res'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let vinculo = c
        .query_row("SELECT vinculado_en FROM tarea_vinculo", [], |r| r.get(0))
        .unwrap();
    (definicion, cierre, resultado, vinculo)
}

fn claves_respaldo(salida: &Value) -> Vec<&str> {
    salida["respaldo_textual"]["resultados"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resultado| resultado["clave_evento"].as_str().unwrap())
        .collect()
}

#[test]
fn tipo_objetivo_valida_forma_y_componentes() {
    let manual = TipoObjetivo::desde_json(&json!(["manual_tarea", "t", 1, "prueba", "1"])).unwrap();
    assert_eq!(manual.origen(), "manual_tarea");
    assert_eq!(manual.verificacion(), "criterio_tarea");
    assert_eq!(
        manual.valor(),
        json!(["manual_tarea", "t", 1, "prueba", "1"])
    );

    let automatico =
        TipoObjetivo::desde_json(&json!(["automatico_lista", "arreglar suma"])).unwrap();
    assert_eq!(automatico.origen(), "automatico_lista");
    assert_eq!(automatico.verificacion(), "comprobacion_tecnica");

    for invalido in [
        json!("manual_tarea"),
        json!(["manual_tarea", "t"]),
        json!(["manual_tarea", "t", 0, "prueba", "1"]),
        json!(["manual_tarea", "t", 1.5, "prueba", "1"]),
        json!(["manual_tarea", "", 1, "prueba", "1"]),
        json!(["manual_tarea", "t", 1, "", "1"]),
        json!(["manual_tarea", "t", 1, "prueba", " "]),
        json!(["automatico_lista"]),
        json!(["automatico_lista", "  "]),
        json!(["desconocido", "x"]),
    ] {
        assert!(
            TipoObjetivo::desde_json(&invalido).is_err(),
            "deberia rechazar {invalido}"
        );
    }
}

#[test]
fn ranking_elige_el_representante_mas_corto_y_es_determinista() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    for (clave, secuencia) in [("a", 1), ("b", 2), ("c", 3)] {
        herramienta(
            &mut s,
            "t",
            clave,
            "g",
            secuencia,
            secuencia,
            true,
            json!({"tool_name":"Read","tool_input":{"file_path":format!("{clave}.rs")}}),
        );
    }
    resultado(&mut s, "t", "prueba", "1", "res1", 4, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    let corta = construir(&mut s, "ep1", "t", "res1");

    for (clave, secuencia) in [("d", 5), ("e", 6)] {
        herramienta(
            &mut s,
            "t",
            clave,
            "g",
            secuencia,
            secuencia,
            true,
            json!({"tool_name":"Read","tool_input":{"file_path":format!("{clave}.rs")}}),
        );
    }
    resultado(&mut s, "t", "prueba", "1", "res2", 7, 1, None);
    marcar(&mut s, "marca/t/cierre/2", "t", "cierre");
    let larga = construir(&mut s, "ep2", "t", "res2");
    assert!(larga["secuencia"]["cantidad_incluidos"].as_i64().unwrap() > 3);

    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        100,
        "necesito una solucion",
    );
    let tipo = tipo_manual("t", 1, "prueba", "1");
    let primera = s
        .recuperar_solucion("p", problema_id, &tipo, None, 20)
        .unwrap();
    assert_eq!(primera["estado"], "candidata_encontrada");
    assert_eq!(primera["candidato_clave"], corta["secuencia"]["clave"]);
    assert_eq!(primera["seleccion"]["cantidad_incluidos"], 3);
    assert_eq!(
        primera["candidato"]["secuencia"]["clave"],
        corta["secuencia"]["clave"]
    );
    assert_eq!(
        primera["candidato"]["secuencia"]["estado"],
        "candidata_sin_descartes"
    );
    assert_eq!(
        primera["candidato"]["resultado_acredita"],
        "episodio_fuente"
    );
    assert_eq!(primera["candidato"]["es_reejecucion"], false);
    assert!(primera["respaldo_textual"].is_null());
    assert_eq!(
        primera["orden"],
        "cantidad_incluidos ASC, reciente DESC, id ASC"
    );
    assert!(primera["vigente"].as_str().unwrap().contains("punto 6"));

    let segunda = s
        .recuperar_solucion("p", problema_id, &tipo, None, 20)
        .unwrap();
    assert_eq!(primera, segunda);
}

#[test]
fn desempate_por_evidencia_mas_reciente() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    for (clave, secuencia) in [("a", 1), ("b", 2)] {
        herramienta(
            &mut s,
            "t",
            clave,
            "g",
            secuencia,
            secuencia,
            true,
            json!({"tool_name":"Read","tool_input":{"file_path":format!("{clave}.rs")}}),
        );
    }
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    resultado(&mut s, "t", "prueba", "1", "res1", 3, 1, None);
    let vieja = construir(&mut s, "ep1", "t", "res1");
    resultado(&mut s, "t", "prueba", "1", "res2", 9, 1, None);
    let reciente = construir(&mut s, "ep2", "t", "res2");
    assert_eq!(
        vieja["secuencia"]["cantidad_incluidos"],
        reciente["secuencia"]["cantidad_incluidos"]
    );

    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        100,
        "necesito una solucion",
    );
    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "candidata_encontrada");
    assert_eq!(
        salida["candidato_clave"], reciente["secuencia"]["clave"],
        "la evidencia mas reciente desempata"
    );
    assert_eq!(salida["seleccion"]["reciente"], 9);
}

#[test]
fn mismo_hash_de_pasos_no_mezcla_criterios_ni_versiones() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    crear_tarea(&mut s, "u", "prueba", "2");
    for (tarea, clave, secuencia) in [("t", "a", 1), ("u", "b", 3)] {
        marcar(&mut s, &format!("marca/{tarea}/inicio/1"), tarea, "inicio");
        herramienta(
            &mut s,
            tarea,
            clave,
            "g",
            secuencia,
            secuencia,
            true,
            json!({"tool_name":"Read","tool_input":{"file_path":"mismo.rs"}}),
        );
        resultado(
            &mut s,
            tarea,
            "prueba",
            if tarea == "t" { "1" } else { "2" },
            &format!("res-{tarea}"),
            secuencia + 1,
            1,
            None,
        );
        marcar(&mut s, &format!("marca/{tarea}/cierre/1"), tarea, "cierre");
    }
    let primera = construir(&mut s, "ep-t", "t", "res-t");
    let segunda = construir(&mut s, "ep-u", "u", "res-u");

    let c = Connection::open(&path).unwrap();
    let hash_t: String = c
        .query_row(
            "SELECT secuencia_sha256 FROM episodio WHERE clave='ep-t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let hash_u: String = c
        .query_row(
            "SELECT secuencia_sha256 FROM episodio WHERE clave='ep-u'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hash_t, hash_u, "los pasos normalizados son identicos");

    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        100,
        "necesito una solucion",
    );
    let de_t = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(de_t["candidato_clave"], primera["secuencia"]["clave"]);
    let de_u = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("u", 1, "prueba", "2"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(de_u["candidato_clave"], segunda["secuencia"]["clave"]);
    assert_ne!(de_t["candidato_clave"], de_u["candidato_clave"]);
}

#[test]
fn resultado_corregido_excluye_la_candidata() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    resultado(&mut s, "t", "prueba", "1", "res1", 2, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    construir(&mut s, "ep1", "t", "res1");
    resultado(&mut s, "t", "prueba", "1", "res2", 3, 0, Some("res1"));

    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        100,
        "necesito una solucion",
    );
    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert!(salida["candidato"].is_null());
    assert_eq!(salida["excluidos"][0]["motivo"], "resultado_corregido");
    assert_eq!(salida["respaldo_textual"]["tipo"], "fts5");
}

#[test]
fn aislamiento_por_proyecto() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    resultado(&mut s, "t", "prueba", "1", "res1", 2, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    construir(&mut s, "ep1", "t", "res1");
    let problema_id = problema(
        &mut s,
        "q",
        "problema-q",
        "problema",
        1,
        100,
        "necesito una solucion",
    );

    let salida = s
        .recuperar_solucion(
            "q",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert!(salida["candidato"].is_null());
    assert!(salida["respaldo_textual"]["resultados"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20
        )
        .is_err());
}

#[test]
fn reloj_local_del_problema_acepta_cierres_y_resultados_previos() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    resultado(&mut s, "t", "prueba", "1", "res1", 2, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    let secuencia = construir(&mut s, "ep1", "t", "res1");
    sleep(Duration::from_millis(2));
    // El reloj de fuente del problema queda muy atrás; el local es posterior.
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        5,
        "necesito una solucion",
    );

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["problema"]["timestamp"], 5);
    let recibido = salida["problema"]["recibido_en"].as_i64().unwrap();
    assert!(recibido > 5);
    assert_eq!(salida["estado"], "candidata_encontrada");
    assert_eq!(salida["candidato_clave"], secuencia["secuencia"]["clave"]);

    let c = Connection::open(&path).unwrap();
    let resultado_registrado: i64 = c
        .query_row(
            "SELECT registrado_en FROM tarea_resultado WHERE clave='res1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let cierre_registrado: i64 = c
        .query_row(
            "SELECT registrado_en FROM tarea_evento WHERE clave='marca/t/cierre/1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(resultado_registrado > 5);
    assert!(cierre_registrado > 5);
}

#[test]
fn resultado_registrado_despues_del_problema_no_esta_disponible() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    sleep(Duration::from_millis(2));
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        5,
        "necesito una solucion",
    );
    sleep(Duration::from_millis(2));
    resultado(&mut s, "t", "prueba", "1", "res1", 2, 1, None);
    construir(&mut s, "ep1", "t", "res1");

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert_eq!(salida["excluidos"][0]["motivo"], "resultado_no_disponible");
}

#[test]
fn evidencia_futura_por_tiempo_excluye_aunque_los_pasos_sean_antiguos() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    // La evidencia tiene id anterior al problema pero tiempo de fuente futuro.
    resultado(&mut s, "t", "prueba", "1", "res1", 9_999, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    construir(&mut s, "ep1", "t", "res1");
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        5,
        "necesito una solucion",
    );

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert_eq!(
        salida["excluidos"][0]["motivo"],
        "evidencia_fuera_del_corte_por_tiempo"
    );
}

#[test]
fn cierre_manual_futuro_excluye_la_candidata() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    resultado(&mut s, "t", "prueba", "1", "res1", 2, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    let futuro = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64
        + 10_000_000;
    insertar_cierre_futuro(&path, "t", futuro);
    construir(&mut s, "ep1", "t", "res1");
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        5,
        "necesito una solucion",
    );

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert_eq!(salida["excluidos"][0]["motivo"], "cierre_no_disponible");
}

#[test]
fn cierre_control_futuro_excluye_el_automatico() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar_auto(
        &mut s,
        "chat/1",
        "chat",
        1,
        1,
        7,
        lista("in_progress", "pending"),
    );
    capturar_auto(&mut s, "chat/2", "chat", 2, 2, 7, bash("cargo test", true));
    let futuro = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64
        + 10_000_000;
    let episodios = s
        .procesar_episodios_automaticos(
            "p",
            "chat",
            &[CierreChat {
                intervalo_id: 7,
                cerrado_en: futuro,
            }],
        )
        .unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();
    let secuencia = s.construir_secuencia_automatica("p", id).unwrap();
    assert_eq!(
        secuencia["tipo_objetivo"],
        json!(["automatico_lista", "arreglar suma"])
    );
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        5,
        "necesito una solucion",
    );

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &json!(["automatico_lista", "arreglar suma"]),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert_eq!(
        salida["excluidos"][0]["motivo"],
        "cierre_control_no_disponible"
    );
}

#[test]
fn automatico_con_fin_posterior_al_problema_se_excluye() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar_auto(
        &mut s,
        "chat/1",
        "chat",
        1,
        1,
        7,
        lista("in_progress", "pending"),
    );
    capturar_auto(&mut s, "chat/2", "chat", 2, 2, 7, bash("cargo test", true));
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        5,
        "necesito una solucion",
    );
    // Pasos y verificación anteriores; el fin del tramo se captura después.
    capturar_auto(
        &mut s,
        "chat/3",
        "chat",
        3,
        10,
        7,
        lista("completed", "pending"),
    );
    let episodios = s.procesar_episodios_automaticos("p", "chat", &[]).unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();
    s.construir_secuencia_automatica("p", id).unwrap();

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &json!(["automatico_lista", "arreglar suma"]),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert_eq!(
        salida["excluidos"][0]["motivo"],
        "fin_fuera_del_corte_por_id"
    );
    assert!(salida["tipo_alcance"]
        .as_str()
        .unwrap()
        .contains("comprobacion tecnica"));
}

#[test]
fn automatico_con_timestamps_iguales_al_problema_es_elegible() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar_auto(
        &mut s,
        "chat/1",
        "chat",
        1,
        5,
        7,
        lista("in_progress", "pending"),
    );
    capturar_auto(&mut s, "chat/2", "chat", 2, 5, 7, bash("cargo test", true));
    capturar_auto(
        &mut s,
        "chat/3",
        "chat",
        3,
        5,
        7,
        lista("completed", "pending"),
    );
    let episodios = s.procesar_episodios_automaticos("p", "chat", &[]).unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();
    let secuencia = s.construir_secuencia_automatica("p", id).unwrap();
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        5,
        "necesito una solucion",
    );

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &json!(["automatico_lista", "arreglar suma"]),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "candidata_encontrada");
    assert_eq!(
        salida["candidato_clave"], secuencia["secuencia"]["clave"],
        "todos los eventos usan timestamp fuente igual al del problema"
    );
    assert_eq!(salida["problema"]["timestamp"], 5);
    assert!(salida["tipo_alcance"]
        .as_str()
        .unwrap()
        .contains("comprobacion tecnica"));
}

#[test]
fn grupo_del_problema_excluye_recorridos_manuales_y_automaticos() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "mismo",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    resultado(&mut s, "t", "prueba", "1", "res1", 2, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    construir(&mut s, "ep1", "t", "res1");
    let manual_id = problema(
        &mut s,
        "p",
        "problema-manual",
        "mismo",
        2,
        100,
        "necesito una solucion",
    );
    let manual = s
        .recuperar_solucion(
            "p",
            manual_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(manual["estado"], "sin_candidato");
    assert_eq!(
        manual["excluidos"][0]["motivo"],
        "paso_del_grupo_del_problema"
    );

    capturar_auto(
        &mut s,
        "chat/1",
        "chat-auto",
        1,
        1,
        7,
        lista("in_progress", "pending"),
    );
    capturar_auto(
        &mut s,
        "chat/2",
        "chat-auto",
        2,
        2,
        7,
        bash("cargo test", true),
    );
    capturar_auto(
        &mut s,
        "chat/3",
        "chat-auto",
        3,
        3,
        7,
        lista("completed", "pending"),
    );
    let episodios = s
        .procesar_episodios_automaticos("p", "chat-auto", &[])
        .unwrap();
    let id = episodios["episodios"][0]["id"].as_i64().unwrap();
    s.construir_secuencia_automatica("p", id).unwrap();
    let automatico_id = problema(
        &mut s,
        "p",
        "problema-auto",
        "chat-auto",
        4,
        100,
        "necesito una solucion",
    );
    let automatico = s
        .recuperar_solucion(
            "p",
            automatico_id,
            &json!(["automatico_lista", "arreglar suma"]),
            None,
            20,
        )
        .unwrap();
    assert_eq!(automatico["estado"], "sin_candidato");
    assert_eq!(
        automatico["excluidos"][0]["motivo"],
        "chat_del_grupo_del_problema"
    );
}

#[test]
fn vinculo_posterior_al_problema_no_esta_disponible() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    // El paso existe desde antes, pero su pertenencia a la tarea se registra
    // después de recibir el problema.
    herramienta_sin_enlazar(
        &mut s,
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    resultado(&mut s, "t", "prueba", "1", "res1", 2, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        5,
        "necesito una solucion",
    );
    sleep(Duration::from_millis(2));
    enlazar(&mut s, "t", "a");
    construir(&mut s, "ep1", "t", "res1");

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert_eq!(salida["excluidos"][0]["motivo"], "vinculo_no_disponible");
}

#[test]
fn vinculo_de_otra_definicion_no_se_acepta() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    revisar_tarea(&mut s, "t", "prueba", "2");
    marcar(&mut s, "marca/t/inicio/2", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "b",
        "g",
        2,
        2,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"b.rs"}}),
    );
    resultado(&mut s, "t", "prueba", "2", "res2", 3, 1, None);
    marcar(&mut s, "marca/t/cierre/2", "t", "cierre");
    let secuencia = construir(&mut s, "ep2", "t", "res2");
    assert_eq!(
        secuencia["tipo_objetivo"],
        json!(["manual_tarea", "t", 2, "prueba", "2"])
    );
    assert_eq!(secuencia["secuencia"]["cantidad_pasos"], 2);

    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        100,
        "necesito una solucion",
    );
    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 2, "prueba", "2"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert_eq!(
        salida["excluidos"][0]["motivo"],
        "vinculo_de_otra_definicion"
    );
}

#[test]
fn manual_con_tiempos_locales_exactos_al_recibido_es_elegible() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let (mut s, problema_id, recibido_en) = manual_con_tiempos(&path, 100, 0);

    assert_eq!(
        tiempos_locales(&path),
        (recibido_en, recibido_en, recibido_en, recibido_en)
    );
    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "candidata_encontrada");
    assert_eq!(salida["problema"]["recibido_en"], recibido_en);
    assert!(salida["tipo_alcance"]
        .as_str()
        .unwrap()
        .contains("misma tarea y version de definicion"));
}

#[test]
fn manual_con_definicion_futura_se_excluye_por_disponibilidad() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let (mut s, problema_id, recibido_en) = manual_con_tiempos(&path, 100, 1);

    let (definicion, cierre, resultado, vinculo) = tiempos_locales(&path);
    assert_eq!(definicion, recibido_en + 1);
    assert_eq!(
        (cierre, resultado, vinculo),
        (recibido_en, recibido_en, recibido_en)
    );

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("t", 1, "prueba", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert_eq!(salida["excluidos"][0]["motivo"], "definicion_no_disponible");
    assert!(salida["tipo_alcance"]
        .as_str()
        .unwrap()
        .contains("misma tarea y version de definicion"));
}

#[test]
fn fallback_fts_excluye_grupo_y_no_escribe() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    s.capture(
        &json!({
            "version":1,
            "clave_evento":"otro/suma",
            "grupo_evidencia":"otro",
            "secuencia":1,
            "timestamp":50,
            "origen":"usuario",
            "rol":"otro",
            "contenido":"la suma se resuelve con un algoritmo",
            "metadatos":{"proyecto":"p"}
        })
        .to_string(),
    )
    .unwrap();
    s.capture(
        &json!({
            "version":1,
            "clave_evento":"problema/suma",
            "grupo_evidencia":"problema",
            "secuencia":2,
            "timestamp":60,
            "origen":"usuario",
            "rol":"otro",
            "contenido":"suma dentro del mismo grupo",
            "metadatos":{"proyecto":"p"}
        })
        .to_string(),
    )
    .unwrap();
    let problema_id = problema(
        &mut s,
        "p",
        "consulta",
        "problema",
        1,
        100,
        "necesito resolver una suma",
    );
    let c = Connection::open(&path).unwrap();
    let antes: i64 = c
        .query_row("SELECT count(*) FROM evento_recuperacion", [], |r| r.get(0))
        .unwrap();
    drop(c);

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("inexistente", 1, "x", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    assert!(salida["candidato"].is_null());
    assert_eq!(salida["respaldo_textual"]["tipo"], "fts5");
    let resultados = salida["respaldo_textual"]["resultados"].as_array().unwrap();
    assert_eq!(resultados.len(), 1, "solo el evento de otro grupo");
    assert_eq!(resultados[0]["grupo_evidencia"], "otro");
    assert_eq!(
        salida["respaldo_textual"]["corte"]["grupo_excluido"],
        "problema"
    );

    let c = Connection::open(&path).unwrap();
    let despues: i64 = c
        .query_row("SELECT count(*) FROM evento_recuperacion", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        antes, despues,
        "la consulta no crea eventos de recuperacion"
    );
}

#[test]
fn fallback_derivado_respeta_limite_de_bytes() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    s.capture(
        &json!({
            "version":1,
            "clave_evento":"otro/objetivo",
            "grupo_evidencia":"otro",
            "secuencia":1,
            "timestamp":50,
            "origen":"usuario",
            "rol":"otro",
            "contenido":"objetivo documentado",
            "metadatos":{"proyecto":"p"}
        })
        .to_string(),
    )
    .unwrap();
    let largo = format!("{} objetivo", "á".repeat(20_000));
    let problema_id = problema(&mut s, "p", "consulta", "problema", 1, 100, &largo);

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("inexistente", 1, "x", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    let consulta = salida["respaldo_textual"]["consulta"].as_str().unwrap();
    assert!(consulta.len() <= 16_384);
    assert!(consulta.is_char_boundary(consulta.len()));
    let resultados = salida["respaldo_textual"]["resultados"].as_array().unwrap();
    assert_eq!(resultados.len(), 1);
    assert_eq!(resultados[0]["grupo_evidencia"], "otro");
}

#[test]
fn fallback_usa_consulta_explicita_o_deriva_del_problema_si_esta_en_blanco() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    evento_textual(
        &mut s,
        "evento/contenido",
        "grupo-a",
        1,
        50,
        "alfa problema",
    );
    evento_textual(
        &mut s,
        "evento/consulta",
        "grupo-b",
        1,
        60,
        "consulta explicita",
    );
    let problema_id = problema(&mut s, "p", "problema", "problema", 1, 100, "alfa problema");
    let tipo = tipo_manual("inexistente", 1, "x", "1");

    let derivada = s
        .recuperar_solucion("p", problema_id, &tipo, Some("   "), 20)
        .unwrap();
    assert_eq!(
        derivada["respaldo_textual"]["consulta"], "alfa problema",
        "una consulta en blanco deriva del contenido"
    );
    let claves = claves_respaldo(&derivada);
    assert_eq!(claves, vec!["evento/contenido"]);

    let explicita = s
        .recuperar_solucion("p", problema_id, &tipo, Some("consulta explicita"), 20)
        .unwrap();
    assert_eq!(
        explicita["respaldo_textual"]["consulta"], "consulta explicita",
        "la consulta explicita prevalece sobre el contenido"
    );
    let claves = claves_respaldo(&explicita);
    assert_eq!(claves, vec!["evento/consulta"]);
}

#[test]
fn fallback_derivado_con_mas_de_64_terminos_no_falla() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let contenido = (0..70)
        .map(|i| format!("t{i:03}"))
        .collect::<Vec<_>>()
        .join(" ");
    let problema_id = problema(&mut s, "p", "problema", "problema", 1, 100, &contenido);

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("inexistente", 1, "x", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    let consulta = salida["respaldo_textual"]["consulta"].as_str().unwrap();
    let esperada = (0..64)
        .map(|i| format!("t{i:03}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(consulta, esperada);
    assert!(!consulta.split(' ').any(|termino| termino == "t064"));
}

#[test]
fn fallback_derivado_conserva_terminos_que_la_minuscula_repartiria() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let contenido = (0..64)
        .map(|i| format!("aİb{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let problema_id = problema(&mut s, "p", "problema", "problema", 1, 100, &contenido);

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("inexistente", 1, "x", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    let consulta = salida["respaldo_textual"]["consulta"].as_str().unwrap();
    let terminos: Vec<&str> = consulta.split(' ').collect();
    assert_eq!(terminos.len(), 64);
    assert!(terminos.contains(&"aİb0"));
    assert!(terminos.contains(&"aİb63"));
}

#[test]
fn fallback_derivado_llena_16384_bytes_y_omite_el_siguiente_termino() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let terminos: Vec<String> = (0..29).map(|i| format!("a{:0>563}", i)).collect();
    let esperada = terminos.join(" ");
    assert_eq!(esperada.len(), 16_384);
    let contenido = format!("{esperada} zzz");
    let problema_id = problema(&mut s, "p", "problema", "problema", 1, 100, &contenido);

    let salida = s
        .recuperar_solucion(
            "p",
            problema_id,
            &tipo_manual("inexistente", 1, "x", "1"),
            None,
            20,
        )
        .unwrap();
    assert_eq!(salida["estado"], "sin_candidato");
    let consulta = salida["respaldo_textual"]["consulta"].as_str().unwrap();
    assert_eq!(consulta.len(), 16_384);
    assert_eq!(consulta, esperada);
    assert!(!consulta.split(' ').any(|termino| termino == "zzz"));
}

#[test]
fn cli_recupera_candidato_y_fallback() {
    let d = tempfile::tempdir().unwrap();
    let datos = d.path().join("datos");
    let mut r = msp::control::Registry::open(&datos).unwrap();
    let memoria = r.create_memory("cli-solucion").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut s = r.open_memory(&memoria).unwrap();
    crear_tarea(&mut s, "t", "prueba", "1");
    marcar(&mut s, "marca/t/inicio/1", "t", "inicio");
    herramienta(
        &mut s,
        "t",
        "a",
        "g",
        1,
        1,
        true,
        json!({"tool_name":"Read","tool_input":{"file_path":"a.rs"}}),
    );
    resultado(&mut s, "t", "prueba", "1", "res1", 2, 1, None);
    marcar(&mut s, "marca/t/cierre/1", "t", "cierre");
    let secuencia = construir(&mut s, "ep1", "t", "res1");
    let problema_id = problema(
        &mut s,
        "p",
        "problema",
        "problema",
        1,
        100,
        "necesito una solucion",
    );
    s.capture(
        &json!({
            "version":1,
            "clave_evento":"otro/suma",
            "grupo_evidencia":"otro",
            "secuencia":1,
            "timestamp":50,
            "origen":"usuario",
            "rol":"otro",
            "contenido":"suma documentada",
            "metadatos":{"proyecto":"p"}
        })
        .to_string(),
    )
    .unwrap();
    let fallback_id = problema(
        &mut s,
        "p",
        "consulta-fallback",
        "problema",
        2,
        100,
        "necesito una suma",
    );
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
    let tipo = json!(["manual_tarea", "t", 1, "prueba", "1"]).to_string();
    let problema_id = problema_id.to_string();
    let salida = corre(&[
        "--control",
        "arandu-solucion-recuperar",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
        "--problema",
        &problema_id,
        "--tipo-objetivo",
        &tipo,
    ]);
    assert!(
        salida.status.success(),
        "{}",
        String::from_utf8_lossy(&salida.stderr)
    );
    let recuperada: Value = serde_json::from_slice(&salida.stdout).unwrap();
    assert_eq!(recuperada["estado"], "candidata_encontrada");
    assert_eq!(
        recuperada["candidato_clave"],
        secuencia["secuencia"]["clave"]
    );

    let repetida = corre(&[
        "--control",
        "arandu-solucion-recuperar",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
        "--problema",
        &problema_id,
        "--tipo-objetivo",
        &tipo,
    ]);
    assert!(repetida.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&repetida.stdout).unwrap(),
        recuperada
    );

    let invalido = corre(&[
        "--control",
        "arandu-solucion-recuperar",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
        "--problema",
        &problema_id,
        "--tipo-objetivo",
        "[\"manual_tarea\"]",
    ]);
    assert!(!invalido.status.success());
    assert!(String::from_utf8_lossy(&invalido.stderr).contains("manual_tarea"));

    let fallback = corre(&[
        "--control",
        "arandu-solucion-recuperar",
        "--memoria",
        &memoria,
        "--proyecto",
        "p",
        "--problema",
        &fallback_id.to_string(),
        "--tipo-objetivo",
        "[\"manual_tarea\",\"inexistente\",1,\"x\",\"1\"]",
        "--consulta",
        "suma",
    ]);
    assert!(
        fallback.status.success(),
        "{}",
        String::from_utf8_lossy(&fallback.stderr)
    );
    let respaldo: Value = serde_json::from_slice(&fallback.stdout).unwrap();
    assert_eq!(respaldo["estado"], "sin_candidato");
    assert_eq!(
        respaldo["respaldo_textual"]["resultados"][0]["grupo_evidencia"],
        "otro"
    );
}
