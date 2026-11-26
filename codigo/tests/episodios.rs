//! Episodios sobre tareas y crudo real en bases temporales.
use msp::episodios::{normalizar_herramienta, PasoNormalizado};
use msp::tasks::{Criterio, TaskRecord};
use msp::Store;
use proptest::prelude::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
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

fn herramienta(s: &mut Store, clave: &str, grupo: &str, secuencia: i64, payload: Value) -> i64 {
    let crudo = json!({"version":1,"clave_evento":clave,"grupo_evidencia":grupo,"secuencia":secuencia,
        "timestamp":1,"origen":"herramienta","rol":"otro","contenido":"crudo completo",
        "metadatos":{"proyecto":"p","tipo":"herramienta-ok","payload_hook":payload}});
    let id = s.capture(&crudo.to_string()).unwrap().instancia_id;
    s.record_task(&TaskRecord::TareaEnlazar {
        tarea: "t".into(),
        evento: clave.into(),
        evidencia: "asociacion explicita".into(),
    })
    .unwrap();
    id
}

fn resultado(s: &mut Store, clave: &str, valor: Option<i64>, anterior: Option<&str>) {
    let evidencia = format!("evidencia/{clave}");
    s.capture(&json!({"version":1,"clave_evento":evidencia,"grupo_evidencia":evidencia,"secuencia":1,"timestamp":1,
        "origen":"usuario","rol":"otro","contenido":"verificacion externa explicita",
        "metadatos":{"proyecto":"p","procedencia":"real","evaluacion":{
            "version":1,"fuente":"usuario_explicito","procedencia":"real","criterio":"prueba","criterio_version":"1",
            "alcance":"tarea","objetivo":null,"resultado":valor,"detalle":{"tarea":"t","declaracion":"comprobacion manual"},"nota":"fixture","anterior":null}}}).to_string()).unwrap();
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

fn preparar(path: &Path) -> Store {
    let mut s = Store::open(path).unwrap();
    tarea(&mut s);
    s
}

#[test]
fn reglas_del_normalizador_tienen_salidas_concretas_y_no_exponen_literales() {
    for (tool, input, destino) in [
        (
            "Read",
            json!({"file_path":"C:\\privado\\OTRO.RS"}),
            "archivo:rs",
        ),
        (
            "Edit",
            json!({"file_path":"/secreto/main.rs"}),
            "archivo:rs",
        ),
        (
            "Write",
            json!({"file_path":"README"}),
            "archivo:sin_extension",
        ),
        (
            "Read",
            json!({"file_path":"/x/.env"}),
            "archivo:sin_extension",
        ),
        ("Read", json!({"file_path":""}), "otro"),
        ("Read", json!({"file_path":42}), "otro"),
        ("Edit", json!({"file_path":"a."}), "archivo:sin_extension"),
        ("Read", json!({"file_path":"a.🚀"}), "otro"),
        (
            "Bash",
            json!({"command":"  C:\\tools\\CARGO.EXE test --offline"}),
            "comando:cargo",
        ),
        (
            "Bash",
            json!({"command":"/usr/bin/git status"}),
            "comando:git",
        ),
        ("Bash", json!({"command":""}), "otro"),
        ("Bash", json!({"command":"X=1 cargo test"}), "otro"),
        (
            "Bash",
            json!({"command":"\"/ruta con espacios/cargo\" test"}),
            "otro",
        ),
        ("Grep", json!({"pattern":"secreto"}), "busqueda"),
        ("Glob", json!({"pattern":"**/*.rs"}), "busqueda"),
        ("WebSearch", json!({"query":"otro literal"}), "busqueda"),
        (
            "WebFetch",
            json!({"url":"HTTPS://secreto.test/a"}),
            "url:https",
        ),
        ("WebFetch", json!({"url":"git+ssh://otro/a"}), "url:git+ssh"),
        ("WebFetch", json!({"url":"1https://a"}), "otro"),
        ("WebFetch", json!({"url":"ht tp://a"}), "otro"),
        ("WebFetch", json!({"url":"sin-esquema"}), "otro"),
        ("WebFetch", json!({"url":"://a"}), "otro"),
        ("Task", json!({"prompt":"texto privado"}), "subagente"),
        ("MCP", json!({"server":"servidor-1"}), "mcp:servidor-1"),
        ("MCP", json!({"server":""}), "otro"),
        ("MCP", json!({"server":"ruta/con/datos"}), "otro"),
    ] {
        assert_eq!(
            normalizar_herramienta(&json!({"tool_name":tool,"tool_input":input})),
            PasoNormalizado {
                herramienta: tool.into(),
                tipo_destino: destino.into()
            }
        );
    }
    for nombre in ["mcp__srv__operacion", "mcp__srv__otra"] {
        assert_eq!(
            normalizar_herramienta(&json!({"tool_name":nombre})),
            PasoNormalizado {
                herramienta: "MCP".into(),
                tipo_destino: "mcp:srv".into()
            }
        );
    }
    for nombre in [
        "Desconocida",
        "mcp____op",
        "mcp__srv__",
        "mcp__srv",
        "mcp__servidor/ruta__op",
    ] {
        assert_eq!(
            normalizar_herramienta(&json!({"tool_name":nombre})),
            PasoNormalizado {
                herramienta: "otro".into(),
                tipo_destino: "otro".into()
            }
        );
    }
    let payload = json!({"tool_name":"Read","tool_input":{"file_path":"x.rs"}});
    assert_eq!(
        normalizar_herramienta(&json!({"metadatos":{"payload_hook":payload}})),
        normalizar_herramienta(&payload)
    );
    assert_eq!(
        normalizar_herramienta(&json!({"tool_name":"Task","metadatos":{"payload_hook":false}}))
            .herramienta,
        "Task"
    );
}

fn cualquiera() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<String>().prop_map(Value::from)
    ]
    .prop_recursive(4, 32, 6, |interno| {
        prop_oneof![
            prop::collection::vec(interno.clone(), 0..5).prop_map(Value::from),
            prop::collection::btree_map(
                prop::sample::select(vec![
                    "tool_name",
                    "tool_input",
                    "file_path",
                    "command",
                    "url",
                    "server",
                    "metadatos",
                    "payload_hook"
                ])
                .prop_map(String::from),
                interno,
                0..8
            )
            .prop_map(|m| Value::Object(m.into_iter().collect()))
        ]
    })
}

