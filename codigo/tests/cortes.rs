//! Payloads sintéticos basados en la documentación oficial. Este worktree no
//! contiene capturas reales de TodoWrite/TaskCreate/TaskUpdate.
use msp::cortes::{detectar_tramos, es_comprobacion, EventoCorte};
use msp::episodios_automaticos::CierreChat;
use msp::Store;
use proptest::prelude::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::process::Command;

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
fn eventos(payloads: Vec<Value>) -> Vec<EventoCorte> {
    payloads
        .into_iter()
        .map(|payload| EventoCorte {
            chat: "a".into(),
            payload,
            fin_sesion: false,
        })
        .collect()
}
fn capturar(s: &mut Store, chat: &str, secuencia: i64, payload: Value) -> i64 {
    s.capture(&json!({"version":1,"clave_evento":format!("{chat}/{secuencia}"),"grupo_evidencia":chat,"secuencia":secuencia,
        "timestamp":secuencia,"origen":"herramienta","rol":"otro","contenido":"texto crudo intacto",
        "metadatos":{"proyecto":"p","intervalo_captura":7,"payload_hook":payload}}).to_string()).unwrap().instancia_id
}

#[test]
fn cortes_por_completado_siguiente_y_sesion_conservan_la_ultima_comprobacion() {
    let e = eventos(vec![
        lista("in_progress", "pending"),
        bash("cargo test --offline", true),
        bash("cargo test", false),
        lista("completed", "in_progress"),
        bash("echo texto", true),
        json!({"hook_event_name":"SessionEnd"}),
    ]);
    let t = detectar_tramos("a", &e);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].inicio, 0);
    assert_eq!(t[0].fin, Some(3));
    assert_eq!(t[0].motivo_fin.as_deref(), Some("completado"));
    assert_eq!(t[0].herramientas, vec![1, 2]);
    assert_eq!(t[0].comprobacion.as_ref().unwrap().resultado, Some(false));
    assert_eq!(t[1].objetivo, "documentar");
    assert_eq!(t[1].herramientas, vec![4]);
    assert_eq!(t[1].motivo_fin.as_deref(), Some("sesion_terminada"));
    assert!(t[1].comprobacion.is_none());
    let e = eventos(vec![
        lista("in_progress", "pending"),
        lista("pending", "in_progress"),
    ]);
    let t = detectar_tramos("a", &e);
    assert_eq!(t[0].motivo_fin.as_deref(), Some("siguiente"));
    assert_eq!(t[0].fin, Some(1));
    assert_eq!(t[1].inicio, 1);
    assert_eq!(t[1].fin, None);
}

#[test]
fn repetir_snapshot_o_llamada_no_duplica_y_fallar_al_actualizar_no_corta() {
    let mut una = bash("pytest", true);
    una["tool_use_id"] = json!("call");
    let mut fallida = lista("completed", "in_progress");
    fallida["hook_event_name"] = json!("PostToolUseFailure");
    let e = eventos(vec![
        lista("in_progress", "pending"),
        lista("in_progress", "pending"),
        una.clone(),
        fallida,
        lista("completed", "in_progress"),
        una,
        lista("completed", "completed"),
    ]);
    let t = detectar_tramos("a", &e);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].herramientas, vec![2]);
    assert_eq!(t[0].fin, Some(4));
    assert!(t[1].herramientas.is_empty());
}

