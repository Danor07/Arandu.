//! Predeterminado de proyecto con modo, sobre registros y memorias temporales.
use msp::adapter::Adapter;
use msp::control::{normalizar_raiz, Registry};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

#[test]
fn hereda_modo_en_sesion_intervalo_e_historial_y_reabrir_no_lo_cambia() {
    for modo in ["solo_registro", "memoria_activa"] {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::open(d.path()).unwrap();
        r.register_project(d.path()).unwrap();
        let m = r.create_memory("Prueba").unwrap()["memoria_id"]
            .as_str()
            .unwrap()
            .to_string();
        let fijado = r.set_default_with_mode(d.path(), Some(&m), modo).unwrap();
        assert_eq!(fijado["modo_predeterminado"], modo);
        let nueva = r.init_session("i", "s", d.path()).unwrap();
        assert_eq!(nueva["estado"], "encendida");
        assert_eq!(nueva["modo"], modo);
        assert_eq!(nueva["memoria"]["memoria_id"], m);
        assert_eq!(nueva["origen_seleccion"], "predeterminado_proyecto");
        assert_eq!(
            r.gate("i", "s", Some(d.path())).unwrap().unwrap().modo,
            modo
        );
        let c = Connection::open(d.path().join("registro.sqlite")).unwrap();
        assert_eq!(
            c.query_row("SELECT modo FROM intervalo_captura", [], |fila| fila
                .get::<_, String>(0))
                .unwrap(),
            modo
        );
        assert_eq!(
            c.query_row("SELECT recuperacion FROM sesion_memoria", [], |fila| fila
                .get::<_, i64>(
                0
            ))
            .unwrap(),
            i64::from(modo == "memoria_activa")
        );
        for accion in [
            "predeterminado_fijado",
            "sesion_inicializada",
            "memoria_encendida",
        ] {
            let detalle: String = c
                .query_row(
                    "SELECT detalle_json FROM control_historial WHERE accion=?1",
                    [accion],
                    |fila| fila.get(0),
                )
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&detalle).unwrap()["modo"],
                modo
            );
        }
        drop(r);
        let mut r = Registry::open(d.path()).unwrap();
        assert_eq!(r.init_session("i", "s", d.path()).unwrap()["modo"], modo);
        assert_eq!(r.init_session("i", "otra", d.path()).unwrap()["modo"], modo);
    }
}

#[test]
fn cambiar_o_quitar_predeterminado_no_altera_sesiones_y_sin_modo_hereda_activa() {
    let d = tempfile::tempdir().unwrap();
    let mut r = Registry::open(d.path()).unwrap();
    r.register_project(d.path()).unwrap();
    let m = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    r.set_default_with_mode(d.path(), Some(&m), "solo_registro")
        .unwrap();
    r.init_session("i", "vieja", d.path()).unwrap();
    let antes = r.status("i", "vieja", Some(d.path())).unwrap();
    let cambio = r.set_default(d.path(), Some(&m)).unwrap();
    assert_eq!(cambio["modo_predeterminado"], "memoria_activa");
    assert_eq!(cambio["sesiones_existentes_sin_cambios"], 1);
    assert_eq!(r.status("i", "vieja", Some(d.path())).unwrap(), antes);
    assert_eq!(
        r.init_session("i", "nueva", d.path()).unwrap()["modo"],
        "memoria_activa"
    );
    assert_eq!(
        r.init_session("i", "vieja", d.path()).unwrap()["modo"],
        "solo_registro"
    );
    r.turn_off("i", "vieja").unwrap();
    r.set_default_with_mode(d.path(), Some(&m), "solo_registro")
        .unwrap();
    assert_eq!(
        r.init_session("i", "vieja", d.path()).unwrap()["modo"],
        "apagada"
    );
    assert_eq!(
        r.status("i", "nueva", None).unwrap()["modo"],
        "memoria_activa"
    );
    r.set_default(d.path(), None).unwrap();
    assert_eq!(
        r.init_session("i", "sin", d.path()).unwrap()["modo"],
        "apagada"
    );
    let c = Connection::open(d.path().join("registro.sqlite")).unwrap();
    let detalle:String=c.query_row("SELECT detalle_json FROM control_historial WHERE accion='sesion_inicializada' ORDER BY id DESC LIMIT 1",[],|fila|fila.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&detalle).unwrap()["modo"],
        "apagada"
    );
}

