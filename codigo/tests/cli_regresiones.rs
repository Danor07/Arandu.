//! Contratos observables de argumentos, filtros y drenado de la cola.
use msp::execution::Record;
use msp::retrieval::{PrepareRequest, SearchRequest};
use msp::Store;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn cli(raiz: &Path, args: &[&str]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_msp"))
        .args(args)
        .env("MSP_RAIZ", raiz.join("control"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let inicio = Instant::now();
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        // Un parser que no avanza debe fallar antes del timeout de mutación.
        if inicio.elapsed() > Duration::from_secs(10) {
            child.kill().unwrap();
            let salida = child.wait_with_output().unwrap();
            panic!(
                "CLI trabada con {args:?}: {}",
                String::from_utf8_lossy(&salida.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn correcto(raiz: &Path, args: &[&str]) -> Vec<Value> {
    let salida = cli(raiz, args);
    assert!(
        salida.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&salida.stderr)
    );
    String::from_utf8(salida.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn evento(clave: &str, secuencia: i64, rol: &str) -> String {
    json!({"version":1,"clave_evento":clave,"grupo_evidencia":"g","secuencia":secuencia,
        "timestamp":secuencia,"origen":"usuario","rol":rol,"contenido":"aguja",
        "metadatos":{"proyecto":"p"}})
    .to_string()
}

#[test]
fn ayuda_y_cantidades_de_argumentos_se_respetan() {
    let d = tempfile::tempdir().unwrap();
    for args in [vec![], vec!["--help"]] {
        let salida = cli(d.path(), &args);
        assert!(salida.status.success());
        assert!(String::from_utf8(salida.stdout)
            .unwrap()
            .contains("memoria BASE"));
    }
    let db = d.path().join("db.sqlite");
    let db = db.to_str().unwrap();
    for args in [
        vec![db, "exposures"],
        vec![db, "attempt", "p", "a", "sobrante"],
        vec![db, "retry", "1", "sobrante"],
        vec![db, "record", "contrato-ausente.json", "sobrante"],
        vec![db, "unexposed", "p", "sobrante"],
        vec![db, "search", "p"],
        vec![db, "detail", "p"],
        vec![db, "prepare", "p"],
        vec![db, "reindex", "sobrante"],
        vec![db, "import", "fuente-ausente.jsonl", "sobrante"],
        vec![db, "episode", "g", "sobrante"],
    ] {
        let salida = cli(d.path(), &args);
        assert_eq!(salida.status.code(), Some(1));
        assert!(
            String::from_utf8(salida.stderr)
                .unwrap()
                .contains("comando o argumentos invalidos"),
            "{args:?}"
        );
    }
}

#[test]
fn preparar_y_reanudar_por_cli_tienen_efecto_observable_sin_esperar_otras_suites() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db.sqlite");
    let mut s = Store::open(&path).unwrap();
    let mut anterior: Value = serde_json::from_str(&evento("antecedente", 1, "solucion")).unwrap();
    anterior["grupo_evidencia"] = json!("anterior");
    s.capture(&anterior.to_string()).unwrap();
    let problema = s
        .capture(&evento("pregunta", 2, "problema"))
        .unwrap()
        .instancia_id
        .to_string();
    drop(s);
    let db = path.to_str().unwrap();
    for (clave, extras) in [
        ("minima", vec![]),
        ("opciones", vec!["--budget-bytes", "4096"]),
    ] {
        let mut args = vec![db, "prepare", "p", &problema, clave, "aguja"];
        args.extend(extras);
        let paquete = correcto(d.path(), &args);
        assert_eq!(paquete[0]["clave_paquete"], clave);
        assert!(paquete[0]["texto_memoria"]
            .as_str()
            .unwrap()
            .contains("antecedente"));
    }
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute("UPDATE trabajo_pendiente SET estado='fallido',intentos=5,ultimo_error='fixture' WHERE id=1",[]).unwrap();
    assert_eq!(
        correcto(d.path(), &[db, "retry", "1"])[0],
        json!({"reanudado":1})
    );
    assert_eq!(
        c.query_row(
            "SELECT estado,intentos,ultimo_error FROM trabajo_pendiente WHERE id=1",
            [],
            |r| Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<String>>(2)?
            ))
        )
        .unwrap(),
        ("pendiente".into(), 5, Some("fixture".into()))
    );
}

#[test]
fn filtros_vecinos_y_reindexado_llegan_a_la_base() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("db.sqlite");
    let mut s = Store::open(&db).unwrap();
    let a = s
        .capture(&evento("a", 10, "problema"))
        .unwrap()
        .instancia_id;
    let b = s
        .capture(&evento("b", 20, "solucion"))
        .unwrap()
        .instancia_id;
    let c = s
        .capture(&evento("c", 30, "correccion"))
        .unwrap()
        .instancia_id;
    drop(s);
    let db = db.to_str().unwrap();
    let buscar = correcto(
        d.path(),
        &[
            db, "search", "p", "aguja", "--role", "solucion", "--origin", "usuario", "--limit", "1",
        ],
    );
    assert_eq!(buscar[0]["resultados"].as_array().unwrap().len(), 1);
    assert_eq!(buscar[0]["resultados"][0]["instancia_id"], b);
    // El presupuesto de bytes es sólo de prepare: search lo rechaza.
    let ajeno = cli(
        d.path(),
        &[db, "search", "p", "aguja", "--budget-bytes", "10"],
    );
    assert!(!ajeno.status.success());
    assert!(String::from_utf8_lossy(&ajeno.stderr).contains("opcion desconocida: --budget-bytes"));
    let detalle = correcto(
        d.path(),
        &[
            db,
            "detail",
            "p",
            &b.to_string(),
            "--before",
            "1",
            "--after",
            "1",
        ],
    );
    // El JSON de detail conserva los vecinos en orden de secuencia.
    assert_eq!(
        detalle[0]["eventos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["instancia_id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![a, b, c]
    );
    assert_eq!(
        correcto(d.path(), &[db, "reindex"])[0],
        json!({"indice":"reconstruido"})
    );
}

#[test]
fn exposiciones_acepta_cero_uno_y_dos_filtros() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("db.sqlite");
    let mut s = Store::open(&db).unwrap();
    let mut antecedente: Value =
        serde_json::from_str(&evento("antecedente", 1, "solucion")).unwrap();
    antecedente["grupo_evidencia"] = json!("anterior");
    s.capture(&antecedente.to_string()).unwrap();
    let problema = s
        .capture(&evento("problema", 10, "problema"))
        .unwrap()
        .instancia_id;
    s.record_execution(&Record::Intento {
        intento: "a".into(),
        proyecto: "p".into(),
        problema: "problema".into(),
    })
    .unwrap();
    let paquete = s
        .prepare_memory(
            "paquete",
            &PrepareRequest {
                problema_id: problema,
                busqueda: SearchRequest {
                    proyecto: "p".into(),
                    consulta: "aguja".into(),
                    origen: None,
                    rol: None,
                    limite: 20,
                },
                presupuesto_bytes: 4096,
            },
        )
        .unwrap();
    for (llamada, secuencia) in [("llamada", 20), ("otra", 30)] {
        let q = json!({"version":1,"clave_evento":llamada,"grupo_evidencia":"g","secuencia":secuencia,
            "timestamp":secuencia,"origen":"herramienta","rol":"otro","contenido":"solicitud",
            "metadatos":{"proyecto":"p","solicitud":{"version":1,"llamada_id":llamada,
                "procedencia":"simulada","frontera":"envio_observado","evidencia_envio":"sumidero local",
                "mensajes":[{"rol":"system","contenido":paquete["texto_memoria"]}],
                "bloques":[{"paquete":"paquete","posicion":0,"texto":paquete["texto_memoria"]}]}}});
        s.capture(&q.to_string()).unwrap();
        s.record_execution(&Record::Enlazar {
            intento: "a".into(),
            evento: llamada.into(),
        })
        .unwrap();
        s.record_execution(&Record::Exponer {
            clave: format!("x/{llamada}"),
            intento: "a".into(),
            solicitud: llamada.into(),
            paquete: "paquete".into(),
            posicion: 0,
        })
        .unwrap();
    }
    drop(s);
    let db = db.to_str().unwrap();
    for (filtros, cantidad) in [
        (vec![], 2),
        (vec!["--call", "llamada"], 1),
        (vec!["--packet", "paquete"], 2),
        (vec!["--call", "llamada", "--packet", "paquete"], 1),
        (vec!["--call", "ausente"], 0),
    ] {
        let mut args = vec![db, "exposures", "p"];
        args.extend(filtros);
        assert_eq!(
            correcto(d.path(), &args)[0]["exposiciones"]
                .as_array()
                .unwrap()
                .len(),
            cantidad
        );
    }
    assert_eq!(
        correcto(d.path(), &[db, "unexposed", "p"])[0]["paquetes"],
        json!([])
    );
    assert_eq!(
        correcto(d.path(), &[db, "attempt", "p", "a"])[0]["ejecucion"]["intento_clave"],
        "a"
    );
    for args in [
        vec![db, "exposures", "p", "--call"],
        vec![db, "exposures", "p", "--desconocido", "x"],
    ] {
        assert_eq!(cli(d.path(), &args).status.code(), Some(1));
    }
}

#[test]
fn importar_crlf_y_limitar_el_trabajo_conserva_el_resto_pendiente() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("db.sqlite");
    let fuente = d.path().join("fuente.jsonl");
    std::fs::write(
        &fuente,
        format!(
            "{}\r\n{}\r\n{}\r\n",
            evento("a", 1, "problema"),
            evento("b", 2, "solucion"),
            evento("c", 3, "otro")
        ),
    )
    .unwrap();
    let db = db.to_str().unwrap();
    assert_eq!(
        correcto(d.path(), &[db, "import", fuente.to_str().unwrap()]).len(),
        3
    );
    let trabajo = correcto(
        d.path(),
        &[db, "work", "--limit", "1", "--lease-ms", "60000"],
    );
    assert_eq!(trabajo.last().unwrap()["completados_en_ejecucion"], 1);
    let mut s = Store::open(db).unwrap();
    let episodio = s.episode("g").unwrap();
    let filas = episodio["eventos"].as_array().unwrap();
    assert_eq!(filas[0]["trabajos"][0]["estado"], "completado");
    assert_eq!(filas[1]["trabajos"][0]["estado"], "pendiente");
    assert_eq!(filas[2]["trabajos"][0]["estado"], "pendiente");
    std::fs::write(
        &fuente,
        format!(
            "{}\r\n{}\r\n{{invalido}}\r\n",
            evento("a", 1, "problema"),
            evento("b", 2, "solucion")
        ),
    )
    .unwrap();
    let salida = cli(d.path(), &[db, "import", fuente.to_str().unwrap()]);
    assert_eq!(salida.status.code(), Some(1));
    assert!(String::from_utf8(salida.stderr)
        .unwrap()
        .contains("linea 3; no confirmada"));
    assert_eq!(String::from_utf8(salida.stdout).unwrap().lines().count(), 2);
}

#[test]
fn bandera_sin_valor_puede_ir_antes_de_opciones_con_valor() {
    let d = tempfile::tempdir().unwrap();
    let proyecto = d.path().join("proyecto");
    std::fs::create_dir(&proyecto).unwrap();
    correcto(
        d.path(),
        &[
            "--control",
            "proyecto-registrar",
            "--raiz",
            proyecto.to_str().unwrap(),
        ],
    );
    let args = [
        "--control",
        "proyecto-predeterminado",
        "--quitar",
        "--raiz",
        proyecto.to_str().unwrap(),
    ];
    let salida = correcto(d.path(), &args);
    assert_eq!(salida[0]["memoria_predeterminada"], Value::Null);
}

// Regresión: `--control` aceptaba cualquier `--clave valor`. Una opción mal escrita caía
// a su valor por omisión: `--modoo solo_registro` encendía la sesión en `memoria_activa`.
#[test]
fn regresion_control_rechaza_una_opcion_mal_escrita_sin_tocar_nada() {
    let d = tempfile::tempdir().unwrap();
    let raiz = d.path();
    let cwd = raiz.to_str().unwrap();
    let memoria = correcto(raiz, &["--control", "memoria-crear", "--nombre", "m"])[0]["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    correcto(raiz, &["--control", "proyecto-registrar", "--raiz", cwd]);
    correcto(
        raiz,
        &["--control", "sesion-iniciar", "--sesion", "s", "--cwd", cwd],
    );
    let estado = || correcto(raiz, &["--control", "sesion-estado", "--sesion", "s"]);
    let antes = estado();
    for mala in [
        vec![
            "--control",
            "sesion-usar",
            "--sesion",
            "s",
            "--cwd",
            cwd,
            "--memoria",
            &memoria,
            "--modoo",
            "solo_registro",
        ],
        vec![
            "--control",
            "sesion-usar",
            "--sesion",
            "s",
            "--cwd",
            cwd,
            "--memoria",
            &memoria,
            "--Modo",
            "solo_registro",
        ],
        vec![
            "--control",
            "canal-publicar",
            "--sesion",
            "s",
            "--canal",
            "general",
            "--texto",
            "hola",
            "--audienca",
            "uno",
        ],
        vec!["--control", "memoria-listar", "--raiz-dato", cwd],
        vec!["--control", "memoria-listar", "--instalación", "otra"],
    ] {
        let salida = cli(raiz, &mala);
        assert!(
            !salida.status.success() && salida.stdout.is_empty(),
            "{mala:?}"
        );
        assert!(
            String::from_utf8_lossy(&salida.stderr).contains("opcion desconocida"),
            "{mala:?}"
        );
    }
    assert_eq!(estado(), antes);
    let historial = correcto(
        raiz,
        &[
            "--control",
            "canal-historial",
            "--sesion",
            "s",
            "--canal",
            "general",
        ],
    );
    assert_eq!(historial[0]["mensajes"], 0, "{historial:?}");
    // Bien escrita, la misma orden sí tiene efecto.
    correcto(
        raiz,
        &[
            "--control",
            "sesion-usar",
            "--sesion",
            "s",
            "--cwd",
            cwd,
            "--memoria",
            &memoria,
            "--modo",
            "solo_registro",
        ],
    );
    assert_ne!(estado(), antes);
}