#[test]
fn contratos_de_task_create_update_respetan_identidad_y_confirmacion() {
    let crear = json!({"hook_event_name":"PostToolUse","tool_name":"TaskCreate","tool_input":{"subject":"compilar","description":"suite"},
        "tool_response":{"task":{"id":"42","subject":"compilar"}}});
    let actualizar = |status: &str, success: bool| {
        json!({"hook_event_name":"PostToolUse","tool_name":"TaskUpdate", "tool_input":{"taskId":"42","status":status},
        "tool_response":{"success":success,"taskId":"42","updatedFields":["status"]}})
    };
    let e = eventos(vec![
        crear,
        actualizar("in_progress", true),
        bash("npm test", true),
        actualizar("completed", false),
        actualizar("completed", true),
    ]);
    let t = detectar_tramos("a", &e);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].objetivo, "compilar");
    assert_eq!(t[0].fuente, "TaskUpdate");
    assert_eq!(t[0].fin, Some(4));
    assert_eq!(t[0].comprobacion.as_ref().unwrap().resultado, Some(true));
    let mut renombrada = e.clone();
    renombrada[1].payload["tool_input"]["subject"] = json!("compilar todo");
    assert_eq!(
        detectar_tramos("a", &renombrada)[0].objetivo,
        "compilar todo"
    );
    let mut sin_confirmar = e.clone();
    sin_confirmar[1].payload["tool_response"]["success"] = json!(false);
    assert!(detectar_tramos("a", &sin_confirmar).is_empty());
    let desconocida = eventos(vec![actualizar("in_progress", true)]);
    assert!(detectar_tramos("a", &desconocida).is_empty());
    let mut otro_id = e.clone();
    otro_id[1].payload["tool_response"]["taskId"] = json!("otro");
    assert!(detectar_tramos("a", &otro_id).is_empty());
}

#[test]
fn no_interpreta_listas_ambiguas_ni_subagentes_como_el_chat_padre() {
    for p in [
        lista("in_progress", "in_progress"),
        lista("desconocido", "pending"),
        json!({"hook_event_name":"PostToolUse","tool_name":"TodoWrite","tool_input":{"todos":[{"status":"in_progress"}]}}),
    ] {
        assert!(detectar_tramos("a", &eventos(vec![p])).is_empty());
    }
    let mut duplicada = lista("in_progress", "pending");
    duplicada["tool_input"]["todos"][1]["content"] = json!("arreglar suma");
    assert!(detectar_tramos("a", &eventos(vec![duplicada])).is_empty());
    let mut hijo = lista("completed", "in_progress");
    hijo["agent_id"] = json!("hijo");
    let mut fin_hijo = json!({"hook_event_name":"SessionEnd","agent_id":"hijo"});
    let t = detectar_tramos(
        "a",
        &eventos(vec![
            lista("in_progress", "pending"),
            hijo,
            fin_hijo.clone(),
        ]),
    );
    assert_eq!(t.len(), 1);
    assert!(t[0].fin.is_none());
    fin_hijo.as_object_mut().unwrap().remove("agent_id");
    assert_eq!(
        detectar_tramos(
            "a",
            &eventos(vec![lista("in_progress", "pending"), fin_hijo])
        )[0]
        .fin,
        Some(1)
    );
}

#[test]
fn comprobaciones_son_comandos_reconocidos_y_no_una_busqueda_de_palabras() {
    for comando in [
        "cargo test",
        "/usr/bin/cargo test --offline",
        "C:\\bin\\CARGO.EXE test",
        "npm test",
        "pnpm test",
        "yarn test",
        "pytest",
        "pytest tests/test_suma.py",
        "python -m pytest",
        "python3 -m pytest -q",
    ] {
        assert!(es_comprobacion(&bash(comando, true)), "{comando}");
    }
    for comando in [
        "cargo build",
        "echo cargo test",
        "cargo +stable test",
        "cargo test --no-run",
        "cargo test --help",
        "pytest --collect-only",
        "pytest -h",
        "npm test || true",
        "cargo test; echo ok",
        "cargo test && git status",
        "cargo test > salida",
        "cargo test\ntrue",
        "python pytest",
        "python -m unittest",
        "\"/ruta con espacio/cargo\" test",
        "pytest $ARGS",
        "npm test --version",
        "pytest -V",
        "cargo test --list-tests",
        "cargo test -- --list",
        "pytest --co",
        "pytest --collectonly",
        "pytest --collect-only=true",
        "npm test --dry-run",
        "npm test --ignore-scripts",
    ] {
        assert!(!es_comprobacion(&bash(comando, true)), "{comando}");
    }
    assert!(!es_comprobacion(
        &json!({"tool_name":"Read","tool_input":{"command":"pytest"}})
    ));
    assert!(!es_comprobacion(&Value::Null));
}

