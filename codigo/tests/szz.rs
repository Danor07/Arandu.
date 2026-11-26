//! Entrega 7: SZZ. Los repos git son reales y los crea cada prueba en un temporal.
use msp::control::Registry;
use msp::szz::{hashes_de_captura, parsear_blame, rangos_viejos, SZZ_VERSION};
use msp::Store;
use proptest::prelude::*;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

const OBJETIVO: &str = "Compilar módulo";

/// git determinista: sin configuración del usuario ni del sistema y con fechas fijas.
fn git(repo: &Path, args: &[&str]) -> String {
    let salida = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=prueba",
            "-c",
            "user.email=prueba@example.invalid",
        ])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        salida.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&salida.stderr)
    );
    String::from_utf8(salida.stdout).unwrap().trim().to_string()
}

/// Escribe `archivo`, lo commitea y devuelve (hash, salida de `git commit`).
fn commit(repo: &Path, archivo: &str, contenido: &str, mensaje: &str) -> (String, String) {
    std::fs::write(repo.join(archivo), contenido).unwrap();
    git(repo, &["add", "--", archivo]);
    let salida = git(repo, &["commit", "-m", mensaje]);
    (git(repo, &["rev-parse", "HEAD"]), salida)
}

fn repo() -> (tempfile::TempDir, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let ruta = d.path().join("repo con espacios");
    std::fs::create_dir(&ruta).unwrap();
    git(&ruta, &["init", "-q"]);
    (d, ruta)
}

fn capturar(s: &mut Store, chat: &str, n: i64, fecha: i64, payload: Value) -> i64 {
    s.capture(&json!({"version":1,"clave_evento":format!("{chat}/{n}"),"grupo_evidencia":chat,"secuencia":n,
        "timestamp":fecha,"origen":"herramienta","rol":"otro","contenido":"captura de herramienta",
        "metadatos":{"proyecto":"p","intervalo_captura":7,"payload_hook":payload}}).to_string()).unwrap().instancia_id
}

/// Episodio automático exitoso con mínima: edita, commitea (con la salida real de git) y comprueba.
fn episodio(
    s: &mut Store,
    chat: &str,
    herramienta: &str,
    salida_commit: Option<&str>,
    fecha: i64,
) -> Value {
    let lista = |estado: &str| {
        json!({"hook_event_name":"PostToolUse","tool_name":"TodoWrite",
        "tool_input":{"todos":[{"content":OBJETIVO,"status":estado,"activeForm":"compilando"}]}})
    };
    let bash = |id: i64, comando: &str, stdout: &str| {
        json!({"hook_event_name":"PostToolUse","tool_name":"Bash","tool_use_id":format!("{chat}/{id}"),"cwd":"/p",
        "tool_input":{"command":comando},"tool_response":{"stdout":stdout,"stderr":"","interrupted":false}})
    };
    capturar(s, chat, 1, fecha, lista("in_progress"));
    capturar(
        s,
        chat,
        2,
        fecha,
        json!({"hook_event_name":"PostToolUse","tool_name":herramienta,"cwd":"/p","tool_use_id":format!("{chat}/2"),
        "tool_input":{"file_path":"/p/lib.rs","content":"a"}}),
    );
    if let Some(salida) = salida_commit {
        capturar(s, chat, 3, fecha, bash(3, "git commit -m cambio", salida));
    }
    capturar(s, chat, 4, fecha, bash(4, "cargo test", "salida"));
    capturar(s, chat, 5, fecha, lista("completed"));
    let ep = s.procesar_episodios_automaticos("p", chat, &[]).unwrap()["episodios"][0].clone();
    s.calcular_secuencia_minima("p", "automatico_lista", &ep["id"].to_string())
        .unwrap();
    ep
}

/// Repo con un commit base, uno «malo» hecho por un episodio y un arreglo posterior.
struct Caso {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    store: Store,
    base: String,
    malo: String,
    arreglo: String,
    episodio: Value,
}

