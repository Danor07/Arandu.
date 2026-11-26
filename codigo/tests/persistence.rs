use msp::Store;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::time::Duration;
use tempfile::TempDir;

fn event(sequence: i64) -> String {
    json!({"version":1,"clave_evento":format!("test/session/{sequence}"),"grupo_evidencia":"episode","secuencia":sequence,"origen":"usuario","rol":"problema","timestamp":100-sequence,"contenido":"á\ncontenido íntegro","metadatos":{"tipo":"mensaje","extra":{"dato":42}}}).to_string()
}

fn database() -> (TempDir, std::path::PathBuf, Store) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let store = Store::open(&path).unwrap();
    (dir, path, store)
}

fn counts(path: &std::path::Path) -> [i64; 6] {
    let conn = Connection::open(path).unwrap();
    [
        "instancia",
        "observado_en",
        "trabajo_pendiente",
        "captura_evento",
        "evento_uso",
        "resultado_tecnico",
    ]
    .map(|table| {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    })
}

#[test]
fn version_del_sobre_se_valida_antes_de_persistir() {
    let (_dir, path, mut store) = database();
    let mut invalido: Value = serde_json::from_str(&event(1)).unwrap();
    invalido["version"] = json!(2);
    let error = store
        .capture(&invalido.to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("version JSONL no soportada"), "{error}");
    assert_eq!(counts(&path), [0; 6]);
}

#[test]
fn resultado_tecnico_describe_el_contenido_y_queda_guardado_en_el_episodio() {
    let (_dir, _path, mut store) = database();
    let mut crudo: Value = serde_json::from_str(&event(1)).unwrap();
    crudo["contenido"] = json!("á🦀");
    store.capture(&crudo.to_string()).unwrap();
    let lease = store.reserve(Duration::from_secs(60)).unwrap().unwrap();
    let esperado = json!({"procesamiento":"tecnico-v1","bytes_utf8":6,"caracteres_unicode":2,
        "sha256_contenido":"f70b0407afec443072ca1d5efcade962606016d77572230b8d04f606fcbfc861"});
    let calculado = store.calculate(&lease).unwrap();
    assert_eq!(calculado, esperado);
    store.finish(&lease, &calculado).unwrap();
    let episodio = store.episode("episode").unwrap();
    assert_eq!(
        episodio["eventos"][0]["trabajos"][0]["resultado_tecnico"],
        esperado
    );
}

#[test]
fn capture_rolls_back_everything_and_can_be_retried_after_reopen() {
    let (_dir, path, mut store) = database();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_queue BEFORE INSERT ON trabajo_pendiente BEGIN SELECT RAISE(ABORT,'injected storage failure'); END;").unwrap();
    assert!(store.capture(&event(1)).is_err());
    drop(store);
    assert_eq!(counts(&path), [0; 6]);
    conn.execute_batch("DROP TRIGGER fail_queue").unwrap();
    let mut store = Store::open(&path).unwrap();
    assert!(store.capture(&event(1)).unwrap().insertado);
    drop(store);
    assert_eq!(counts(&path), [1, 1, 1, 1, 0, 0]);
}

#[test]
fn retries_are_identical_and_all_envelope_changes_conflict() {
    let (_dir, path, mut store) = database();
    let raw = event(1);
    let receipt = store.capture(&raw).unwrap();
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let repeated = store.capture(&raw).unwrap();
    assert!(!repeated.insertado);
    assert_eq!(receipt.instancia_id, repeated.instancia_id);
    let mut modified: Value = serde_json::from_str(&raw).unwrap();
    modified["metadatos"]["extra"]["dato"] = json!(43);
    assert!(store
        .capture(&modified.to_string())
        .unwrap_err()
        .to_string()
        .contains("conflicto"));
    assert!(store.capture(&(raw.clone() + " ")).is_err());
    modified["clave_evento"] = json!("different-key");
    assert!(store
        .capture(&modified.to_string())
        .unwrap_err()
        .to_string()
        .contains("secuencia"));
    assert_eq!(
        store.episode("episode").unwrap()["eventos"][0]["json_original"],
        raw
    );
    assert_eq!(counts(&path), [1, 1, 1, 1, 0, 0]);
}