#[test]
fn una_comprobacion_incierta_final_no_hereda_un_exito_anterior() {
    for (contenedor, campo, valor) in [
        ("tool_input", "run_in_background", json!(true)),
        ("tool_response", "interrupted", json!(true)),
        ("tool_response", "backgroundTaskId", json!("bg")),
        ("tool_response", "backgroundedByUser", json!(true)),
        ("tool_response", "timedOutAfterMs", json!(1000)),
    ] {
        let mut ultima = bash("pytest", true);
        ultima[contenedor][campo] = valor;
        let t = detectar_tramos(
            "a",
            &eventos(vec![
                lista("in_progress", "pending"),
                bash("pytest", true),
                ultima,
                lista("completed", "pending"),
            ]),
        );
        assert_eq!(t[0].comprobacion.as_ref().unwrap().evento, 2);
        assert_eq!(t[0].comprobacion.as_ref().unwrap().resultado, None);
    }
    let mut ultima = bash("pytest", false);
    ultima["is_interrupt"] = json!(true);
    let t = detectar_tramos(
        "a",
        &eventos(vec![
            lista("in_progress", "pending"),
            ultima,
            lista("completed", "pending"),
        ]),
    );
    assert_eq!(t[0].comprobacion.as_ref().unwrap().resultado, None);
    let mut sin_respuesta = bash("pytest", true);
    sin_respuesta
        .as_object_mut()
        .unwrap()
        .remove("tool_response");
    assert_eq!(
        detectar_tramos(
            "a",
            &eventos(vec![lista("in_progress", "pending"), sin_respuesta])
        )[0]
        .comprobacion
        .as_ref()
        .unwrap()
        .resultado,
        None
    );
}

fn valor_arbitrario() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<String>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<bool>().prop_map(Value::from)
    ]
    .prop_recursive(3, 24, 5, |v| {
        prop_oneof![
            prop::collection::vec(v.clone(), 0..5).prop_map(Value::from),
            prop::collection::btree_map(
                prop::sample::select(vec![
                    "tool_name",
                    "tool_input",
                    "todos",
                    "content",
                    "status",
                    "tool_response",
                    "hook_event_name",
                    "taskId",
                    "success",
                    "task",
                    "subject",
                    "id"
                ])
                .prop_map(String::from),
                v,
                0..6
            )
            .prop_map(|m| Value::Object(m.into_iter().collect()))
        ]
    })
}
fn secuencia() -> impl Strategy<Value = Vec<EventoCorte>> {
    prop::collection::vec(
        (any::<bool>(), 0u8..9, valor_arbitrario(), any::<bool>()),
        0..50,
    )
    .prop_map(|items| {
        items
            .into_iter()
            .enumerate()
            .map(|(i, (otro, tipo, valor, fin))| {
                let mut payload = match tipo {
                    0 => lista("in_progress", "pending"),
                    1 => lista("completed", "in_progress"),
                    2 => lista("pending", "in_progress"),
                    3 => lista("completed", "completed"),
                    4 => bash("pytest", true),
                    5 => bash("cargo test", false),
                    6 => json!({"hook_event_name":"SessionEnd"}),
                    _ => valor,
                };
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("tool_use_id".into(), json!(format!("id{}", i % 7)));
                }
                EventoCorte {
                    chat: if otro { "b" } else { "a" }.into(),
                    payload,
                    fin_sesion: fin,
                }
            })
            .collect()
    })
}
proptest! {
    #[test]
    fn detector_total_determinista_sin_superposiciones_ni_llamadas_compartidas(eventos in secuencia()) {
        let tramos=detectar_tramos("a",&eventos); prop_assert_eq!(&tramos,&detectar_tramos("a",&eventos));
        let propios:Vec<_>=eventos.iter().filter(|e|e.chat=="a").collect(); let mut indices=std::collections::BTreeSet::new(); let mut ids=std::collections::BTreeSet::new();
        let mut anterior=None;
        for t in &tramos {
            if let Some(fin)=anterior {prop_assert!(fin<=t.inicio);}
            if let Some(fin)=t.fin {prop_assert!(t.inicio<fin);}
            for i in &t.herramientas {prop_assert!(*i>t.inicio); prop_assert!(*i<t.fin.unwrap_or(propios.len())); prop_assert!(indices.insert(*i));
                if let Some(id)=propios[*i].payload["tool_use_id"].as_str(){prop_assert!(ids.insert(id));}}
            anterior=t.fin;
        }
    }
    #[test]
    fn reordenar_otro_chat_no_cambia_los_tramos_de_este(eventos in secuencia()) {
        let reordenados:Vec<_>=eventos.iter().filter(|e|e.chat=="a").cloned().chain(eventos.iter().rev().filter(|e|e.chat!="a").cloned()).collect();
        prop_assert_eq!(detectar_tramos("a",&eventos),detectar_tramos("a",&reordenados));
    }
}

