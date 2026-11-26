use msp::{
    execution::Record,
    retrieval::{PrepareRequest, SearchRequest},
    Store,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Command};
fn ev(key: &str, seq: i64) -> Value {
    json!({"version":1,"clave_evento":key,"grupo_evidencia":"B","secuencia":seq,"timestamp":seq,"origen":"herramienta","rol":"otro","contenido":"5","metadatos":{"proyecto":"p","procedencia":"simulada"},"referencias":[]})
}
fn record(s: &mut Store, v: Value) -> anyhow::Result<Value> {
    s.record_execution(&serde_json::from_value::<Record>(v)?)
}
fn attempt(s: &mut Store, key: &str) -> Value {
    record(
        s,
        json!({"operacion":"intento","intento":key,"proyecto":"p","problema":"problem"}),
    )
    .unwrap()
}
fn link(s: &mut Store, key: &str) -> anyhow::Result<Value> {
    record(s, json!({"operacion":"enlazar","intento":"a","evento":key}))
}
fn capture(s: &mut Store, v: &Value) {
    s.capture(&v.to_string()).unwrap();
}
fn setup(path: &Path) -> Store {
    let mut s = Store::open(path).unwrap();
    let mut a = ev("old", 1);
    a["grupo_evidencia"] = json!("A");
    a["contenido"] = json!("suma 2 y 3 = 5");
    capture(&mut s, &a);
    let mut p = ev("problem", 10);
    p["rol"] = json!("problema");
    p["origen"] = json!("usuario");
    let id = s.capture(&p.to_string()).unwrap().instancia_id;
    for key in ["packet", "unused"] {
        s.prepare_memory(
            key,
            &PrepareRequest {
                problema_id: id,
                busqueda: SearchRequest {
                    proyecto: "p".into(),
                    consulta: "suma".into(),
                    origen: None,
                    rol: None,
                    limite: 20,
                },
                presupuesto_bytes: 4000,
            },
        )
        .unwrap();
    }
    attempt(&mut s, "a");
    s
}
fn request(s: &Store, key: &str, seq: i64) -> Value {
    let text = s.memory_packet("p", "packet").unwrap()["texto_memoria"].clone();
    let mut e = ev(key, seq);
    e["metadatos"]["solicitud"] = json!({"version":1,"llamada_id":key,"procedencia":"simulada","frontera":"envio_observado","evidencia_envio":"adaptador local observó entrega al sumidero simulado","mensajes":[{"rol":"user","contenido":"sumar"},{"rol":"system","contenido":text}],"bloques":[{"paquete":"packet","posicion":1,"texto":text}]});
    e
}
fn expose(key: &str, req: &str) -> Value {
    json!({"operacion":"exponer","clave":key,"intento":"a","solicitud":req,"paquete":"packet","posicion":1})
}
fn evaluation(
    key: &str,
    seq: i64,
    result: Value,
    target: Option<&str>,
    previous: Option<&str>,
) -> Value {
    let mut e = ev(key, seq);
    e["origen"] = json!("usuario");
    e["referencias"] = json!([{"tipo":"evalua","clave_evento":target.unwrap_or("problem")}]);
    e["metadatos"]["evaluacion"] = json!({"version":1,"fuente":"usuario_explicito","procedencia":"simulada","criterio":"suma","criterio_version":"1","alcance":if target.is_some(){"evento"}else{"intento"},"objetivo":target,"resultado":result,"detalle":{"descripcion":"igual a 5"},"nota":"evaluacion explícita de fixture","anterior":previous});
    e
}
fn evaluate(key: &str) -> Value {
    json!({"operacion":"evaluar","clave":key,"intento":"a","evidencia":key})
}
fn count(path: &Path, table: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
#[test]
fn writers_reopen_idempotency_conflicts_and_no_causal_credit() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    let q = request(&s, "q", 20);
    capture(&mut s, &q);
    let l = link(&mut s, "q").unwrap();
    let x = record(&mut s, expose("x", "q")).unwrap();
    for (recibo, entidad) in [(&l, "ejecucion_evento"), (&x, "exposicion_memoria")] {
        assert_eq!(recibo["entidad"], entidad);
        assert!(recibo["id"].as_i64().unwrap() > 0);
        assert!(recibo["registrado_en"].as_i64().unwrap() > 0);
    }
    let e = evaluation("v", 30, json!(1), Some("q"), None);
    capture(&mut s, &e);
    let v = record(&mut s, evaluate("v")).unwrap();
    let before = s.execution("p", "a").unwrap();
    drop(s);
    let mut s = Store::open(&p).unwrap();
    assert_eq!(link(&mut s, "q").unwrap(), l);
    assert_eq!(record(&mut s, expose("x", "q")).unwrap(), x);
    assert_eq!(record(&mut s, evaluate("v")).unwrap(), v);
    attempt(&mut s, "a");
    assert_eq!(before, s.execution("p", "a").unwrap());
    let mut bad = expose("x", "q");
    bad["posicion"] = json!(0);
    assert!(record(&mut s, bad).is_err());
    let mut bad = evaluate("v");
    bad["evidencia"] = json!("q");
    assert!(record(&mut s, bad).is_err());
    assert!(record(
        &mut s,
        json!({"operacion":"intento","intento":"a","proyecto":"other","problema":"problem"})
    )
    .is_err());
    attempt(&mut s, "b");
    assert!(record(
        &mut s,
        json!({"operacion":"enlazar","intento":"b","evento":"q"})
    )
    .is_err());
    let q2 = request(&s, "q2", 40);
    capture(&mut s, &q2);
    link(&mut s, "q2").unwrap();
    record(&mut s, expose("x2", "q2")).unwrap();
    let view = s.execution("p", "a").unwrap();
    assert_eq!(view["exposiciones"].as_array().unwrap().len(), 2);
    assert_eq!(
        s.exposures("p", Some("q"), Some("packet")).unwrap()["exposiciones"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        s.exposures("other", None, None).unwrap()["exposiciones"],
        json!([])
    );
    assert_eq!(
        s.exposures("p", Some("' OR 1=1 --"), None).unwrap()["exposiciones"],
        json!([])
    );
    assert_eq!(view["atribucion_causal"], "no_evaluada");
    assert_eq!(count(&p, "evento_feedback"), 0);
    assert_eq!(
        s.unexposed_packets("p").unwrap()["paquetes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn solicitud_con_procedencia_desconocida_no_se_enlaza() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    let mut q = request(&s, "q", 20);
    q["metadatos"]["procedencia"] = json!("inventada");
    q["metadatos"]["solicitud"]["procedencia"] = json!("inventada");
    capture(&mut s, &q);
    let error = link(&mut s, "q").unwrap_err().to_string();
    assert!(
        error.contains("procedencia debe ser real o simulada"),
        "{error}"
    );
    assert_eq!(count(&p, "ejecucion_evento"), 0);
}

#[test]
fn exponer_rechaza_bloques_ambiguos_por_paquete_o_por_posicion() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    for (seq, paquete, posicion) in [(20, "packet", 0), (21, "otro", 1)] {
        let clave = format!("q{seq}");
        let mut q = request(&s, &clave, seq);
        let mut adicional = q["metadatos"]["solicitud"]["bloques"][0].clone();
        adicional["paquete"] = json!(paquete);
        adicional["posicion"] = json!(posicion);
        q["metadatos"]["solicitud"]["bloques"]
            .as_array_mut()
            .unwrap()
            .push(adicional);
        capture(&mut s, &q);
        link(&mut s, &clave).unwrap();
        let error = record(&mut s, expose(&format!("x{seq}"), &clave))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("bloque/posicion/texto incompatible"),
            "{error}"
        );
    }
    assert_eq!(count(&p, "exposicion_memoria"), 0);
}

#[test]
fn evaluar_admite_uso_nulo_y_compatible_y_rechaza_otro_intento_sin_escribir() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    for (seq, intento) in [(20, None), (21, Some("a")), (22, Some("ajeno"))] {
        let clave = format!("v{seq}");
        let mut v = evaluation(&clave, seq, json!(1), None, None);
        v["uso"] = intento
            .map(|i| json!({"intento_clave":i,"tipo":"prueba"}))
            .unwrap_or(Value::Null);
        capture(&mut s, &v);
        let respuesta = record(&mut s, evaluate(&clave));
        if intento == Some("ajeno") {
            assert!(respuesta
                .unwrap_err()
                .to_string()
                .contains("intento de evidencia incompatible"));
        } else {
            assert_eq!(respuesta.unwrap()["entidad"], "evaluacion_ejecucion");
        }
    }
    assert_eq!(count(&p, "evaluacion_ejecucion"), 2);
    assert_eq!(count(&p, "ejecucion_evento"), 2);
}
#[test]
fn links_reject_incompatible_data_and_pending_response_can_be_retried() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    let mut response = ev("response", 30);
    response["origen"] = json!("modelo");
    response["metadatos"]["respuesta"] =
        json!({"version":1,"llamada_id":"q","procedencia":"simulada","solicitud":"q"});
    response["referencias"] = json!([{"tipo":"responde_a","clave_evento":"q"}]);
    capture(&mut s, &response);
    assert!(link(&mut s, "response").is_err());
    let view = s.execution("p", "a").unwrap();
    let pending = view["eventos_del_episodio_sin_asignar"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["clave_evento"] == "response")
        .unwrap();
    assert_eq!(pending["referencias"][0]["disponible"], false);
    let q = request(&s, "q", 20);
    capture(&mut s, &q);
    assert!(link(&mut s, "response").is_err());
    link(&mut s, "q").unwrap();
    link(&mut s, "response").unwrap();
    link(&mut s, "response").unwrap();
    let view = s.execution("p", "a").unwrap();
    assert_eq!(view["eventos"][0]["secuencia"], 20);
    assert_eq!(
        view["llamadas"][0]["estado_respuesta"],
        "respuesta_enlazada"
    );
    assert_eq!(view["estado_evaluacion"], "desconocido");
    for (i, field, value) in [
        (40, "project", json!("other")),
        (41, "group", json!("C")),
        (42, "project", Value::Null),
        (43, "usage", json!("other")),
        (44, "provenance", json!("real")),
    ] {
        let mut e = if field == "provenance" {
            response.clone()
        } else {
            ev(&format!("bad{i}"), i)
        };
        e["clave_evento"] = json!(format!("bad{i}"));
        e["secuencia"] = json!(i);
        match field {
            "project" => e["metadatos"]["proyecto"] = value,
            "group" => e["grupo_evidencia"] = value,
            "usage" => e["uso"] = json!({"intento_clave":value,"tipo":"inicio"}),
            _ => {
                e["metadatos"]["respuesta"]["procedencia"] = value;
            }
        }
        capture(&mut s, &e);
        assert!(link(&mut s, &format!("bad{i}")).is_err());
    }
}
#[test]
fn exposure_checks_exact_snapshot_boundary_problem_and_call_identity() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    for (i, kind) in [
        "text",
        "position",
        "prepared",
        "uncertain",
        "evidence",
        "provenance",
        "empty",
    ]
    .iter()
    .enumerate()
    {
        let key = format!("q{i}");
        let mut q = request(&s, &key, 20 + i as i64);
        match *kind {
            "text" => q["metadatos"]["solicitud"]["mensajes"][1]["contenido"] = json!("recortado"),
            "position" => q["metadatos"]["solicitud"]["bloques"][0]["posicion"] = json!(0),
            "prepared" => q["metadatos"]["solicitud"]["frontera"] = json!("preparada"),
            "uncertain" => q["metadatos"]["solicitud"]["frontera"] = json!("incierta"),
            "evidence" => q["metadatos"]["solicitud"]["evidencia_envio"] = Value::Null,
            "provenance" => q["metadatos"]["solicitud"]["procedencia"] = json!("real"),
            _ => q["metadatos"]["solicitud"]["bloques"][0]["texto"] = json!(""),
        };
        capture(&mut s, &q);
        if *kind == "provenance" {
            assert!(link(&mut s, &key).is_err());
        } else {
            link(&mut s, &key).unwrap();
            assert!(record(&mut s, expose(&format!("x{i}"), &key)).is_err());
        }
    }
    let q = request(&s, "valid", 40);
    capture(&mut s, &q);
    link(&mut s, "valid").unwrap();
    record(&mut s, expose("valid", "valid")).unwrap();
    let mut dup = q;
    dup["clave_evento"] = json!("duplicate-call");
    dup["secuencia"] = json!(41);
    capture(&mut s, &dup);
    assert!(link(&mut s, "duplicate-call").is_err());
    let mut other = ev("problem2", 50);
    other["rol"] = json!("problema");
    capture(&mut s, &other);
    record(
        &mut s,
        json!({"operacion":"intento","intento":"other","proyecto":"p","problema":"problem2"}),
    )
    .unwrap();
    let q = request(&s, "different-problem", 51);
    capture(&mut s, &q);
    record(
        &mut s,
        json!({"operacion":"enlazar","intento":"other","evento":"different-problem"}),
    )
    .unwrap();
    let mut x = expose("bad-problem", "different-problem");
    x["intento"] = json!("other");
    assert!(record(&mut s, x).is_err());
    assert_eq!(count(&p, "exposicion_memoria"), 1);
}
#[test]
fn evaluation_chains_null_conflicts_and_distinct_targets() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    let a = ev("output", 20);
    capture(&mut s, &a);
    link(&mut s, "output").unwrap();
    let b = ev("fixed", 21);
    capture(&mut s, &b);
    link(&mut s, "fixed").unwrap();
    for (key, seq, result, target, prev) in [
        ("fail", 30, json!(0), "output", None),
        ("pass", 31, json!(1), "fixed", None),
        ("unknown", 32, Value::Null, "output", Some("fail")),
    ] {
        capture(&mut s, &evaluation(key, seq, result, Some(target), prev));
        record(&mut s, evaluate(key)).unwrap();
    }
    let view = s.execution("p", "a").unwrap();
    assert!(view["criterios"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["estado"] != "conflicto"));
    assert_eq!(view["evaluaciones"][0]["vigente"], false);
    assert_eq!(view["evaluaciones"][2]["resultado"], Value::Null);
    capture(
        &mut s,
        &evaluation("independent", 33, json!(1), Some("output"), None),
    );
    record(&mut s, evaluate("independent")).unwrap();
    assert!(s.execution("p", "a").unwrap()["criterios"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["estado"] == "conflicto"));
    capture(
        &mut s,
        &evaluation("correct", 34, json!(1), Some("output"), Some("unknown")),
    );
    record(&mut s, evaluate("correct")).unwrap();
    assert!(s.execution("p", "a").unwrap()["criterios"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["estado"] != "conflicto"));
    for (key, seq, target, prev) in [
        ("branch", 35, "output", "unknown"),
        ("wrong-target", 36, "fixed", "correct"),
        ("cycle", 37, "output", "cycle"),
    ] {
        capture(
            &mut s,
            &evaluation(key, seq, json!(0), Some(target), Some(prev)),
        );
        assert!(record(&mut s, evaluate(key)).is_err());
    }
    assert_eq!(count(&p, "evaluacion_ejecucion"), 5);
}
#[test]
fn evaluation_rollback_keeps_raw_and_rejects_model_and_wrong_provenance() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    let e = evaluation("v", 20, json!(1), None, None);
    capture(&mut s, &e);
    let before = count(&p, "instancia");
    let conn = Connection::open(&p).unwrap();
    conn.execute_batch("CREATE TRIGGER inject BEFORE INSERT ON evaluacion_ejecucion BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(record(&mut s, evaluate("v")).is_err());
    assert_eq!(count(&p, "ejecucion_evento"), 0);
    assert_eq!(count(&p, "evaluacion_ejecucion"), 0);
    assert_eq!(count(&p, "instancia"), before);
    conn.execute_batch("DROP TRIGGER inject").unwrap();
    drop(s);
    let mut s = Store::open(&p).unwrap();
    record(&mut s, evaluate("v")).unwrap();
    for (i, kind) in ["model", "provenance", "result", "scope", "missingref"]
        .iter()
        .enumerate()
    {
        let key = format!("bad{i}");
        let mut e = evaluation(&key, 30 + i as i64, json!(1), None, None);
        match *kind {
            "model" => e["origen"] = json!("modelo"),
            "provenance" => e["metadatos"]["evaluacion"]["procedencia"] = json!("real"),
            "result" => e["metadatos"]["evaluacion"]["resultado"] = json!(2),
            "scope" => e["metadatos"]["evaluacion"]["alcance"] = json!("evento"),
            _ => e["referencias"] = json!([]),
        };
        capture(&mut s, &e);
        assert!(record(&mut s, evaluate(&key)).is_err());
    }
    assert_eq!(s.execution("p", "a").unwrap()["exposiciones"], json!([]));
    assert_eq!(count(&p, "evento_recuperacion"), 2);
}
#[test]
fn external_verifier_requires_observed_input_output_and_mapping() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    let target = ev("output", 20);
    capture(&mut s, &target);
    link(&mut s, "output").unwrap();
    for (i, kind) in ["valid", "wrong-result", "wrong-input", "missing-rule"]
        .iter()
        .enumerate()
    {
        let key = format!("v{i}");
        let mut e = evaluation(&key, 30 + i as i64, json!(1), Some("output"), None);
        e["origen"] = json!("herramienta");
        e["metadatos"]["evaluacion"]["fuente"] = json!("verificador_externo");
        e["metadatos"]["evaluacion"]["detalle"] = json!({"nombre":"igual-cinco","version":"1","entrada":{"objetivo":"output","contenido":"5"},"salida":{"cumple":true},"regla":{"campo":"/cumple","si_true":1,"si_false":0},"ejecucion":"real_local"});
        match *kind {
            "wrong-result" => e["metadatos"]["evaluacion"]["resultado"] = json!(0),
            "wrong-input" => {
                e["metadatos"]["evaluacion"]["detalle"]["entrada"]["contenido"] = json!("6")
            }
            "missing-rule" => e["metadatos"]["evaluacion"]["detalle"]["regla"] = Value::Null,
            _ => {}
        };
        capture(&mut s, &e);
        let result = record(&mut s, evaluate(&key));
        assert_eq!(result.is_ok(), *kind == "valid");
    }
}
#[test]
fn all_new_histories_are_immutable_and_database_integrity_holds() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    let q = request(&s, "q", 20);
    capture(&mut s, &q);
    link(&mut s, "q").unwrap();
    record(&mut s, expose("x", "q")).unwrap();
    capture(&mut s, &evaluation("v", 30, json!(1), Some("q"), None));
    record(&mut s, evaluate("v")).unwrap();
    let c = Connection::open(&p).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for (table, col) in [
        ("ejecucion", "id"),
        ("ejecucion_evento", "instancia_id"),
        ("exposicion_memoria", "id"),
        ("evaluacion_ejecucion", "id"),
    ] {
        assert!(c
            .execute(&format!("UPDATE {table} SET {col}={col}"), [])
            .is_err());
        assert!(c.execute(&format!("DELETE FROM {table}"), []).is_err());
    }
    assert_eq!(
        c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert!(!c
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_some());
    assert!(c
        .execute("UPDATE evento_recuperacion SET salida_instancia_id=1", [])
        .is_err());
}
#[test]
fn migrating_real_v3_preserves_raw_packet_and_seal() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let c = Connection::open(&p).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,sha256 TEXT NOT NULL,aplicado_en INTEGER NOT NULL)").unwrap();
    for (v, sql) in [
        (1, include_str!("../migrations/001_erd.sql")),
        (2, include_str!("../migrations/002_captura_tecnica.sql")),
        (
            3,
            include_str!("../migrations/003_recuperacion_textual.sql"),
        ),
    ] {
        c.execute_batch(sql).unwrap();
        c.execute(
            "INSERT INTO schema_migrations VALUES (?1,?2,0)",
            params![
                v,
                format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
            ],
        )
        .unwrap();
    }
    let raw = ev("legacy", 1).to_string();
    c.execute("INSERT INTO instancia(id,clave_evento,grupo_evidencia,origen,rol,contenido,metadatos_json,timestamp) VALUES (1,'legacy','B','herramienta','problema','5','{\"proyecto\":\"p\"}',1)",[]).unwrap();
    c.execute("INSERT INTO captura_evento VALUES (1,'B',1,?1,1)", [&raw])
        .unwrap();
    c.execute(
        "INSERT INTO configuracion(parametros_json,timestamp) VALUES ('{}',1)",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO evento_recuperacion(id,tarea_hash,clave_evento,problema_instancia_id,configuracion_id,timestamp) VALUES (1,'hash','legacy-packet',1,1,1)",[]).unwrap();
    c.execute("INSERT INTO evento_recuperacion_instancia VALUES (1,1)", [])
        .unwrap();
    c.execute(
        "INSERT INTO recuperacion_entrega_instancia VALUES (1,1,1,0,'ñ')",
        [],
    )
    .unwrap();
    let packet = json!({"texto_memoria":"ñ","version":1});
    c.execute(
        "INSERT INTO paquete_recuperacion VALUES (1,'p','{}',2,'ñ',2,?1)",
        [packet.to_string()],
    )
    .unwrap();
    drop(c);
    let s = Store::open(&p).unwrap();
    assert_eq!(s.memory_packet("p", "legacy-packet").unwrap(), packet);
    let c = Connection::open(&p).unwrap();
    assert_eq!(
        c.query_row("SELECT json_original FROM captura_evento", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        raw
    );
    assert!(c
        .execute("DELETE FROM recuperacion_entrega_instancia", [])
        .is_err());
    assert_eq!(count(&p, "schema_migrations"), 13);
}
#[test]
fn cli_separate_processes_persist_replay_and_conflict_without_ack() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    drop(setup(&p));
    let file = d.path().join("record.json");
    std::fs::write(
        &file,
        json!({"operacion":"intento","intento":"cli","proyecto":"p","problema":"problem"})
            .to_string(),
    )
    .unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_msp"))
            .arg(&p)
            .arg("record")
            .arg(&file)
            .output()
            .unwrap()
    };
    let a = run();
    assert!(a.status.success());
    assert_eq!(a.stdout, run().stdout);
    std::fs::write(&file,"{\n\"problema\": \"problem\",\"proyecto\":\"p\",\"intento\":\"cli\",\"operacion\":\"intento\"\n}").unwrap();
    assert_eq!(a.stdout, run().stdout);
    std::fs::write(
        &file,
        json!({"operacion":"intento","intento":"cli","proyecto":"wrong","problema":"problem"})
            .to_string(),
    )
    .unwrap();
    let failed = run();
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    let view = Command::new(env!("CARGO_BIN_EXE_msp"))
        .arg(&p)
        .args(["attempt", "p", "cli"])
        .output()
        .unwrap();
    assert!(view.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&view.stdout).unwrap()["estado_evaluacion"],
        "desconocido"
    );
}