proptest! {
    #[test]
    fn normalizar_es_total_y_determinista(evento in cualquiera()) {
        let primero = normalizar_herramienta(&evento);
        prop_assert_eq!(normalizar_herramienta(&evento), primero);
    }

    #[test]
    fn rutas_distintas_de_igual_extension_dan_el_mismo_paso(a in "[a-zA-Z0-9]{1,30}", b in "[a-zA-Z0-9]{1,30}", extension in "[a-z]{1,8}", herramienta in prop::sample::select(vec!["Read","Edit","Write"])) {
        let uno = json!({"tool_name":herramienta,"tool_input":{"file_path":format!("/uno/{a}.{extension}"),"otro":"literal"}});
        let dos = json!({"tool_name":herramienta,"tool_input":{"file_path":format!("C:\\dos\\{b}.{extension}")}});
        prop_assert_eq!(normalizar_herramienta(&uno),normalizar_herramienta(&dos));
    }

    #[test]
    fn argumentos_de_un_programa_no_cambian_el_paso(a in any::<String>(), b in any::<String>(), programa in prop::sample::select(vec!["cargo","git","python","node"])) {
        let uno = json!({"tool_name":"Bash","tool_input":{"command":format!("{programa} {a}")}});
        let dos = json!({"tool_name":"Bash","tool_input":{"command":format!("{programa} {b}")}});
        prop_assert_eq!(normalizar_herramienta(&uno),normalizar_herramienta(&dos));
    }
}