#[test]
fn construir_sin_tarea_manual_es_idempotente_y_no_sella_el_abierto() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar(&mut s, "a", 1, lista("in_progress", "pending"));
    let prueba = capturar(&mut s, "a", 2, bash("cargo test", true));
    capturar(&mut s, "a", 3, lista("completed", "in_progress"));
    capturar(&mut s, "a", 4, bash("pytest", false));
    let primera = s.procesar_episodios_automaticos("p", "a", &[]).unwrap();
    assert_eq!(primera["episodios"].as_array().unwrap().len(), 1);
    assert_eq!(primera["abiertos"].as_array().unwrap().len(), 1);
    assert_eq!(
        primera["episodios"][0]["origen_objetivo"],
        "automatico_lista"
    );
    assert_eq!(primera["episodios"][0]["resultado"]["resultado"], 1);
    assert_eq!(
        primera["episodios"][0]["resultado"]["comprobacion_instancia_id"],
        prueba
    );
    assert_eq!(
        primera["episodios"][0]["pasos"][0]["tipo_destino"],
        "comando:cargo"
    );
    assert_eq!(
        s.procesar_episodios_automaticos("p", "a", &[]).unwrap(),
        primera
    );
    capturar(&mut s, "a", 5, lista("completed", "completed"));
    let completa = s.procesar_episodios_automaticos("p", "a", &[]).unwrap();
    assert_eq!(completa["episodios"].as_array().unwrap().len(), 2);
    assert_eq!(completa["episodios"][1]["resultado"]["resultado"], 0);
    assert_eq!(completa["abiertos"], json!([]));
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert_eq!(
        s.procesar_episodios_automaticos("p", "a", &[]).unwrap(),
        completa
    );
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM tarea", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        s.listar_episodios_automaticos("ajeno", "a").unwrap()["episodios"],
        json!([])
    );
    assert_eq!(
        s.listar_episodios_automaticos("p", "otro").unwrap()["episodios"],
        json!([])
    );
}

#[test]
fn cerrar_sin_verificar_no_crea_resultado_y_el_registro_control_cierra_el_abierto() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar(&mut s, "a", 1, lista("in_progress", "pending"));
    capturar(&mut s, "a", 2, bash("cargo build", true));
    capturar(&mut s, "a", 3, lista("completed", "in_progress"));
    let salida = s
        .procesar_episodios_automaticos(
            "p",
            "a",
            &[CierreChat {
                intervalo_id: 7,
                cerrado_en: 99,
            }],
        )
        .unwrap();
    assert_eq!(salida["episodios"].as_array().unwrap().len(), 2);
    for ep in salida["episodios"].as_array().unwrap() {
        assert_eq!(ep["resultado"], Value::Null);
        assert_eq!(ep["estado_resultado"], "sin_verificacion");
    }
    assert_eq!(salida["episodios"][1]["cierre_intervalo_id"], 7);
    assert_eq!(salida["episodios"][1]["cierre_control_en"], 99);
    assert_eq!(salida["episodios"][1]["fin_instancia_id"], Value::Null);
    assert_eq!(salida["abiertos"], json!([]));
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM episodio_automatico_resultado",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM episodio_automatico_sello", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        2
    );
}