#[test]
fn modo_invalido_no_cambia_el_predeterminado_ni_crea_historial() {
    let d = tempfile::tempdir().unwrap();
    let mut r = Registry::open(d.path()).unwrap();
    r.register_project(d.path()).unwrap();
    let m = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    r.set_default(d.path(), Some(&m)).unwrap();
    let c = Connection::open(d.path().join("registro.sqlite")).unwrap();
    let antes: i64 = c
        .query_row("SELECT count(*) FROM control_historial", [], |fila| {
            fila.get(0)
        })
        .unwrap();
    for invalido in ["", "apagada", "SOLO_REGISTRO", "solo_registro extra"] {
        assert!(r
            .set_default_with_mode(d.path(), Some(&m), invalido)
            .is_err());
    }
    assert_eq!(
        c.query_row("SELECT count(*) FROM control_historial", [], |fila| fila
            .get::<_, i64>(0))
            .unwrap(),
        antes
    );
    assert_eq!(
        r.init_session("i", "s", d.path()).unwrap()["modo"],
        "memoria_activa"
    );
    assert!(c
        .execute(
            "UPDATE proyecto_local SET modo_predeterminado='invalido'",
            []
        )
        .is_err());
    assert!(c
        .execute("UPDATE proyecto_local SET modo_predeterminado=NULL", [])
        .is_err());
}

fn hook(sesion: &str, cwd: &Path, evento: &str, extra: Value) -> Value {
    let mut v = json!({"hook_event_name":evento,"session_id":sesion,"cwd":cwd});
    for (k, valor) in extra.as_object().unwrap() {
        v[k] = valor.clone();
    }
    v
}
fn orden(a: &mut Adapter, cwd: &Path, texto: &str) -> Value {
    let salida = a
        .hook(&hook(
            "admin",
            cwd,
            "UserPromptSubmit",
            json!({"prompt":texto}),
        ))
        .unwrap()
        .contexto
        .unwrap();
    let cuerpo = salida
        .split_once('\n')
        .unwrap()
        .1
        .rsplit_once("\n</msp:control>")
        .unwrap()
        .0;
    serde_json::from_str(cuerpo).unwrap()
}