#[test]
fn expired_and_replaced_tokens_cannot_finish_or_fail() {
    let (_dir, path, mut store) = database();
    store.capture(&event(1)).unwrap();
    let old = store.reserve(Duration::from_secs(30)).unwrap().unwrap();
    let result = store.calculate(&old).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE trabajo_pendiente SET reserva_hasta=0 WHERE id=?1",
        [old.trabajo_id],
    )
    .unwrap();
    assert!(store.finish(&old, &result).is_err());
    assert!(store.fail(&old, "stale").is_err());
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let new = store.reserve(Duration::from_secs(30)).unwrap().unwrap();
    assert_eq!(new.trabajo_id, old.trabajo_id);
    assert_ne!(new.token, old.token);
    assert!(store.finish(&old, &result).is_err());
    assert!(store.fail(&old, "stale").is_err());
    assert!(store.finish(&new, &result).unwrap());
    assert!(store.finish(&old, &result).is_err());
    drop(store);
    let mut store = Store::open(&path).unwrap();
    assert!(!store.finish(&new, &result).unwrap());
    assert!(store.finish(&new, &json!({"different":true})).is_err());
    assert_eq!(counts(&path), [1, 1, 1, 1, 0, 1]);
}

#[test]
fn result_and_completion_roll_back_together_then_finish_once() {
    let (_dir, path, mut store) = database();
    store.capture(&event(1)).unwrap();
    let lease = store.reserve(Duration::from_secs(60)).unwrap().unwrap();
    let result = store.calculate(&lease).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_result BEFORE INSERT ON resultado_tecnico BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.finish(&lease, &result).is_err());
    drop(store);
    let state: String = conn
        .query_row("SELECT estado FROM trabajo_pendiente", [], |r| r.get(0))
        .unwrap();
    assert_eq!(state, "en_proceso");
    assert_eq!(counts(&path)[5], 0);
    conn.execute_batch("DROP TRIGGER fail_result").unwrap();
    let mut store = Store::open(&path).unwrap();
    assert!(store.finish(&lease, &result).unwrap());
    assert!(!store.finish(&lease, &result).unwrap());
    assert!(store.reserve(Duration::from_secs(1)).unwrap().is_none());
    let complete_logs: i64 = conn
        .query_row(
            "SELECT count(*) FROM trabajo_historial WHERE tipo='completado'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(complete_logs, 1);
}