#[test]
fn rollback_sellado_y_datos_tardios_no_reescriben_un_episodio() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar(&mut s, "a", 10, lista("in_progress", "pending"));
    capturar(&mut s, "a", 20, bash("pytest", true));
    capturar(&mut s, "a", 40, lista("completed", "pending"));
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TRIGGER fallo BEFORE INSERT ON episodio_automatico_sello BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(s.procesar_episodios_automaticos("p", "a", &[]).is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM episodio_automatico", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER fallo").unwrap();
    let guardado = s.procesar_episodios_automaticos("p", "a", &[]).unwrap();
    for tabla in [
        "episodio_automatico",
        "episodio_automatico_paso",
        "episodio_automatico_resultado",
        "episodio_automatico_sello",
    ] {
        assert!(c.execute(&format!("DELETE FROM {tabla}"), []).is_err());
    }
    assert!(c
        .execute("UPDATE episodio_automatico SET objetivo='otro'", [])
        .is_err());
    assert!(c
        .execute("UPDATE episodio_automatico_paso SET orden=2", [])
        .is_err());
    assert!(c
        .execute("UPDATE episodio_automatico_resultado SET resultado=0", [])
        .is_err());
    assert!(c
        .execute("UPDATE episodio_automatico_sello SET episodio_id=2", [])
        .is_err());
    let id = guardado["episodios"][0]["id"].as_i64().unwrap();
    assert!(c
        .execute(
            "INSERT INTO episodio_automatico_paso VALUES (?1,2,999,'Read','archivo:rs')",
            [id]
        )
        .is_err());
    capturar(&mut s, "a", 30, bash("pytest", false));
    assert!(s
        .procesar_episodios_automaticos("p", "a", &[])
        .unwrap_err()
        .to_string()
        .contains("datos tardios"));
    assert_eq!(
        s.listar_episodios_automaticos("p", "a").unwrap()["episodios"],
        guardado["episodios"]
    );
}

