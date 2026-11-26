use msp::{
    retrieval::{PrepareRequest, SearchRequest},
    Store,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{path::Path, process::Command};

fn query(text: &str) -> SearchRequest {
    SearchRequest {
        proyecto: "calculadora".into(),
        consulta: text.into(),
        origen: None,
        rol: None,
        limite: 100,
    }
}
fn event(key: &str, episode: &str, sequence: i64, content: &str) -> String {
    json!({"version":1,"clave_evento":key,"grupo_evidencia":episode,"secuencia":sequence,"timestamp":sequence,
        "origen":"usuario","rol":"problema","contenido":content,"metadatos":{"proyecto":"calculadora"}}).to_string()
}
fn fixture(store: &mut Store) -> i64 {
    let mut problem = 0;
    for line in include_str!("../examples/recuperacion.jsonl").lines() {
        let receipt = store.capture(line).unwrap();
        if receipt.clave_evento == "demo/B/1" {
            problem = receipt.instancia_id;
        }
    }
    problem
}
fn prepare(problem: i64) -> PrepareRequest {
    PrepareRequest {
        problema_id: problem,
        busqueda: query("suma enteros"),
        presupuesto_bytes: 4096,
    }
}
fn count(path: &Path, table: &str) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn migration_indexes_old_rows_and_new_capture_is_atomic_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,sha256 TEXT NOT NULL,aplicado_en INTEGER NOT NULL)").unwrap();
    for (v, sql) in [
        (1, include_str!("../migrations/001_erd.sql")),
        (2, include_str!("../migrations/002_captura_tecnica.sql")),
    ] {
        conn.execute_batch(sql).unwrap();
        let hash = format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()));
        conn.execute(
            "INSERT INTO schema_migrations VALUES (?1,?2,0)",
            params![v, hash],
        )
        .unwrap();
    }
    let raw = event("old", "A", 1, "suma legado");
    conn.execute("INSERT INTO instancia(id,clave_evento,grupo_evidencia,origen,rol,contenido,metadatos_json,timestamp) VALUES (1,'old','A','usuario','problema','suma legado','{\"proyecto\":\"calculadora\"}',1)",[]).unwrap();
    conn.execute("INSERT INTO captura_evento VALUES (1,'A',1,?1,1)", [&raw])
        .unwrap();
    conn.execute(
        "INSERT INTO observado_en(instancia_id,timestamp) VALUES (1,1)",
        [],
    )
    .unwrap();
    drop(conn);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.search(&query("legado")).unwrap().resultados[0].instancia_id,
        1
    );
    assert!(!store.capture(&raw).unwrap().insertado);
    let new = event("new", "C", 2, "suma novedosa");
    store.capture(&new).unwrap();
    store.capture(&new).unwrap();
    assert_eq!(store.search(&query("suma")).unwrap().resultados.len(), 2);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_queue BEFORE INSERT ON trabajo_pendiente BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store
        .capture(&event("rollback", "D", 3, "irrepetible"))
        .is_err());
    assert!(store
        .search(&query("irrepetible"))
        .unwrap()
        .resultados
        .is_empty());
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.search(&query("suma")).unwrap().resultados.len(), 2);
    assert_eq!(count(&path, "schema_migrations"), 13);
}

#[test]
fn project_origin_role_and_missing_project_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    fixture(&mut store);
    let mut q = query("suma enteros");
    q.origen = Some("modelo".into());
    q.rol = Some("correccion".into());
    let found = store.search(&q).unwrap();
    assert_eq!(found.sin_proyecto_en_base, 1);
    assert_eq!(found.resultados.len(), 1);
    assert_eq!(found.resultados[0].clave_evento, "demo/A/3");
    q.proyecto = "otro-proyecto".into();
    assert_eq!(
        store.search(&q).unwrap().resultados[0].clave_evento,
        "otro/A/1"
    );
    assert!(store
        .detail("otro-proyecto", found.resultados[0].instancia_id, 1, 1)
        .is_err());
    q.proyecto = " ".into();
    assert!(store.search(&q).is_err());
    assert_eq!(count(&path, "evento_recuperacion"), 0);
}