#[test]
fn empty_packet_is_not_exposure_and_silent_call_remains_unknown() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = setup(&p);
    let problem = s.execution("p", "a").unwrap()["ejecucion"]["problema_instancia_id"]
        .as_i64()
        .unwrap();
    s.prepare_memory(
        "empty",
        &PrepareRequest {
            problema_id: problem,
            busqueda: SearchRequest {
                proyecto: "p".into(),
                consulta: "suma".into(),
                origen: None,
                rol: None,
                limite: 20,
            },
            presupuesto_bytes: 0,
        },
    )
    .unwrap();
    let mut q = request(&s, "q", 20);
    q["metadatos"]["solicitud"]["mensajes"][1]["contenido"] = json!("");
    q["metadatos"]["solicitud"]["bloques"][0] = json!({"paquete":"empty","posicion":1,"texto":""});
    capture(&mut s, &q);
    link(&mut s, "q").unwrap();
    let mut x = expose("x", "q");
    x["paquete"] = json!("empty");
    assert!(record(&mut s, x).is_err());
    let view = s.execution("p", "a").unwrap();
    assert_eq!(view["llamadas"][0]["estado_respuesta"], "desconocida");
    assert_eq!(view["estado_evaluacion"], "desconocido");
    assert_eq!(view["exposiciones"], json!([]));
    assert_eq!(
        s.unexposed_packets("p").unwrap()["paquetes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
#[test]
fn actor_fixture_preserves_cancellation_authorship_and_typed_shift_links() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = Store::open(&p).unwrap();
    for line in include_str!("../examples/ejecucion.jsonl").lines() {
        s.capture(line).unwrap();
    }
    record(&mut s,json!({"operacion":"intento","intento":"turno","proyecto":"tanda3","problema":"t3/turno/problema"})).unwrap();
    for suffix in ["abre", "cancela", "cierra"] {
        record(
            &mut s,
            json!({"operacion":"enlazar","intento":"turno","evento":format!("t3/turno/{suffix}")}),
        )
        .unwrap();
    }
    let before = s.execution("tanda3", "turno").unwrap();
    drop(s);
    let mut s = Store::open(&p).unwrap();
    assert_eq!(before, s.execution("tanda3", "turno").unwrap());
    assert_eq!(
        before["eventos"][1]["evento"]["metadatos"]["actor"],
        "Carla"
    );
    assert_eq!(
        before["eventos"][1]["evento"]["metadatos"]["accion"],
        "cancela"
    );
    assert_eq!(before["eventos"][2]["evento"]["metadatos"]["actor"], "Juan");
    assert_eq!(
        before["eventos"][2]["evento"]["referencias"][0]["tipo"],
        "cierra_turno_abierto"
    );
    assert_eq!(
        before["eventos"][1]["evento"]["metadatos"]["rol_vigente"],
        "operadora"
    );
    assert_eq!(before["criterios"], json!([]));
}

#[test]
fn evaluation_without_any_retrieval_or_context_is_valid() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("db");
    let mut s = Store::open(&p).unwrap();
    let mut problem = ev("problem", 1);
    problem["rol"] = json!("problema");
    capture(&mut s, &problem);
    attempt(&mut s, "a");
    capture(&mut s, &evaluation("v", 2, Value::Null, None, None));
    record(&mut s, evaluate("v")).unwrap();
    drop(s);
    let mut s = Store::open(&p).unwrap();
    let view = s.execution("p", "a").unwrap();
    assert_eq!(view["criterios"][0]["estado"], "desconocido");
    for table in [
        "evento_recuperacion",
        "paquete_recuperacion",
        "exposicion_memoria",
        "evento_feedback",
        "contexto",
    ] {
        assert_eq!(count(&p, table), 0);
    }
    assert_eq!(view["evaluaciones"].as_array().unwrap().len(), 1);
}