#[test]
fn cli_usa_cierres_reales_del_registro_sin_capturar_session_end_y_aisla_memoria() {
    let d = tempfile::tempdir().unwrap();
    let raiz = d.path().join("proyecto");
    std::fs::create_dir(&raiz).unwrap();
    let datos = d.path().join("datos");
    let mut registry = msp::control::Registry::open(&datos).unwrap();
    let proyecto = registry.register_project(&raiz).unwrap()["raiz_normalizada"]
        .as_str()
        .unwrap()
        .to_string();
    let memoria = registry.create_memory("A").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let otra = registry.create_memory("B").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    registry
        .use_memory("inst", "chat", &raiz, &memoria)
        .unwrap();
    let mut ad = msp::adapter::Adapter::new(registry, "inst".into(), 4096);
    let hook = |mut p: Value, id: &str| {
        p["session_id"] = json!("chat");
        p["cwd"] = json!(raiz);
        p["tool_use_id"] = json!(id);
        p
    };
    ad.hook(&hook(lista("in_progress", "pending"), "l1"))
        .unwrap();
    ad.hook(&hook(bash("pytest", true), "p1")).unwrap();
    let chat = "claude-code/inst/chat";
    let correr = |comando: &str| {
        let salida = Command::new(env!("CARGO_BIN_EXE_msp"))
            .args([
                "--control",
                comando,
                "--memoria",
                &memoria,
                "--proyecto",
                &proyecto,
                "--chat",
                chat,
            ])
            .env("MSP_RAIZ", &datos)
            .output()
            .unwrap();
        assert!(
            salida.status.success(),
            "{}",
            String::from_utf8_lossy(&salida.stderr)
        );
        serde_json::from_slice::<Value>(&salida.stdout).unwrap()
    };
    let abierto = correr("episodios-procesar");
    assert_eq!(abierto["episodios"], json!([]));
    assert_eq!(abierto["abiertos"].as_array().unwrap().len(), 1);
    ad.hook(
        &json!({"hook_event_name":"SessionEnd","session_id":"chat","cwd":raiz,"reason":"salida"}),
    )
    .unwrap();
    let cerrado = correr("episodios-procesar");
    assert_eq!(cerrado["episodios"].as_array().unwrap().len(), 1);
    assert_eq!(cerrado["episodios"][0]["resultado"]["resultado"], 1);
    assert!(cerrado["episodios"][0]["cierre_intervalo_id"]
        .as_i64()
        .is_some());
    assert_eq!(correr("episodios-procesar"), cerrado);
    assert_eq!(
        correr("episodios-listar")["episodios"],
        cerrado["episodios"]
    );
    assert!(ad
        .registry
        .cierres_para_episodios(&otra, &proyecto, chat)
        .unwrap()
        .is_empty());
    assert!(ad
        .registry
        .cierres_para_episodios(&memoria, "ajeno", chat)
        .unwrap()
        .is_empty());
    assert!(ad
        .registry
        .cierres_para_episodios(&memoria, &proyecto, "otro")
        .unwrap()
        .is_empty());
    let archivo = datos.join("memorias").join(format!("{memoria}.sqlite"));
    let c = Connection::open(&archivo).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM captura_evento", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    // Reactivar explícitamente conserva el chat y abre un intervalo nuevo.
    ad.registry.turn_off("inst", "chat").unwrap();
    ad.registry
        .use_memory("inst", "chat", &raiz, &memoria)
        .unwrap();
    ad.hook(&hook(lista("in_progress", "pending"), "l2"))
        .unwrap();
    ad.hook(&hook(bash("pytest", false), "p2")).unwrap();
    ad.hook(
        &json!({"hook_event_name":"SessionEnd","session_id":"chat","cwd":raiz,"reason":"salida"}),
    )
    .unwrap();
    let dos = correr("episodios-procesar");
    assert_eq!(dos["episodios"].as_array().unwrap().len(), 2);
    assert_eq!(dos["episodios"][0], cerrado["episodios"][0]);
    assert_eq!(dos["episodios"][1]["resultado"]["resultado"], 0);
    ad.registry.turn_off("inst", "chat").unwrap();
    ad.registry
        .use_memory("inst", "chat", &raiz, &memoria)
        .unwrap();
    ad.hook(&hook(lista("in_progress", "pending"), "l3"))
        .unwrap();
    ad.registry.turn_off("inst", "chat").unwrap();
    ad.registry
        .use_memory("inst", "chat", &raiz, &memoria)
        .unwrap();
    ad.hook(
        &json!({"hook_event_name":"SessionEnd","session_id":"chat","cwd":raiz,"reason":"salida"}),
    )
    .unwrap();
    let tres = correr("episodios-procesar");
    assert_eq!(tres["episodios"].as_array().unwrap().len(), 3);
    assert_eq!(tres["episodios"][2]["motivo_fin"], "sesion_terminada");
    assert_eq!(tres["episodios"][2]["resultado"], Value::Null);
    assert_eq!(
        c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}

#[test]
fn migrar_007_a_008_preserva_los_hashes_y_el_historial_manual() {
    use sha2::{Digest, Sha256};
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,sha256 TEXT NOT NULL,aplicado_en INTEGER NOT NULL)").unwrap();
    let migraciones = [
        include_str!("../migrations/001_erd.sql"),
        include_str!("../migrations/002_captura_tecnica.sql"),
        include_str!("../migrations/003_recuperacion_textual.sql"),
        include_str!("../migrations/004_ejecucion_evaluacion.sql"),
        include_str!("../migrations/005_entrega_contexto_host.sql"),
        include_str!("../migrations/006_tareas.sql"),
        include_str!("../migrations/007_episodios.sql"),
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
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.list_tasks("p").unwrap()["tareas"][0]["clave"], "anterior");
    assert_eq!(
        c.query_row("SELECT creada_en FROM tarea WHERE id=19", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        22
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
        s.listar_episodios_automaticos("p", "a").unwrap()["episodios"],
        json!([])
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    drop(s);
    Store::open(&path).unwrap();
    c.execute(
        "UPDATE schema_migrations SET sha256='alterado' WHERE version=9",
        [],
    )
    .unwrap();
    assert!(Store::open(&path).is_err());
}

#[test]
fn task_update_se_persiste_y_una_comprobacion_final_incierta_no_tiene_resultado() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    capturar(
        &mut s,
        "a",
        1,
        json!({"hook_event_name":"PostToolUse","tool_name":"TaskCreate","tool_input":{"subject":"compilar","description":"suite"},
        "tool_response":{"task":{"id":"1","subject":"compilar"}}}),
    );
    capturar(
        &mut s,
        "a",
        2,
        json!({"hook_event_name":"PostToolUse","tool_name":"TaskUpdate","tool_input":{"taskId":"1","status":"in_progress"},
        "tool_response":{"success":true,"taskId":"1","updatedFields":["status"]}}),
    );
    capturar(&mut s, "a", 3, bash("pytest", true));
    let mut incierta = bash("pytest", true);
    incierta["tool_input"]["run_in_background"] = json!(true);
    let ultima = capturar(&mut s, "a", 4, incierta);
    capturar(
        &mut s,
        "a",
        5,
        json!({"hook_event_name":"PostToolUse","tool_name":"TaskUpdate","tool_input":{"taskId":"1","status":"completed"},
        "tool_response":{"success":true,"taskId":"1","updatedFields":["status"]}}),
    );
    let ep = s.procesar_episodios_automaticos("p", "a", &[]).unwrap()["episodios"][0].clone();
    assert_eq!(ep["objetivo"], "compilar");
    assert_eq!(ep["fuente"], "TaskUpdate");
    assert_eq!(ep["resultado"], Value::Null);
    assert_eq!(ep["ultima_comprobacion_id"], ultima);
    assert_eq!(ep["pasos"].as_array().unwrap().len(), 2);
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM episodio_automatico_resultado",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM episodio_automatico_sello", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
}

#[test]
fn listar_conserva_orden_de_fuente_aunque_un_tramo_completo_llegue_despues() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    capturar(&mut s, "a", 100, lista("in_progress", "pending"));
    capturar(&mut s, "a", 200, lista("completed", "pending"));
    let anterior = s.procesar_episodios_automaticos("p", "a", &[]).unwrap()["episodios"][0].clone();
    let primero = capturar(&mut s, "a", 10, lista("pending", "in_progress"));
    capturar(&mut s, "a", 20, lista("pending", "completed"));
    let eps = s.procesar_episodios_automaticos("p", "a", &[]).unwrap()["episodios"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(eps.len(), 2);
    assert_eq!(eps[0]["inicio_instancia_id"], primero);
    assert_eq!(eps[1], anterior);
}

#[test]
fn un_cierre_sin_eventos_usa_el_primer_fin_observado_y_no_el_ultimo() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    capturar(&mut s, "a", 1, lista("in_progress", "pending"));
    let cierres = [
        CierreChat {
            intervalo_id: 9,
            cerrado_en: 200,
        },
        CierreChat {
            intervalo_id: 8,
            cerrado_en: 100,
        },
    ];
    let ep = s
        .procesar_episodios_automaticos("p", "a", &cierres)
        .unwrap()["episodios"][0]
        .clone();
    assert_eq!(ep["cierre_intervalo_id"], 8);
    assert_eq!(ep["cierre_control_en"], 100);
    assert!(s
        .procesar_episodios_automaticos(
            "p",
            "a",
            &[CierreChat {
                intervalo_id: 8,
                cerrado_en: 101
            }]
        )
        .is_err());
}