#[test]
fn raw_and_erd_histories_remain_immutable_after_processing() {
    let (_dir, path, mut store) = database();
    let raw = event(1);
    store.capture(&raw).unwrap();
    let lease = store.reserve(Duration::from_secs(60)).unwrap().unwrap();
    store
        .finish(&lease, &store.calculate(&lease).unwrap())
        .unwrap();
    drop(store);
    let conn = Connection::open(&path).unwrap();
    for sql in [
        "DELETE FROM instancia",
        "UPDATE instancia SET contenido='lost'",
        "DELETE FROM observado_en",
        "UPDATE configuracion SET timestamp=0",
        "DELETE FROM captura_evento",
        "DELETE FROM trabajo_pendiente",
        "DELETE FROM resultado_tecnico",
        "DELETE FROM trabajo_historial",
    ] {
        assert!(conn.execute(sql, []).is_err(), "{sql}");
    }
    let mut store = Store::open(&path).unwrap();
    assert_eq!(
        store.episode("episode").unwrap()["eventos"][0]["json_original"],
        raw
    );
    assert_eq!(counts(&path), [1, 1, 1, 1, 0, 1]);
    assert_eq!(
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
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
fn episode_reconstructs_attempts_adjustments_and_explicit_links_in_source_order() {
    let (_dir, path, mut store) = database();
    let lines: Vec<_> = include_str!("../examples/episodio.jsonl").lines().collect();
    // Las referencias pueden preceder a sus destinos: se informa disponibilidad.
    store.capture(lines[9]).unwrap();
    assert_eq!(
        store.episode("demo/episodio-1").unwrap()["eventos"][0]["referencias"][0]["disponible"],
        false
    );
    for line in lines[..9].iter().rev() {
        store.capture(line).unwrap();
    }
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let episode = store.episode("demo/episodio-1").unwrap();
    let events = episode["eventos"].as_array().unwrap();
    assert_eq!(events.len(), 10);
    for (index, row) in events.iter().enumerate() {
        assert_eq!(row["secuencia"], (index + 1) as i64);
        assert_eq!(row["json_original"], lines[index]);
        for link in row["referencias"].as_array().unwrap() {
            assert_eq!(link["disponible"], true);
        }
    }
    assert_eq!(events[4]["uso"]["tipo"], "ajuste");
    assert_eq!(events[6]["uso"]["tipo"], "interrupcion");
    assert!(events[1]["uso"]["evento_recuperacion_id"].is_null());
    assert!(events[1]["uso"]["contenido_id"].is_null());
    assert_eq!(events[1]["uso"]["metricas"], json!({}));
    assert!(events[3]["uso"]["metricas"]["tokens_salida"].is_null());
    assert_eq!(
        events[8]["evento"]["metadatos"]["evaluacion"]["resultado"],
        1
    );
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM evento_feedback", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(counts(&path), [10, 10, 10, 10, 8, 0]);
}

#[test]
fn conflicting_attempt_episode_rolls_back_capture_and_use() {
    let (_dir, path, mut store) = database();
    let mut first: Value = serde_json::from_str(&event(1)).unwrap();
    first["uso"] = json!({"intento_clave":"attempt","tipo":"inicio"});
    store.capture(&first.to_string()).unwrap();
    first["clave_evento"] = json!("other");
    first["grupo_evidencia"] = json!("another episode");
    assert!(store.capture(&first.to_string()).is_err());
    drop(store);
    assert_eq!(counts(&path), [1, 1, 1, 1, 1, 0]);
}

#[test]
fn concurrent_workers_do_not_own_the_same_unexpired_job() {
    let (_dir, path, mut store) = database();
    store.capture(&event(1)).unwrap();
    drop(store);
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = Store::open(path).unwrap();
                barrier.wait();
                store.reserve(Duration::from_secs(60)).unwrap()
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .filter_map(|h| h.join().unwrap())
            .count(),
        1
    );
}

#[test]
fn technical_failures_back_off_remain_in_history_and_can_resume() {
    let (_dir, path, mut store) = database();
    store.capture(&event(1)).unwrap();
    let conn = Connection::open(&path).unwrap();
    let mut previous_delay = 0;
    for index in 1..=5 {
        let lease = store.reserve(Duration::from_secs(60)).unwrap().unwrap();
        store.fail(&lease, "technical failure").unwrap();
        let (available,time):(i64,i64)=conn.query_row("SELECT t.disponible_desde,h.timestamp FROM trabajo_pendiente t JOIN trabajo_historial h ON h.trabajo_id=t.id ORDER BY h.id DESC LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert!(available - time > previous_delay);
        assert_eq!(
            available - time,
            [100_000, 200_000, 400_000, 800_000, 1_600_000][index - 1]
        );
        previous_delay = available - time;
        // Evitar que la carga del host convierta esta comprobación en una
        // carrera contra el primer backoff de 100 ms.
        conn.execute(
            "UPDATE trabajo_pendiente SET disponible_desde=?1",
            [i64::MAX],
        )
        .unwrap();
        assert!(store.reserve(Duration::from_secs(1)).unwrap().is_none());
        if index < 5 {
            conn.execute("UPDATE trabajo_pendiente SET disponible_desde=0", [])
                .unwrap();
        }
    }
    drop(store);
    let mut store = Store::open(&path).unwrap();
    store.retry(1).unwrap();
    let lease = store.reserve(Duration::from_secs(60)).unwrap().unwrap();
    store
        .finish(&lease, &store.calculate(&lease).unwrap())
        .unwrap();
    assert_eq!(counts(&path), [1, 1, 1, 1, 0, 1]);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM trabajo_historial WHERE tipo IN ('reintento','fallido')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        5
    );
}

#[test]
fn migrations_reopen_without_duplication_and_detect_tampering() {
    // WAL requiere la corrección de corrupción incorporada en SQLite 3.51.3.
    assert!(rusqlite::version_number() >= 3_051_003);
    let (_dir, path, store) = database();
    drop(store);
    drop(Store::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM configuracion", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    conn.execute(
        "UPDATE schema_migrations SET sha256='tampered' WHERE version=1",
        [],
    )
    .unwrap();
    assert!(Store::open(&path).is_err());
}

#[test]
fn killed_worker_restarts_and_jsonl_reimports_without_loss_or_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("process.sqlite");
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/episodio.jsonl");
    let binary = env!("CARGO_BIN_EXE_msp");
    let imported = Command::new(binary)
        .arg(&path)
        .arg("import")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(
        imported.status.success(),
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    assert_eq!(
        String::from_utf8(imported.stdout).unwrap().lines().count(),
        10
    );
    let mut child = Command::new(binary)
        .arg(&path)
        .args([
            "work",
            "--lease-ms",
            "100",
            "--pause-after-reserve-ms",
            "60000",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("reservado")); // Reserva confirmada y stdout vaciado.
    child.kill().unwrap();
    child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(120));
    let processed = Command::new(binary)
        .arg(&path)
        .arg("work")
        .output()
        .unwrap();
    assert!(
        processed.status.success(),
        "{}",
        String::from_utf8_lossy(&processed.stderr)
    );
    let repeated = Command::new(binary)
        .arg(&path)
        .arg("import")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(repeated.status.success());
    for line in String::from_utf8(repeated.stdout).unwrap().lines() {
        assert_eq!(
            serde_json::from_str::<Value>(line).unwrap()["insertado"],
            false
        );
    }
    let output = Command::new(binary)
        .arg(&path)
        .args(["episode", "demo/episodio-1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let episode: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(episode["eventos"].as_array().unwrap().len(), 10);
    assert_eq!(episode["eventos"][0]["trabajos"][0]["reservas_totales"], 2);
    for row in episode["eventos"].as_array().unwrap() {
        assert_eq!(row["trabajos"][0]["estado"], "completado");
    }
    assert_eq!(counts(&path), [10, 10, 10, 10, 8, 10]);
}

#[test]
fn erd_injected_version_constraints_are_preserved() {
    let (_dir, path, mut store) = database();
    store.capture(&event(1)).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON;
        INSERT INTO dominio VALUES(1,'project');
        INSERT INTO contexto VALUES(1,1,'provisional',1,1);
        INSERT INTO contexto_contenido_evento VALUES(1,1,1,'observed','inicial',1,NULL);
        INSERT INTO evento_recuperacion(id,tarea_hash,clave_evento,problema_instancia_id,configuracion_id,timestamp) VALUES(1,'hash','retrieval',1,1,1);
        INSERT INTO evento_recuperacion_contexto(evento_id,contexto_id,contenido_id,similitud,evaluado_detalle,score,fue_inyectado) VALUES(1,1,1,0.5,1,0.5,0);").unwrap();
    assert!(conn.execute("INSERT INTO evento_uso(instancia_id,intento_clave,evento_recuperacion_id,contenido_id,tipo,timestamp) VALUES(1,'attempt',1,1,'inicio',1)",[]).is_err());
    assert!(conn.execute("INSERT INTO evento_feedback(evento_recuperacion_id,clave_evento,clave_evaluacion,contenido_objetivo_id,tipo,fuente,resultado,timestamp) VALUES(1,'f','test',1,'confirmacion_explicita','usuario_explicito',1,1)",[]).is_err());
    conn.execute("INSERT INTO pertenece_a(instancia_id,contexto_id,vigente_desde,motivo) VALUES (1,1,10,'inicial')",[]).unwrap();
    conn.execute("INSERT INTO pertenece_a(instancia_id,contexto_id,vigente_desde,motivo) VALUES (1,1,20,'revision')",[]).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT vigente_hasta FROM pertenece_a_vigencia WHERE vigente_desde=?1",
            params![10],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        20
    );
}

#[test]
fn importer_acknowledges_only_committed_lines_and_can_resume_the_file() {
    let (_dir, path, store) = database();
    drop(store);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TRIGGER fail_second_event BEFORE INSERT ON trabajo_pendiente
        WHEN (SELECT clave_evento FROM instancia WHERE id=NEW.instancia_id)='demo/local/sesion-1/e2'
        BEGIN SELECT RAISE(ABORT,'injected queue failure'); END;",
    )
    .unwrap();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/episodio.jsonl");
    let binary = env!("CARGO_BIN_EXE_msp");
    let failed = Command::new(binary)
        .arg(&path)
        .arg("import")
        .arg(&fixture)
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    let receipts: Vec<Value> = String::from_utf8(failed.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0]["clave_evento"], "demo/local/sesion-1/e1");
    assert_eq!(counts(&path), [1, 1, 1, 1, 0, 0]);
    conn.execute_batch("DROP TRIGGER fail_second_event")
        .unwrap();
    let resumed = Command::new(binary)
        .arg(&path)
        .arg("import")
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(resumed.status.success());
    let receipts: Vec<Value> = String::from_utf8(resumed.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(receipts.len(), 10);
    assert_eq!(receipts[0]["insertado"], false);
    assert_eq!(counts(&path), [10, 10, 10, 10, 8, 0]);
}

#[test]
fn separate_attempts_and_nonmonotonic_timestamps_do_not_change_source_order() {
    let (_dir, path, mut store) = database();
    for (sequence, attempt, kind) in [
        (3, "second", "ajuste"),
        (1, "first", "inicio"),
        (2, "second", "inicio"),
    ] {
        let mut value: Value = serde_json::from_str(&event(sequence)).unwrap();
        value["uso"] = json!({"intento_clave":attempt,"tipo":kind});
        store.capture(&value.to_string()).unwrap();
    }
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let episode = store.episode("episode").unwrap();
    let rows = episode["eventos"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row["secuencia"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(rows[0]["uso"]["intento_clave"], "first");
    assert_eq!(rows[1]["uso"]["intento_clave"], "second");
    assert_eq!(rows[2]["uso"]["tipo"], "ajuste");
    assert!(
        rows[0]["evento"]["timestamp"].as_i64().unwrap()
            > rows[2]["evento"]["timestamp"].as_i64().unwrap()
    );
    assert!(rows
        .iter()
        .all(|row| row["referencias"].as_array().unwrap().is_empty()));
}