#[test]
fn orden_de_chat_estado_y_captura_sin_recuperacion_heredan_el_modo() {
    let d = tempfile::tempdir().unwrap();
    let raiz = d.path().join("datos");
    let cwd = d.path().join("proyecto");
    std::fs::create_dir(&cwd).unwrap();
    let mut r = Registry::open(&raiz).unwrap();
    let p = r.register_project(&cwd).unwrap()["raiz_normalizada"]
        .as_str()
        .unwrap()
        .to_string();
    let m = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    r.open_memory(&m).unwrap().capture(&json!({"version":1,"clave_evento":"anterior","grupo_evidencia":"anterior","secuencia":1,"timestamp":1,
        "origen":"modelo","rol":"solucion","contenido":"objetivo","metadatos":{"proyecto":p}}).to_string()).unwrap();
    let mut a = Adapter::new(r, "i".into(), 4096);
    let fijado = orden(
        &mut a,
        &cwd,
        &format!("msp:: predeterminado {m} solo_registro"),
    );
    assert_eq!(fijado["resultado"]["modo_predeterminado"], "solo_registro");
    let inicio = a
        .hook(&hook("nuevo", &cwd, "SessionStart", json!({})))
        .unwrap();
    assert_eq!(inicio.informe["estado"]["modo"], "solo_registro");
    let estado = a
        .hook(&hook(
            "nuevo",
            &cwd,
            "UserPromptSubmit",
            json!({"prompt":"msp:: estado"}),
        ))
        .unwrap()
        .contexto
        .unwrap();
    assert!(estado.contains("\"modo\": \"solo_registro\""));
    let captura = a
        .hook(&hook(
            "nuevo",
            &cwd,
            "UserPromptSubmit",
            json!({"prompt":"objetivo","prompt_id":"p1"}),
        ))
        .unwrap();
    assert!(captura.contexto.is_none());
    assert_eq!(captura.informe["entrega"]["busqueda"], false);
    let c = Connection::open(raiz.join("memorias").join(format!("{m}.sqlite"))).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM evento_recuperacion", [], |fila| fila
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    for texto in [
        "msp:: predeterminado".to_string(),
        format!("msp:: predeterminado {m} invalido"),
        format!("msp:: predeterminado {m} solo_registro extra"),
        "msp:: predeterminado quitar solo_registro".into(),
    ] {
        assert!(orden(&mut a, &cwd, &texto)["error"].is_string());
    }
    assert_eq!(
        orden(
            &mut a,
            &cwd,
            &format!("msp:: predeterminado {m} memoria_activa")
        )["resultado"]["modo_predeterminado"],
        "memoria_activa"
    );
    assert_eq!(
        a.hook(&hook("activa", &cwd, "SessionStart", json!({})))
            .unwrap()
            .informe["estado"]["modo"],
        "memoria_activa"
    );
    assert!(a
        .hook(&hook(
            "activa",
            &cwd,
            "UserPromptSubmit",
            json!({"prompt":"objetivo","prompt_id":"p2"})
        ))
        .unwrap()
        .contexto
        .is_some());
    assert_eq!(
        orden(&mut a, &cwd, &format!("msp:: predeterminado {m}"))["resultado"]
            ["modo_predeterminado"],
        "memoria_activa"
    );
    assert!(
        orden(&mut a, &cwd, "msp:: predeterminado quitar")["resultado"]["memoria_predeterminada"]
            .is_null()
    );
}

#[test]
fn cli_fija_modo_opcional_y_rechaza_modo_con_quitar() {
    let d = tempfile::tempdir().unwrap();
    let mut r = Registry::open(d.path()).unwrap();
    r.register_project(d.path()).unwrap();
    let m = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let corre = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_msp"))
            .args([
                "--control",
                "proyecto-predeterminado",
                "--raiz",
                d.path().to_str().unwrap(),
            ])
            .args(extra)
            .env("MSP_RAIZ", d.path())
            .output()
            .unwrap()
    };
    for (n, extra, modo) in [
        (
            "solo",
            vec!["--memoria", &m, "--modo", "solo_registro"],
            "solo_registro",
        ),
        (
            "activa",
            vec!["--memoria", &m, "--modo", "memoria_activa"],
            "memoria_activa",
        ),
        ("sin_modo", vec!["--memoria", &m], "memoria_activa"),
    ] {
        let salida = corre(&extra);
        assert!(
            salida.status.success(),
            "{}",
            String::from_utf8_lossy(&salida.stderr)
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&salida.stdout).unwrap()["modo_predeterminado"],
            modo
        );
        assert_eq!(r.init_session("i", n, d.path()).unwrap()["modo"], modo);
    }
    for extras in [
        vec!["--quitar", "--modo", "solo_registro"],
        vec!["--memoria", &m, "--modo", "invalido"],
        vec!["--modo", "solo_registro"],
        vec!["--quitar", "--memoria", &m],
    ] {
        let salida = corre(&extras);
        assert_eq!(salida.status.code(), Some(1));
        assert!(salida.stdout.is_empty());
    }
    assert!(corre(&["--quitar"]).status.success());
    assert_eq!(
        r.init_session("i", "sin", d.path()).unwrap()["modo"],
        "apagada"
    );
}