#[test]
fn episodio_ordenado_con_resultado_costo_y_crudo_intacto_es_idempotente() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = preparar(&path);
    let inicio = marcar(&mut s, "inicio");
    let z = herramienta(
        &mut s,
        "z",
        "z",
        20,
        json!({"tool_name":"Bash","tool_input":{"command":"cargo test --offline"}}),
    );
    let a30 = herramienta(
        &mut s,
        "a30",
        "a",
        30,
        json!({"tool_name":"Read","tool_input":{"file_path":"x.toml"}}),
    );
    let a10 = herramienta(
        &mut s,
        "a10",
        "a",
        10,
        json!({"tool_name":"Edit","tool_input":{"file_path":"privado.rs","old_string":"contenido"}}),
    );
    resultado(&mut s, "verificado", Some(1), None);
    std::thread::sleep(std::time::Duration::from_millis(1));
    let fin = marcar(&mut s, "cierre");
    let primero = s.construir_episodio("ep", "p", "t", "verificado").unwrap();
    assert_eq!(primero["episodio"]["tarea"], "t");
    assert_eq!(primero["episodio"]["criterio_id"], "prueba");
    assert_eq!(primero["episodio"]["criterio_version"], "1");
    assert_eq!(primero["episodio"]["cantidad_pasos"], 3);
    let pasos = primero["pasos"].as_array().unwrap();
    assert_eq!(
        pasos
            .iter()
            .map(|p| p["instancia_id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![a10, a30, z]
    );
    assert_eq!(
        pasos
            .iter()
            .map(|p| p["orden"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(pasos[0]["tipo_destino"], "archivo:rs");
    assert_eq!(pasos[1]["herramienta"], "Read");
    assert_eq!(pasos[2]["tipo_destino"], "comando:cargo");
    assert_eq!(primero["resultado"]["resultado"], 1);
    assert_eq!(primero["resultado"]["duracion_microsegundos"], fin - inicio);
    assert_eq!(primero["resultado"]["reintentos"], Value::Null);
    assert_eq!(primero["resultado"]["tokens"], Value::Null);
    let c = Connection::open(&path).unwrap();
    let antes: String = c
        .query_row(
            "SELECT json_original FROM captura_evento WHERE instancia_id=?1",
            [a10],
            |r| r.get(0),
        )
        .unwrap();
    assert!(antes.contains("privado.rs") && antes.contains("contenido"));
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert_eq!(
        s.construir_episodio("ep", "p", "t", "verificado").unwrap(),
        primero
    );
    assert_eq!(s.leer_episodio("p", "ep").unwrap(), primero);
    assert!(s.leer_episodio("ajeno", "ep").is_err());
    for (proyecto, tarea, res) in [
        ("otro", "t", "verificado"),
        ("p", "otra", "verificado"),
        ("p", "t", "otra"),
    ] {
        assert!(s
            .construir_episodio("ep", proyecto, tarea, res)
            .unwrap_err()
            .to_string()
            .contains("conflicto"));
    }
    assert_eq!(
        c.query_row("SELECT count(*) FROM episodio", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT json_original FROM captura_evento WHERE instancia_id=?1",
            [a10],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        antes
    );
}

#[test]
fn exige_cierre_y_comprobacion_conocida_vigente_de_la_misma_tarea_y_version() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = preparar(&path);
    resultado(&mut s, "res", Some(0), None);
    assert!(s.construir_episodio("ep", "p", "t", "res").is_err());
    marcar(&mut s, "inicio");
    assert!(s.construir_episodio("ep", "p", "t", "res").is_err());
    marcar(&mut s, "pausa");
    assert!(s.construir_episodio("ep", "p", "t", "res").is_err());
    marcar(&mut s, "cierre");
    resultado(&mut s, "desconocido", None, None);
    assert!(s.construir_episodio("ep", "p", "t", "desconocido").is_err());
    for (proyecto, tarea, res) in [
        ("ajeno", "t", "res"),
        ("p", "ajena", "res"),
        ("p", "t", "ausente"),
    ] {
        assert!(s.construir_episodio("ep", proyecto, tarea, res).is_err());
    }
    resultado(&mut s, "corregido", Some(1), Some("res"));
    assert!(s.construir_episodio("ep", "p", "t", "res").is_err());
    let nuevo = s.construir_episodio("ep", "p", "t", "corregido").unwrap();
    assert_eq!(nuevo["resultado"]["resultado"], 1);
    resultado(&mut s, "correccion-tardia", None, Some("corregido"));
    assert_eq!(
        s.construir_episodio("ep", "p", "t", "corregido").unwrap(),
        nuevo
    );
    assert!(s.construir_episodio("otra", "p", "t", "corregido").is_err());
    marcar(&mut s, "reanudacion");
    assert!(s
        .construir_episodio("otra", "p", "t", "correccion-tardia")
        .is_err());
    assert_eq!(s.leer_episodio("p", "ep").unwrap(), nuevo);
}

#[test]
fn fallo_sin_tiempos_conocidos_y_secuencia_vacia_no_inventa_costo() {
    let d = tempfile::tempdir().unwrap();
    let mut s = preparar(&d.path().join("db"));
    resultado(&mut s, "res", Some(0), None);
    marcar(&mut s, "cierre");
    let ep = s.construir_episodio("ep", "p", "t", "res").unwrap();
    assert_eq!(ep["resultado"]["resultado"], 0);
    assert_eq!(ep["resultado"]["duracion_microsegundos"], Value::Null);
    assert_eq!(ep["pasos"], json!([]));
    assert_eq!(
        ep["episodio"]["secuencia_sha256"],
        format!("{:x}", Sha256::digest(b"[]"))
    );
    assert_eq!(
        ep["episodio"]["normalizador_version"],
        "herramienta-destino-v1"
    );
    assert!(ep["costo"]["tokens"].as_str().unwrap().contains("NULL"));
}

#[test]
fn una_ventana_aun_abierta_no_se_presenta_como_duracion_completa() {
    let d = tempfile::tempdir().unwrap();
    let mut s = preparar(&d.path().join("db"));
    marcar(&mut s, "inicio");
    marcar(&mut s, "pausa");
    marcar(&mut s, "reanudacion");
    s.record_task(&TaskRecord::TareaMarcar {
        clave: "otro-inicio".into(),
        tarea: "t".into(),
        tipo: "inicio".into(),
        detalle: json!({}),
    })
    .unwrap();
    marcar(&mut s, "cierre");
    resultado(&mut s, "res", Some(1), None);
    assert_eq!(
        s.construir_episodio("ep", "p", "t", "res").unwrap()["resultado"]["duracion_microsegundos"],
        Value::Null
    );
}

#[test]
fn un_resultado_de_otra_definicion_no_comprueba_el_objetivo_del_cierre() {
    let d = tempfile::tempdir().unwrap();
    let mut s = preparar(&d.path().join("db"));
    resultado(&mut s, "anterior", Some(1), None);
    s.record_task(&TaskRecord::TareaRevisar {
        tarea: "t".into(),
        definicion: "def/2".into(),
        descripcion: "objetivo revisado".into(),
        criterios: vec![Criterio {
            id: "prueba".into(),
            version: "1".into(),
            descripcion: "suite revisada".into(),
            obligatorio: true,
        }],
    })
    .unwrap();
    marcar(&mut s, "cierre");
    assert!(s.construir_episodio("ep", "p", "t", "anterior").is_err());
    resultado(&mut s, "actual", Some(0), None);
    let ep = s.construir_episodio("ep", "p", "t", "actual").unwrap();
    assert_eq!(ep["episodio"]["definicion_version"], 2);
    assert_eq!(ep["resultado"]["resultado"], 0);
}

#[test]
fn argumentos_distintos_conservan_el_hash_y_los_enlaces_tardios_no_cambian_la_instantanea() {
    let d = tempfile::tempdir().unwrap();
    let mut hashes = Vec::new();
    for (n, archivo, argumentos) in [
        (1, "/uno/a.rs", "cargo test --offline"),
        (2, "C:\\dos\\b.rs", "cargo build --release"),
    ] {
        let mut s = preparar(&d.path().join(format!("db{n}")));
        herramienta(
            &mut s,
            "leer",
            "g",
            1,
            json!({"tool_name":"Read","tool_input":{"file_path":archivo}}),
        );
        herramienta(
            &mut s,
            "ejecutar",
            "g",
            2,
            json!({"tool_name":"Bash","tool_input":{"command":argumentos}}),
        );
        resultado(&mut s, "res", Some(1), None);
        marcar(&mut s, "cierre");
        let ep = s.construir_episodio("ep", "p", "t", "res").unwrap();
        hashes.push(ep["episodio"]["secuencia_sha256"].clone());
        herramienta(
            &mut s,
            "tardio",
            "g",
            3,
            json!({"tool_name":"Task","tool_input":{"prompt":"otro"}}),
        );
        assert_eq!(s.construir_episodio("ep", "p", "t", "res").unwrap(), ep);
        let otro = s
            .construir_episodio("otra-instantanea", "p", "t", "res")
            .unwrap();
        assert_ne!(
            otro["episodio"]["secuencia_sha256"],
            ep["episodio"]["secuencia_sha256"]
        );
    }
    assert_eq!(hashes[0], hashes[1]);
}

#[test]
fn conserva_fallos_y_herramientas_desconocidas_pero_no_agrega_marcadores_ni_texto() {
    let d = tempfile::tempdir().unwrap();
    let mut s = preparar(&d.path().join("db"));
    for (clave, origen, tipo, payload) in [
        (
            "prompt",
            "usuario",
            "prompt",
            json!({"prompt":"orden del usuario"}),
        ),
        (
            "compacta",
            "herramienta",
            "frontera-compactacion",
            json!({"hook_event_name":"PreCompact"}),
        ),
        (
            "fallo",
            "herramienta",
            "herramienta-error",
            json!({"tool_name":"Bash","tool_input":{"command":"cargo test"},"error":"fallo"}),
        ),
        (
            "desconocida",
            "herramienta",
            "herramienta-ok",
            json!({"tool_name":"NoConocida"}),
        ),
    ] {
        s.capture(&json!({"version":1,"clave_evento":clave,"grupo_evidencia":clave,"secuencia":1,"timestamp":1,
            "origen":origen,"rol":"otro","contenido":"todo el texto","metadatos":{"proyecto":"p","tipo":tipo,"payload_hook":payload}}).to_string()).unwrap();
        s.record_task(&TaskRecord::TareaEnlazar {
            tarea: "t".into(),
            evento: clave.into(),
            evidencia: "explicita".into(),
        })
        .unwrap();
    }
    resultado(&mut s, "res", Some(0), None);
    marcar(&mut s, "cierre");
    let ep = s.construir_episodio("ep", "p", "t", "res").unwrap();
    let pasos = ep["pasos"].as_array().unwrap();
    assert_eq!(pasos.len(), 2);
    assert_eq!(pasos[0]["herramienta"], "otro");
    assert_eq!(pasos[0]["tipo_destino"], "otro");
    assert_eq!(pasos[1]["tipo_destino"], "comando:cargo");
    assert_eq!(ep["resultado"]["resultado"], 0);
}

#[test]
fn construccion_atomica_y_tablas_inmutables_selladas_con_fks() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = preparar(&path);
    let instancia = herramienta(
        &mut s,
        "tool",
        "g",
        1,
        json!({"tool_name":"Read","tool_input":{"file_path":"x.rs"}}),
    );
    resultado(&mut s, "res", Some(1), None);
    marcar(&mut s, "cierre");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TRIGGER fallo_sello BEFORE INSERT ON episodio_resultado BEGIN SELECT RAISE(ABORT,'fallo inyectado'); END;").unwrap();
    assert!(s.construir_episodio("ep", "p", "t", "res").is_err());
    for tabla in ["episodio", "episodio_paso", "episodio_resultado"] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {tabla}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    c.execute_batch("DROP TRIGGER fallo_sello").unwrap();
    let ep = s.construir_episodio("ep", "p", "t", "res").unwrap();
    let id = ep["episodio"]["id"].as_i64().unwrap();
    for sql in [
        "UPDATE episodio SET clave='otra'",
        "DELETE FROM episodio",
        "UPDATE episodio_paso SET tipo_destino='otro'",
        "DELETE FROM episodio_paso",
        "UPDATE episodio_resultado SET resultado=0",
        "DELETE FROM episodio_resultado",
    ] {
        assert!(c.execute(sql, []).is_err(), "{sql}");
    }
    assert!(c
        .execute(
            "INSERT INTO episodio_paso VALUES (?1,2,?2,'Read','archivo:rs')",
            params![id, instancia]
        )
        .is_err());
    assert!(c
        .execute(
            "INSERT INTO episodio_paso VALUES (999,1,?1,'Read','archivo:rs')",
            [instancia]
        )
        .is_err());
    assert!(c
        .execute(
            "INSERT INTO episodio_resultado VALUES (999,1,1,NULL,NULL,NULL)",
            []
        )
        .is_err());
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
    assert_eq!(s.leer_episodio("p", "ep").unwrap(), ep);
}

#[test]
fn migracion_007_abre_una_base_006_y_preserva_tarea_resultados_y_crudo() {
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
    c.execute("INSERT INTO tarea VALUES (8,'historica','p',42,'{}')", [])
        .unwrap();
    c.execute("INSERT INTO tarea_evento(clave,tarea_id,tipo,definicion_version,descripcion,criterios_json,detalle_json,fuente,registrado_en,contrato_json) VALUES ('def',8,'definicion',1,'original','[{\"id\":\"c\",\"version\":\"1\",\"descripcion\":\"original\",\"obligatorio\":true}]','{}','usuario_explicito',43,'{}')",[]).unwrap();
    let legado = json!({"version":1,"clave_evento":"legado","grupo_evidencia":"g","secuencia":2,"timestamp":44,
        "origen":"herramienta","rol":"otro","contenido":"literal anterior",
        "metadatos":{"proyecto":"p","tipo":"herramienta-ok","payload_hook":{"tool_name":"Read","tool_input":{"file_path":"antes.rs"}}}}).to_string();
    c.execute("INSERT INTO instancia(id,clave_evento,grupo_evidencia,origen,rol,contenido,metadatos_json,timestamp) VALUES (1,'legado','g','herramienta','otro','literal anterior',?1,44)",
        [serde_json::from_str::<Value>(&legado).unwrap()["metadatos"].to_string()]).unwrap();
    c.execute(
        "INSERT INTO captura_evento VALUES (1,'g',2,?1,45)",
        [&legado],
    )
    .unwrap();
    c.execute("INSERT INTO tarea_vinculo(instancia_id,tarea_id,definicion_version,metodo,evidencia,vinculado_en,contrato_json) VALUES (1,8,1,'enlace_manual','explicita',46,'{}')",[]).unwrap();
    c.execute("INSERT INTO instancia(id,clave_evento,grupo_evidencia,origen,rol,contenido,metadatos_json,timestamp) VALUES (2,'evaluacion','v','usuario','otro','comprobacion anterior','{\"proyecto\":\"p\"}',47)",[]).unwrap();
    c.execute("INSERT INTO tarea_resultado(id,clave,tarea_id,definicion_version,criterio_id,criterio_version,fuente_instancia_id,alcance,fuente,resultado,detalle_json,registrado_en,contrato_json) VALUES (9,'verificado',8,1,'c','1',2,'tarea','usuario_explicito',1,'{\"tarea\":\"historica\"}',48,'{}')",[]).unwrap();
    c.execute("INSERT INTO tarea_evento(clave,tarea_id,tipo,definicion_version,detalle_json,fuente,registrado_en,contrato_json) VALUES ('cierre',8,'cierre',1,'{}','usuario_explicito',49,'{}')",[]).unwrap();
    let mut s = Store::open(&path).unwrap();
    assert_eq!(
        s.list_tasks("p").unwrap()["tareas"][0]["clave"],
        "historica"
    );
    assert_eq!(
        s.task_report("p", "historica").unwrap()["definiciones"][0]["descripcion"],
        "original"
    );
    assert_eq!(
        c.query_row("SELECT creada_en FROM tarea WHERE id=8", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        42
    );
    assert_eq!(
        c.query_row(
            "SELECT json_original FROM captura_evento WHERE instancia_id=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        legado
    );
    assert_eq!(
        c.query_row(
            "SELECT registrado_en FROM tarea_resultado WHERE id=9",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        48
    );
    let ep = s
        .construir_episodio("migrado", "p", "historica", "verificado")
        .unwrap();
    assert_eq!(ep["pasos"][0]["instancia_id"], 1);
    assert_eq!(ep["pasos"][0]["tipo_destino"], "archivo:rs");
    assert_eq!(ep["resultado"]["tarea_resultado_id"], 9);
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
        c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    drop(s);
    Store::open(&path).unwrap();
    c.execute(
        "UPDATE schema_migrations SET sha256='alterado' WHERE version=7",
        [],
    )
    .unwrap();
    assert!(Store::open(&path).is_err());
}

#[test]
fn cli_construye_relee_y_rechaza_conflictos_sin_confirmar() {
    let d = tempfile::tempdir().unwrap();
    let mut r = msp::control::Registry::open(d.path()).unwrap();
    let memoria = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut s = r.open_memory(&memoria).unwrap();
    tarea(&mut s);
    resultado(&mut s, "res", Some(1), None);
    marcar(&mut s, "cierre");
    drop(s);
    let corre = |comando: &str, extras: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_msp"))
            .args([
                "--control",
                comando,
                "--memoria",
                &memoria,
                "--proyecto",
                "p",
                "--episodio",
                "ep",
            ])
            .args(extras)
            .env("MSP_RAIZ", d.path())
            .output()
            .unwrap()
    };
    let primera = corre(
        "episodio-construir",
        &["--tarea", "t", "--resultado", "res"],
    );
    assert!(
        primera.status.success(),
        "{}",
        String::from_utf8_lossy(&primera.stderr)
    );
    assert_eq!(
        primera.stdout,
        corre(
            "episodio-construir",
            &["--tarea", "t", "--resultado", "res"]
        )
        .stdout
    );
    assert_eq!(primera.stdout, corre("episodio-leer", &[]).stdout);
    for extras in [
        vec!["--tarea", "t", "--resultado", "otra"],
        vec!["--tarea", "t"],
        vec![],
    ] {
        let salida = corre("episodio-construir", &extras);
        assert_eq!(salida.status.code(), Some(1));
        assert!(salida.stdout.is_empty());
    }
}

#[test]
fn la_base_rechaza_objetivos_ajenos_pasos_incompletos_y_resultados_incompatibles() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = preparar(&path);
    let uno = herramienta(
        &mut s,
        "uno",
        "g",
        1,
        json!({"tool_name":"Read","tool_input":{"file_path":"x.rs"}}),
    );
    let dos = herramienta(
        &mut s,
        "dos",
        "g",
        2,
        json!({"tool_name":"Write","tool_input":{"file_path":"y.rs"}}),
    );
    resultado(&mut s, "res", Some(1), None);
    marcar(&mut s, "cierre");
    let ep = s.construir_episodio("ep", "p", "t", "res").unwrap();
    let id = ep["episodio"]["id"].as_i64().unwrap();
    let res = ep["resultado"]["tarea_resultado_id"].as_i64().unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    assert!(c.execute("INSERT INTO episodio SELECT id+101,'criterio-ajeno',tarea_id,definicion_evento_id,cierre_evento_id,'otro',criterio_version,normalizador_version,secuencia_sha256,2,creada_en FROM episodio WHERE id=?1",[id]).is_err());
    assert!(c.execute("INSERT INTO episodio SELECT id+101,'sin-cierre',tarea_id,definicion_evento_id,definicion_evento_id,criterio_id,criterio_version,normalizador_version,secuencia_sha256,2,creada_en FROM episodio WHERE id=?1",[id]).is_err());
    c.execute("INSERT INTO episodio SELECT id+100,'copia',tarea_id,definicion_evento_id,cierre_evento_id,criterio_id,criterio_version,normalizador_version,secuencia_sha256,2,creada_en FROM episodio WHERE id=?1",[id]).unwrap();
    let copia = id + 100;
    c.execute("INSERT INTO instancia(id,clave_evento,grupo_evidencia,origen,rol,contenido,metadatos_json,timestamp) VALUES (500,'sin-crudo','g','herramienta','otro','x','{\"proyecto\":\"p\"}',1)",[]).unwrap();
    c.execute("INSERT INTO tarea_vinculo(instancia_id,tarea_id,definicion_version,metodo,evidencia,vinculado_en,contrato_json) VALUES (500,1,1,'enlace_manual','explicita',1,'{}')",[]).unwrap();
    assert!(c
        .execute(
            "INSERT INTO episodio_paso VALUES (?1,1,500,'Read','archivo:rs')",
            [copia]
        )
        .is_err());
    assert!(c
        .execute(
            "INSERT INTO episodio_paso VALUES (?1,2,?2,'Read','archivo:rs')",
            params![copia, uno]
        )
        .is_err());
    assert!(c
        .execute(
            "INSERT INTO episodio_paso VALUES (?1,1,9999,'Read','archivo:rs')",
            [copia]
        )
        .is_err());
    c.execute(
        "INSERT INTO episodio_paso VALUES (?1,1,?2,'Read','archivo:rs')",
        params![copia, uno],
    )
    .unwrap();
    assert!(c
        .execute(
            "INSERT INTO episodio_resultado VALUES (?1,?2,1,NULL,NULL,NULL)",
            params![copia, res]
        )
        .is_err());
    assert!(s.leer_episodio("p", "copia").is_err());
    c.execute(
        "INSERT INTO episodio_paso VALUES (?1,2,?2,'Write','archivo:rs')",
        params![copia, dos],
    )
    .unwrap();
    for (valor, duracion, reintentos, tokens) in [
        (0, None, None, None),
        (1, Some(-1), None, None),
        (1, None, Some(-1), None),
        (1, None, None, Some(-1)),
    ] {
        assert!(c
            .execute(
                "INSERT INTO episodio_resultado VALUES (?1,?2,?3,?4,?5,?6)",
                params![copia, res, valor, duracion, reintentos, tokens]
            )
            .is_err());
    }
    c.execute(
        "INSERT INTO episodio_resultado VALUES (?1,?2,1,NULL,NULL,NULL)",
        params![copia, res],
    )
    .unwrap();
    assert_eq!(
        s.leer_episodio("p", "copia").unwrap()["pasos"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