fn caso() -> Caso {
    let (dir, repo) = repo();
    let (base, _) = commit(&repo, "lib.rs", "uno\ndos\ntres\n", "base");
    let (malo, salida) = commit(&repo, "lib.rs", "uno\nDOS ROTO\ntres\n", "cambio con error");
    let mut store = Store::open(dir.path().join("db")).unwrap();
    let episodio = episodio(&mut store, "chat-malo", "Write", Some(&salida), 10);
    let (arreglo, _) = commit(
        &repo,
        "lib.rs",
        "uno\ndos arreglado\ntres\ncuatro\n",
        "arreglo",
    );
    Caso {
        _dir: dir,
        repo,
        store,
        base,
        malo,
        arreglo,
        episodio,
    }
}

fn camino(s: &mut Store) -> Value {
    s.consultar_vigente("p", OBJETIVO).unwrap()["vigente"]["camino"].clone()
}

#[test]
fn un_arreglo_senala_al_episodio_y_solo_el_vinculo_confirmado_resta_evidencia() {
    let mut c = caso();
    assert_eq!(camino(&mut c.store)["fallos"], 0);
    let r = c.store.szz_rastrear("p", &c.repo, "HEAD").unwrap();
    assert_eq!(r["regla"], SZZ_VERSION);
    assert_eq!(r["arreglo"], c.arreglo);
    assert_eq!(r["senalados_sin_episodio"], json!([]));
    assert_eq!(r["archivos_omitidos"], json!([]));
    let vinculos = r["vinculos"].as_array().unwrap();
    assert_eq!(vinculos.len(), 1);
    let v = &vinculos[0];
    assert_eq!(v["senalado_commit"], c.malo);
    assert_eq!(v["estado"], "propuesto");
    assert_eq!(
        (v["origen"].as_str(), &v["episodio_id"]),
        (Some("automatico_lista"), &c.episodio["id"])
    );
    // Sólo la línea reemplazada; la agregada al final no señala a nadie.
    assert_eq!(v["archivos_json"], r#"[{"archivo":"lib.rs","lineas":1}]"#);
    // Proponer no resta nada, y repetir el rastreo no duplica.
    assert_eq!(camino(&mut c.store)["fallos"], 0);
    assert_eq!(c.store.szz_rastrear("p", &c.repo, &c.arreglo).unwrap(), r);
    assert_eq!(c.store.szz_listar("p").unwrap()["vinculos"], r["vinculos"]);

    let id = v["id"].as_i64().unwrap();
    let pregunta_antes = problema(&mut c.store, "antes");
    let confirmado = c.store.szz_decidir("p", id, "confirmado").unwrap();
    assert_eq!(confirmado["estado"], "confirmado");
    let consulta = c.store.consultar_vigente("p", OBJETIVO).unwrap();
    let vigente = &consulta["vigente"]["camino"];
    assert_eq!(
        (&vigente["exitos"], &vigente["fallos"], &vigente["neto"]),
        (&json!(1), &json!(1), &json!(0))
    );
    assert_eq!(consulta["evidencia_tipo"]["fallos_tardios"], 1);
    assert_eq!(consulta["evidencia_tipo"]["fallos_atribuidos"], 1);
    let tardio = consulta["observaciones"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["tipo_registro"] == "fallo_tardio_szz")
        .unwrap();
    assert_eq!(tardio["arreglo_commit"], c.arreglo);
    assert_eq!(tardio["fecha"], confirmado["decidido_en"]);
    // Una decisión es definitiva y el rastreo posterior la conserva.
    assert!(c.store.szz_decidir("p", id, "descartado").is_err());
    assert!(c.store.szz_decidir("p", id, "confirmado").is_err());
    let otra_vez = c.store.szz_rastrear("p", &c.repo, &c.arreglo).unwrap();
    assert_eq!(otra_vez["vinculos"][0]["estado"], "confirmado");
    assert_eq!(camino(&mut c.store)["fallos"], 1);

    // Corte histórico: un pedido anterior a la confirmación no ve el fallo tardío.
    let antes = c
        .store
        .prepare_memory("antes", &peticion(pregunta_antes))
        .unwrap();
    assert_eq!(antes["vigente"]["vigente"]["camino"]["fallos"], 0);
    let pregunta_despues = problema(&mut c.store, "despues");
    let despues = c
        .store
        .prepare_memory("despues", &peticion(pregunta_despues))
        .unwrap();
    assert_eq!(despues["vigente"]["vigente"]["camino"]["fallos"], 1);
    assert_eq!(c.store.memory_packet("p", "antes").unwrap(), antes);
}

fn problema(s: &mut Store, chat: &str) -> i64 {
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    std::thread::sleep(std::time::Duration::from_millis(2));
    s.capture(&json!({"version":1,"clave_evento":format!("pregunta/{chat}"),"grupo_evidencia":chat,"secuencia":100,
        "timestamp":ahora,"origen":"usuario","rol":"problema","contenido":OBJETIVO,"metadatos":{"proyecto":"p"}}).to_string()).unwrap().instancia_id
}

fn peticion(id: i64) -> msp::retrieval::PrepareRequest {
    msp::retrieval::PrepareRequest {
        problema_id: id,
        busqueda: msp::retrieval::SearchRequest {
            proyecto: "p".into(),
            consulta: "compilar módulo".into(),
            origen: None,
            rol: None,
            limite: 100,
        },
        presupuesto_bytes: 10000,
    }
}

#[test]
fn un_vinculo_descartado_no_resta_y_no_vuelve_como_propuesto() {
    let mut c = caso();
    let id = c.store.szz_rastrear("p", &c.repo, &c.arreglo).unwrap()["vinculos"][0]["id"]
        .as_i64()
        .unwrap();
    assert_eq!(
        c.store.szz_decidir("p", id, "descartado").unwrap()["estado"],
        "descartado"
    );
    assert_eq!(camino(&mut c.store)["fallos"], 0);
    let r = c.store.szz_rastrear("p", &c.repo, &c.arreglo).unwrap();
    assert_eq!(r["vinculos"].as_array().unwrap().len(), 1);
    assert_eq!(r["vinculos"][0]["estado"], "descartado");
    // Decisiones inválidas, vínculo inexistente o de otro proyecto.
    assert!(c.store.szz_decidir("p", id + 1, "confirmado").is_err());
    assert!(c.store.szz_decidir("otro", id, "confirmado").is_err());
    assert!(c.store.szz_decidir("p", id, "quizas").is_err());
    assert!(c.store.szz_listar("").is_err());
    assert_eq!(c.store.szz_listar("otro").unwrap()["vinculos"], json!([]));
}

#[test]
fn sin_rastro_del_commit_en_una_captura_no_hay_vinculo() {
    // El commit señalado es el base, que ningún episodio produjo.
    let (dir, repo) = repo();
    let (base, _) = commit(&repo, "a.txt", "x\n", "base");
    let (arreglo, _) = commit(&repo, "a.txt", "y\n", "arreglo");
    let mut s = Store::open(dir.path().join("db")).unwrap();
    // Un episodio que menciona el hash fuera de un `git commit`, o en otro proyecto, no cuenta.
    let corto = &base[..7];
    capturar(
        &mut s,
        "ruido",
        1,
        1,
        json!({"tool_name":"Bash","tool_input":{"command":"git log"},"tool_response":{"stdout":format!("[main {corto}] base")}}),
    );
    capturar(
        &mut s,
        "ruido",
        2,
        1,
        json!({"tool_name":"Read","tool_input":{"command":"git commit"},"tool_response":{"stdout":format!("[main {corto}] base")}}),
    );
    let r = s.szz_rastrear("p", &repo, &arreglo).unwrap();
    assert_eq!(r["vinculos"], json!([]));
    assert_eq!(r["senalados_sin_episodio"], json!([base]));
    assert_eq!(s.szz_listar("p").unwrap()["vinculos"], json!([]));
}

#[test]
fn el_mismo_commit_en_dos_episodios_propone_los_dos_y_cada_arreglo_cuenta_una_vez() {
    let (dir, repo) = repo();
    commit(&repo, "a.txt", "1\n2\n", "base");
    let (malo_a, salida_a) = commit(&repo, "a.txt", "1 mal\n2\n", "error en la uno");
    let (malo_b, salida_b) = commit(&repo, "a.txt", "1 mal\n2 mal\n", "error en la dos");
    let mut s = Store::open(dir.path().join("db")).unwrap();
    // El primer episodio hizo los dos commits; el segundo repite la salida del primero.
    let e1 = episodio(
        &mut s,
        "uno",
        "Write",
        Some(&format!("{salida_a}\n{salida_b}")),
        10,
    );
    let e2 = episodio(&mut s, "dos", "Write", Some(&salida_a), 20);
    let (arreglo, _) = commit(&repo, "a.txt", "1 bien\n2 bien\n", "arreglo de las dos");
    let r = s.szz_rastrear("p", &repo, &arreglo).unwrap();
    let pares: Vec<(String, i64)> = r["vinculos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["senalado_commit"].as_str().unwrap().to_string(),
                v["episodio_id"].as_i64().unwrap(),
            )
        })
        .collect();
    let (id1, id2) = (e1["id"].as_i64().unwrap(), e2["id"].as_i64().unwrap());
    let mut esperados = vec![(malo_a.clone(), id1), (malo_a, id2), (malo_b, id1)];
    esperados.sort();
    let mut vistos = pares.clone();
    vistos.sort();
    assert_eq!(vistos, esperados);
    // Confirmar los dos vínculos del primer episodio para el mismo arreglo es un solo fallo.
    for v in r["vinculos"].as_array().unwrap() {
        if v["episodio_id"] == id1 {
            s.szz_decidir("p", v["id"].as_i64().unwrap(), "confirmado")
                .unwrap();
        }
    }
    let consulta = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(consulta["evidencia_tipo"]["fallos_tardios"], 1);
    assert_eq!(consulta["vigente"]["camino"]["fallos"], 1);
    assert_eq!(consulta["vigente"]["camino"]["exitos"], 2);
}