#[test]
fn empty_special_and_no_match_queries_do_not_execute_fts_syntax() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    fixture(&mut store);
    for q in ["", " \t\n", "\"*():{} - + ^", "terminoausenteqzx", "🚀"] {
        assert!(
            store.search(&query(q)).unwrap().resultados.is_empty(),
            "{q}"
        );
    }
    for q in [
        "suma OR (",
        "suma\"* NOT :",
        "suma'; DROP TABLE instancia; --",
        "suma\0enteros",
    ] {
        assert!(
            !store.search(&query(q)).unwrap().resultados.is_empty(),
            "{q}"
        );
    }
    assert_eq!(count(&path, "instancia"), 8);
    let mut q = query("suma");
    q.limite = 0;
    assert!(store.search(&q).unwrap().resultados.is_empty());
}

#[test]
fn rank_and_ties_are_stable_without_merging_identical_episodes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    store.capture(&event("z", "Z", 1, "aguja")).unwrap();
    store.capture(&event("a", "A", 1, "aguja")).unwrap();
    store
        .capture(&event(
            "long",
            "L",
            1,
            &format!("aguja {}", "relleno ".repeat(80)),
        ))
        .unwrap();
    let found = store.search(&query("aguja")).unwrap().resultados;
    assert_eq!(
        found
            .iter()
            .map(|h| h.clave_evento.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "z", "long"]
    );
    assert_eq!(found[0].score_textual, found[1].score_textual);
    assert!(found[0].score_textual < found[2].score_textual);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    store.rebuild_search().unwrap();
    assert_eq!(found, store.search(&query("aguja")).unwrap().resultados);
}

#[test]
fn detail_uses_sequence_gaps_and_filters_every_neighbor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    fixture(&mut store);
    let mut q = query("suma");
    q.rol = Some("correccion".into());
    let id = store.search(&q).unwrap().resultados[0].instancia_id;
    let mut foreign: Value =
        serde_json::from_str(&event("intruso", "episodio-A", 25, "suma")).unwrap();
    foreign["metadatos"]["proyecto"] = json!("otro-proyecto");
    store.capture(&foreign.to_string()).unwrap();
    let detail = store.detail("calculadora", id, 2, 1).unwrap();
    let events = detail["eventos"].as_array().unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| e["secuencia"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![10, 20, 30, 40]
    );
    assert_eq!(events[1]["evento"]["uso"]["tipo"], "prueba");
    assert_eq!(
        events[3]["evento"]["metadatos"]["evaluacion"]["resultado"],
        1
    );
    assert_eq!(
        store.detail("calculadora", id, 0, 0).unwrap()["eventos"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(count(&path, "evento_recuperacion"), 0);
}

#[test]
fn packages_exclude_own_episode_late_capture_and_future_source_time() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .capture(&event("future-source", "F", 999, "suma enteros"))
        .unwrap();
    let id = fixture(&mut store);
    store
        .capture(&event("late-import", "L", 1, "suma enteros"))
        .unwrap();
    let packet = store.prepare_memory("package", &prepare(id)).unwrap();
    let selected = packet["seleccion"].as_array().unwrap();
    assert_eq!(selected.len(), 4);
    assert!(selected
        .iter()
        .all(|v| v["candidato"]["grupo_evidencia"] == "episodio-A"));
    assert_eq!(count(&path, "evento_recuperacion_instancia"), 4);
    assert_eq!(count(&path, "contexto"), 0);
    assert_eq!(count(&path, "evento_recuperacion_contexto"), 0);
}