#[test]
fn migrar_registro_v2_preserva_predeterminado_historial_y_sesiones() {
    let d = tempfile::tempdir().unwrap();
    let raiz = d.path().join("datos");
    std::fs::create_dir(&raiz).unwrap();
    let cwd = d.path().join("proyecto");
    std::fs::create_dir(&cwd).unwrap();
    let (normalizada, original) = normalizar_raiz(&cwd).unwrap();
    let path = raiz.join("registro.sqlite");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TABLE registro_migraciones(version INTEGER PRIMARY KEY,sha256 TEXT NOT NULL,aplicado_en INTEGER NOT NULL)").unwrap();
    let migraciones = [
        include_str!("../registro/001_control.sql"),
        include_str!("../registro/002_tarea_y_modo.sql"),
    ];
    for (n, sql) in migraciones.iter().enumerate() {
        c.execute_batch(sql).unwrap();
        c.execute(
            "INSERT INTO registro_migraciones VALUES (?1,?2,13)",
            params![
                n as i64 + 1,
                format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
            ],
        )
        .unwrap();
    }
    let m = "a".repeat(32);
    let p = "b".repeat(32);
    c.execute(
        "INSERT INTO memoria VALUES (?1,'Anterior',?2,1,22)",
        params![m, format!("memorias/{m}.sqlite")],
    )
    .unwrap();
    c.execute(
        "INSERT INTO proyecto_local VALUES (?1,?2,?3,?4,22)",
        params![p, normalizada, original, m],
    )
    .unwrap();
    c.execute("INSERT INTO sesion_memoria(id,instalacion,sesion_host,proyecto_id,memoria_id,estado,origen_seleccion,revision,creada_en,actualizado_en,recuperacion)
        VALUES (19,'i','vieja',?1,?2,'encendida','sesion_explicita',0,22,22,0)",params![p,m]).unwrap();
    c.execute("INSERT INTO intervalo_captura(sesion_id,memoria_id,revision_activacion,frontera_apertura,estado,abierto_en,modo) VALUES (19,?1,0,'{}','abierto',22,'solo_registro')",[&m]).unwrap();
    c.execute(
        "INSERT INTO control_historial VALUES (17,19,'memoria_encendida','{\"anterior\":true}',22)",
        [],
    )
    .unwrap();
    let mut r = Registry::open(&raiz).unwrap();
    assert_eq!(
        r.init_session("i", "vieja", &cwd).unwrap()["modo"],
        "solo_registro"
    );
    assert_eq!(
        r.init_session("i", "nueva", &cwd).unwrap()["modo"],
        "memoria_activa"
    );
    assert_eq!(
        c.query_row(
            "SELECT modo_predeterminado FROM proyecto_local",
            [],
            |fila| fila.get::<_, String>(0)
        )
        .unwrap(),
        "memoria_activa"
    );
    assert_eq!(
        c.query_row(
            "SELECT detalle_json,registrado_en FROM control_historial WHERE id=17",
            [],
            |fila| Ok((fila.get::<_, String>(0)?, fila.get::<_, i64>(1)?))
        )
        .unwrap(),
        ("{\"anterior\":true}".into(), 22)
    );
    for (n, sql) in migraciones.iter().enumerate() {
        assert_eq!(
            c.query_row(
                "SELECT sha256,aplicado_en FROM registro_migraciones WHERE version=?1",
                [n as i64 + 1],
                |fila| Ok((fila.get::<_, String>(0)?, fila.get::<_, i64>(1)?))
            )
            .unwrap(),
            (
                format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes())),
                13
            )
        );
    }
    assert_eq!(
        c.query_row("SELECT count(*) FROM registro_migraciones", [], |fila| fila
            .get::<_, i64>(0))
            .unwrap(),
        5
    );
    drop(r);
    Registry::open(&raiz).unwrap();
    c.execute(
        "UPDATE registro_migraciones SET sha256='alterado' WHERE version=3",
        [],
    )
    .unwrap();
    assert!(Registry::open(&raiz).is_err());
}