#[test]
fn arreglos_que_no_se_pueden_rastrear_fallan_sin_escribir_nada() {
    let mut c = caso();
    let nada = |s: &mut Store| assert_eq!(s.szz_listar("p").unwrap()["vinculos"], json!([]));
    // Commit raíz: sin padre.
    let error = c
        .store
        .szz_rastrear("p", &c.repo, &c.base)
        .unwrap_err()
        .to_string();
    assert!(error.contains("0 padres"), "{error}");
    // Merge: dos padres.
    git(&c.repo, &["checkout", "-q", "-b", "rama", &c.base]);
    commit(&c.repo, "otro.txt", "z\n", "en la rama");
    git(&c.repo, &["checkout", "-q", "main"]);
    git(&c.repo, &["merge", "-q", "--no-ff", "-m", "merge", "rama"]);
    let error = c
        .store
        .szz_rastrear("p", &c.repo, "HEAD")
        .unwrap_err()
        .to_string();
    assert!(error.contains("2 padres"), "{error}");
    // Revisiones que no existen, vacías, con espacios, o que git tomaría por una opción.
    let salida = c.repo.join("inyectado");
    for malo in [
        "no-existe".to_string(),
        String::new(),
        "HEAD --output=x".to_string(),
        format!("--output={}", salida.display()),
        "-h".to_string(),
        "--all".to_string(),
        "HEAD;touch inyectado".to_string(),
        "$(touch inyectado)".to_string(),
    ] {
        assert!(
            c.store.szz_rastrear("p", &c.repo, &malo).is_err(),
            "{malo:?}"
        );
    }
    assert!(!salida.exists() && !c.repo.join("x").exists());
    // Un árbol o un blob no son un commit.
    let arbol = git(&c.repo, &["rev-parse", "HEAD^{tree}"]);
    assert!(c.store.szz_rastrear("p", &c.repo, &arbol).is_err());
    // Rutas que no son un repo, y proyecto vacío.
    for ruta in [
        c.repo.join("no-existe"),
        c._dir.path().to_path_buf(),
        PathBuf::from("--help"),
    ] {
        assert!(
            c.store.szz_rastrear("p", &ruta, "HEAD").is_err(),
            "{ruta:?}"
        );
    }
    assert!(c.store.szz_rastrear("", &c.repo, &c.arreglo).is_err());
    nada(&mut c.store);
}