#[test]
fn byte_budget_includes_jsonl_framing_preserves_unicode_and_skips_whole_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    store
        .capture(&event("a", "A", 1, "aguja á🚀\né\"終"))
        .unwrap();
    let id = store
        .capture(&event("b", "B", 2, "aguja nueva"))
        .unwrap()
        .instancia_id;
    let mut request = prepare(id);
    request.busqueda = query("aguja");
    let large = store.prepare_memory("large", &request).unwrap();
    let text = large["texto_memoria"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap()["contenido"],
        "aguja á🚀\né\"終"
    );
    request.presupuesto_bytes = text.len();
    assert_eq!(
        store.prepare_memory("exact", &request).unwrap()["texto_memoria"],
        text
    );
    request.presupuesto_bytes = text.len() - 1;
    let small = store.prepare_memory("small", &request).unwrap();
    assert_eq!(small["texto_memoria"], "");
    assert_eq!(small["omitidos"].as_array().unwrap().len(), 1);
    request.presupuesto_bytes = 0;
    assert_eq!(
        store.prepare_memory("zero", &request).unwrap()["bytes_utf8"],
        0
    );
    request.busqueda = query("noexiste");
    assert!(store.prepare_memory("none", &request).unwrap()["seleccion"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(count(&path, "instancia"), 2);
}

#[test]
fn packet_is_atomic_immutable_and_idempotent_after_reopen_and_corpus_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    let id = fixture(&mut store);
    let request = prepare(id);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_packet BEFORE INSERT ON paquete_recuperacion BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.prepare_memory("key", &request).is_err());
    for table in [
        "evento_recuperacion",
        "evento_recuperacion_instancia",
        "recuperacion_entrega_instancia",
        "paquete_recuperacion",
    ] {
        assert_eq!(count(&path, table), 0);
    }
    conn.execute_batch("DROP TRIGGER fail_packet").unwrap();
    let packet = store.prepare_memory("key", &request).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    store
        .capture(&event("extra", "extra", 1, "suma enteros extra"))
        .unwrap();
    store.rebuild_search().unwrap();
    assert_eq!(packet, store.prepare_memory("key", &request).unwrap());
    assert_eq!(packet, store.memory_packet("calculadora", "key").unwrap());
    assert!(store.memory_packet("otro-proyecto", "key").is_err());
    let mut changed = request.clone();
    changed.presupuesto_bytes += 1;
    assert!(store
        .prepare_memory("key", &changed)
        .unwrap_err()
        .to_string()
        .contains("conflicto"));
    changed = request.clone();
    changed.busqueda.consulta += " ";
    assert!(store.prepare_memory("key", &changed).is_err());
    changed = request.clone();
    changed.busqueda.proyecto = "otro-proyecto".into();
    assert!(store.prepare_memory("key", &changed).is_err());
    let mut stmt = conn
        .prepare("SELECT texto_entregado FROM recuperacion_entrega_instancia ORDER BY orden")
        .unwrap();
    let pieces = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(pieces.concat(), packet["texto_memoria"].as_str().unwrap());
    assert_eq!(count(&path, "evento_recuperacion"), 1);
    for sql in [
        "DELETE FROM paquete_recuperacion",
        "UPDATE recuperacion_entrega_instancia SET orden=20",
        "DELETE FROM evento_recuperacion_instancia",
        "UPDATE evento_recuperacion SET tarea_hash='modified'",
    ] {
        assert!(conn.execute(sql, []).is_err());
    }
}

#[test]
fn damaged_index_rebuild_does_not_change_raw_history_or_packages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    let id = fixture(&mut store);
    let before = store.episode("episodio-A").unwrap();
    let packet = store.prepare_memory("saved", &prepare(id)).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute("INSERT INTO instancia_fts(instancia_fts,rowid,contenido) SELECT 'delete',id,contenido FROM instancia WHERE clave_evento='demo/A/3'",[]).unwrap();
    let mut q = query("reemplazar");
    q.rol = Some("correccion".into());
    assert!(store.search(&q).unwrap().resultados.is_empty());
    store.rebuild_search().unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.search(&q).unwrap().resultados.len(), 1);
    assert_eq!(before, store.episode("episodio-A").unwrap());
    assert_eq!(packet, store.memory_packet("calculadora", "saved").unwrap());
    assert_eq!(count(&path, "instancia"), 8);
    assert!(conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query([])
        .unwrap()
        .next()
        .unwrap()
        .is_none());
}

#[test]
fn cli_prepares_and_reopens_identical_packet_and_emits_nothing_on_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let mut store = Store::open(&path).unwrap();
    let id = fixture(&mut store);
    drop(store);
    let bin = env!("CARGO_BIN_EXE_msp");
    let args = [
        "prepare",
        "calculadora",
        &id.to_string(),
        "cli",
        "suma enteros",
        "--budget-bytes",
        "700",
    ];
    let first = Command::new(bin).arg(&path).args(args).output().unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = Command::new(bin).arg(&path).args(args).output().unwrap();
    assert_eq!(first.stdout, second.stdout);
    let read = Command::new(bin)
        .arg(&path)
        .args(["packet", "calculadora", "cli"])
        .output()
        .unwrap();
    assert_eq!(first.stdout, read.stdout);
    let conflict = Command::new(bin)
        .arg(&path)
        .args([
            "prepare",
            "calculadora",
            &id.to_string(),
            "cli",
            "different",
        ])
        .output()
        .unwrap();
    assert!(!conflict.status.success());
    assert!(conflict.stdout.is_empty());
}