#[test]
fn archivos_agregados_borrados_renombrados_y_binarios_no_senalan_a_nadie() {
    let (dir, repo) = repo();
    commit(&repo, "viejo.txt", "a\nb\n", "base");
    std::fs::write(repo.join("bin.dat"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    git(&repo, &["add", "bin.dat"]);
    git(&repo, &["commit", "-q", "-m", "binario"]);
    // El arreglo renombra un archivo, agrega otro, cambia el binario y borra nada más.
    git(&repo, &["mv", "viejo.txt", "nuevo nombre.txt"]);
    std::fs::write(repo.join("agregado.txt"), "nuevo\n").unwrap();
    std::fs::write(repo.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "arreglo"]);
    let mut s = Store::open(dir.path().join("db")).unwrap();
    let r = s.szz_rastrear("p", &repo, "HEAD").unwrap();
    assert_eq!(r["vinculos"], json!([]));
    assert_eq!(r["senalados_sin_episodio"], json!([]));
}

#[test]
fn rutas_con_espacios_comillas_y_unicode_se_rastrean_como_datos() {
    let (dir, repo) = repo();
    let archivo = "carpeta rara/ñandú \"citado\" 'simple'.txt";
    std::fs::create_dir(repo.join("carpeta rara")).unwrap();
    commit(&repo, archivo, "uno\n", "base");
    let (malo, salida) = commit(
        &repo,
        archivo,
        "uno\n@@ -1,5 +1,5 @@ falso\n",
        "con una línea que parece encabezado",
    );
    let mut s = Store::open(dir.path().join("db")).unwrap();
    episodio(&mut s, "chat", "Write", Some(&salida), 10);
    let (arreglo, _) = commit(&repo, archivo, "uno\narreglado\n", "arreglo");
    let r = s.szz_rastrear("p", &repo, &arreglo).unwrap();
    assert_eq!(r["vinculos"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["vinculos"][0]["senalado_commit"], malo);
    let archivos: Value =
        serde_json::from_str(r["vinculos"][0]["archivos_json"].as_str().unwrap()).unwrap();
    assert_eq!(archivos, json!([{"archivo":archivo,"lineas":1}]));
}

#[test]
fn el_esquema_es_append_only_y_valida_la_captura() {
    let mut c = caso();
    let r = c.store.szz_rastrear("p", &c.repo, &c.arreglo).unwrap();
    let id = r["vinculos"][0]["id"].as_i64().unwrap();
    c.store.szz_decidir("p", id, "confirmado").unwrap();
    let ruta = c._dir.path().join("db");
    drop(c.store);
    let conn = Connection::open(ruta).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    for sql in [
        "UPDATE szz_vinculo SET proyecto='q'",
        "DELETE FROM szz_vinculo",
        "UPDATE szz_decision SET decision='descartado'",
        "DELETE FROM szz_decision",
        "INSERT OR REPLACE INTO szz_decision VALUES (1,'descartado',1)",
        "INSERT OR REPLACE INTO szz_vinculo SELECT id,proyecto,arreglo_commit,senalado_commit,'[]',instancia_id,origen,episodio_id,1 FROM szz_vinculo",
        "INSERT INTO szz_decision VALUES (99,'confirmado',1)",
        "INSERT INTO szz_decision VALUES (1,'quizas',1)",
        // Captura que no es de ese episodio, o proyecto ajeno.
        "INSERT INTO szz_vinculo(proyecto,arreglo_commit,senalado_commit,archivos_json,instancia_id,origen,episodio_id,registrado_en)
         SELECT 'p',arreglo_commit,senalado_commit,'[]',instancia_id,origen,episodio_id+7,1 FROM szz_vinculo",
        "INSERT INTO szz_vinculo(proyecto,arreglo_commit,senalado_commit,archivos_json,instancia_id,origen,episodio_id,registrado_en)
         SELECT 'otro',arreglo_commit,senalado_commit,'[]',instancia_id,origen,episodio_id,1 FROM szz_vinculo",
        "INSERT INTO szz_vinculo(proyecto,arreglo_commit,senalado_commit,archivos_json,instancia_id,origen,episodio_id,registrado_en)
         SELECT 'p',arreglo_commit,arreglo_commit,'[]',instancia_id,origen,episodio_id,1 FROM szz_vinculo",
    ] {
        assert!(conn.execute(sql, []).is_err(), "{sql}");
    }
    let filas: (i64, String) = conn
        .query_row(
            "SELECT count(*),(SELECT decision FROM szz_decision) FROM szz_vinculo",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(filas, (1, "confirmado".into()));
}

#[test]
fn cli_rastrea_lista_confirma_y_descarta_en_procesos_separados() {
    let (dir, repo) = repo();
    commit(&repo, "lib.rs", "uno\n", "base");
    let (malo, salida) = commit(&repo, "lib.rs", "uno roto\n", "error");
    let (arreglo, _) = commit(&repo, "lib.rs", "uno bien\n", "arreglo");
    let datos = dir.path().join("datos");
    let mut r = Registry::open(&datos).unwrap();
    let m = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut s = r.open_memory(&m).unwrap();
    episodio(&mut s, "chat", "Write", Some(&salida), 10);
    drop(s);
    let corre = |orden: &str, extras: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_msp"))
            .args(["--control", orden, "--memoria", &m, "--proyecto", "p"])
            .args(extras)
            .env("MSP_RAIZ", &datos)
            .output()
            .unwrap()
    };
    let json_de = |salida: std::process::Output| -> Value {
        assert!(
            salida.status.success(),
            "{}",
            String::from_utf8_lossy(&salida.stderr)
        );
        serde_json::from_slice(&salida.stdout).unwrap()
    };
    let ruta = repo.to_str().unwrap();
    let rastreo = json_de(corre(
        "szz-rastrear",
        &["--repo", ruta, "--arreglo", "HEAD"],
    ));
    assert_eq!(rastreo["arreglo"], arreglo);
    assert_eq!(rastreo["vinculos"][0]["senalado_commit"], malo);
    assert_eq!(rastreo["vinculos"][0]["estado"], "propuesto");
    let id = rastreo["vinculos"][0]["id"].to_string();
    assert_eq!(
        json_de(corre("szz-listar", &[]))["vinculos"],
        rastreo["vinculos"]
    );
    assert_eq!(
        json_de(corre("szz-confirmar", &["--vinculo", &id]))["estado"],
        "confirmado"
    );
    // Ya decidido, vínculo inexistente, número inválido, y un arreglo que git tomaría por opción.
    for (orden, extras) in [
        ("szz-descartar", vec!["--vinculo", id.as_str()]),
        ("szz-confirmar", vec!["--vinculo", "999"]),
        ("szz-confirmar", vec!["--vinculo", "uno"]),
        ("szz-rastrear", vec!["--repo", ruta, "--arreglo", "-h"]),
        ("szz-rastrear", vec!["--repo", ruta]),
    ] {
        let salida = corre(orden, &extras);
        assert!(
            !salida.status.success() && salida.stdout.is_empty(),
            "{orden} {extras:?}"
        );
    }
    let consulta = json_de(corre("vigente-consultar", &["--objetivo", OBJETIVO]));
    assert_eq!(consulta["evidencia_tipo"]["fallos_tardios"], 1);
    assert_eq!(consulta["vigente"]["camino"]["fallos"], 1);
    assert_eq!(
        json_de(corre("szz-listar", &[]))["vinculos"][0]["estado"],
        "confirmado"
    );
}

#[test]
fn los_parsers_leen_solo_los_encabezados_y_los_corchetes_de_un_git_commit() {
    assert_eq!(rangos_viejos(b"@@ -3 +3 @@\n@@ -7,2 +8,0 @@ ctx\n@@ -0,0 +1 @@\n@@ -0,2 +1 @@\n@@ -5,0 +6,2 @@\n-@@ -9,9 +1 @@\n @@ -9 +1 @@\n@@ -x,1 +1 @@\n@@ -"), [(3, 1), (7, 2)]);
    let h = "a".repeat(40);
    let h64 = "B".repeat(64);
    let salida = format!(
        "{h} 1 1 2\nauthor x\nsummary {h} 9 9\nfilename a\n\t{h} 5 5 contenido que imita un registro\n{h} 2 2\nfilename a\n\tdos\n{h64} 3 3 1\nfilename a\n\t\nabc 1 1\n{h} uno dos\n{h}\n"
    );
    let lineas = parsear_blame(salida.as_bytes());
    assert_eq!(lineas, [(h.clone(), 2), (h64.to_lowercase(), 1)].into());
    assert!(parsear_blame(b"").is_empty() && parsear_blame(&[0xff, b'\n', 0xfe]).is_empty());

    let captura = |nombre: &str, comando: &str, respuesta: Value| {
        hashes_de_captura(
            &json!({"tool_name":nombre,"tool_input":{"command":comando},"tool_response":respuesta}),
        )
    };
    let stdout = |s: &str| json!({"stdout":s,"stderr":""});
    assert_eq!(
        captura(
            "Bash",
            "git commit -m x",
            stdout("[main abc1234] x\n 1 file changed")
        ),
        ["abc1234"]
    );
    assert_eq!(
        captura(
            "Bash",
            "git add . && git commit -m x",
            json!("[main (root-commit) ABCDEF12] x")
        ),
        ["abcdef12"]
    );
    // Otra herramienta, otro comando, prefijo corto, no hexadecimal, sin corchetes.
    assert!(captura("Read", "git commit", stdout("[main abc1234] x")).is_empty());
    assert!(captura("Bash", "git log", stdout("[main abc1234] x")).is_empty());
    assert!(captura("Bash", "git commit", stdout("[main abc123] x")).is_empty());
    assert!(captura("Bash", "git commit", stdout("[main abc123g] x")).is_empty());
    assert!(captura("Bash", "git commit", stdout("abc1234 sin corchetes")).is_empty());
    assert!(captura("Bash", "git commit", stdout("[] [ ] [abc1234")).len() <= 1);
    assert!(hashes_de_captura(&Value::Null).is_empty());
}

proptest! {
    /// El contenido de un archivo nunca cuenta como registro de blame: toda línea de
    /// contenido empieza con tabulador, diga lo que diga después.
    #[test]
    fn el_blame_cuenta_registros_y_no_contenido(
        registros in prop::collection::vec((0usize..3, ".*"), 0..12),
    ) {
        let hashes = ["a".repeat(40), "b".repeat(40), "c".repeat(64)];
        let mut salida = String::new();
        let mut esperado = std::collections::BTreeMap::new();
        for (n, (cual, contenido)) in registros.iter().enumerate() {
            let contenido = contenido.replace('\n', " ");
            salida += &format!("{} {} {} 1\nauthor {}\nsummary {}\nfilename x\n\t{}\n", hashes[*cual], n + 1, n + 1, contenido, contenido, contenido);
            *esperado.entry(hashes[*cual].clone()).or_insert(0usize) += 1;
        }
        prop_assert_eq!(parsear_blame(salida.as_bytes()), esperado);
    }

    /// Bytes arbitrarios no hacen fallar los parsers, y un rango nunca empieza en cero ni es vacío.
    #[test]
    fn los_parsers_aceptan_cualquier_entrada(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        prop_assert!(rangos_viejos(&bytes).iter().all(|(inicio, cantidad)| *inicio > 0 && *cantidad > 0));
        let blame = parsear_blame(&bytes);
        prop_assert!(blame.keys().all(|h| [40, 64].contains(&h.len())) && blame.values().all(|n| *n > 0));
    }

    /// De una captura sólo salen prefijos hexadecimales de 7 a 64, en minúsculas, y sólo de un `git commit` por Bash.
    #[test]
    fn los_hashes_de_una_captura_son_prefijos_hexadecimales(
        nombre in "Bash|Read|bash|",
        comando in "(git commit|git log|ls)? ?.{0,10}",
        respuesta in ".{0,60}",
        hash in "[0-9a-fA-F]{7,64}",
    ) {
        let payload = |r: &str| json!({"tool_name":nombre,"tool_input":{"command":comando},"tool_response":{"stdout":r}});
        let hashes = hashes_de_captura(&payload(&respuesta));
        prop_assert!(hashes.iter().all(|h| (7..=64).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())));
        let es_commit = nombre == "Bash" && comando.contains("git commit");
        prop_assert!(es_commit || hashes.is_empty());
        let con_hash = hashes_de_captura(&payload(&format!("{respuesta}[rama {hash}] mensaje")));
        prop_assert_eq!(con_hash.contains(&hash.to_lowercase()), es_commit);
    }
}
