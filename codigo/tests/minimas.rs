//! Derivaciones conservadoras sobre episodios sellados y bases temporales.
use msp::minimas::clasificar_pasos;
use msp::tasks::{Criterio, TaskRecord};
use msp::Store;
use proptest::prelude::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::process::Command;

fn paso(id: i64, herramienta: &str, entrada: Value, exito: bool) -> Value {
    let mut v = json!({"version":1,"clave_evento":format!("evento/{id}"),"grupo_evidencia":"g","secuencia":id,"timestamp":id,
        "origen":"herramienta","rol":"otro","contenido":"crudo completo sin alterar",
        "metadatos":{"proyecto":"p","intervalo_captura":7,"payload_hook":{
            "hook_event_name":if exito {"PostToolUse"} else {"PostToolUseFailure"},
            "tool_name":herramienta,"tool_use_id":format!("op{id}"),"cwd":"/p","tool_input":entrada,
            "tool_response":{"stdout":"observado","stderr":"","interrupted":false},"is_interrupt":false}}});
    if !exito {
        v["metadatos"]["payload_hook"]["error"] = json!("fallo observado");
        v["metadatos"]["payload_hook"]
            .as_object_mut()
            .unwrap()
            .remove("tool_response");
    }
    v
}
fn bash(id: i64, comando: &str, exito: bool) -> Value {
    paso(id, "Bash", json!({"command":comando}), exito)
}
fn editar(id: i64) -> Value {
    paso(
        id,
        "Edit",
        json!({"file_path":"/p/lib.rs","old_string":"a","new_string":"b"}),
        true,
    )
}
fn escribir(id: i64, contenido: &str) -> Value {
    paso(
        id,
        "Write",
        json!({"file_path":"/p/lib.rs","content":contenido}),
        true,
    )
}
fn conservados(v: &[Value], exito: bool) -> Vec<usize> {
    clasificar_pasos(v, exito)
        .iter()
        .filter(|d| d.en_minima)
        .map(|d| d.orden_original)
        .collect()
}

#[test]
fn fallos_superados_tienen_motivo_y_apoyo_en_la_llamada_exitosa() {
    for (herramienta, entrada, motivo) in [
        (
            "Bash",
            json!({"command":"cargo test --offline"}),
            "comprobacion_fallida_superada",
        ),
        (
            "Bash",
            json!({"command":"cargo build"}),
            "reintento_equivalente_exitoso",
        ),
        (
            "Read",
            json!({"file_path":"/p/lib.rs"}),
            "reintento_equivalente_exitoso",
        ),
        (
            "mcp__servidor__consultar",
            json!({"clave":"x"}),
            "reintento_equivalente_exitoso",
        ),
    ] {
        let eventos = vec![
            paso(1, herramienta, entrada.clone(), false),
            paso(2, herramienta, entrada, true),
        ];
        let d = clasificar_pasos(&eventos, true);
        assert!(!d[0].en_minima);
        assert!(d[1].en_minima);
        assert_eq!(d[0].motivo, motivo);
        assert_eq!(d[0].certeza, "alta");
        assert_eq!(d[0].apoyo_orden, Some(2));
    }
    let mut ultimo = bash(2, "cargo test", true);
    ultimo["metadatos"]["payload_hook"]["tool_input"]["description"] = json!("otra descripcion");
    assert_eq!(
        conservados(&[bash(1, "cargo test", false), ultimo], true),
        vec![2]
    );
}

#[test]
fn igualdad_normalizada_no_prueba_reintento_ni_identidad_del_contexto() {
    assert_eq!(
        conservados(
            &[
                bash(1, "cargo test caso_a", false),
                bash(2, "cargo test caso_b", true)
            ],
            true
        ),
        vec![1, 2]
    );
    let primero = bash(1, "cargo test", false);
    for (campo, valor) in [
        ("cwd", json!("/otro")),
        ("tool_use_id", json!("op1")),
        ("tool_use_id", Value::Null),
        ("agent_id", json!("hijo")),
    ] {
        let mut segundo = bash(2, "cargo test", true);
        segundo["metadatos"]["payload_hook"][campo] = valor;
        assert_eq!(
            conservados(&[primero.clone(), segundo], true),
            vec![1, 2],
            "{campo}"
        );
    }
    let mut segundo = bash(2, "cargo test", true);
    segundo["grupo_evidencia"] = json!("otro chat");
    assert_eq!(conservados(&[primero.clone(), segundo], true), vec![1, 2]);
    let mut segundo = bash(2, "cargo test", true);
    segundo["metadatos"]["intervalo_captura"] = json!(8);
    assert_eq!(conservados(&[primero, segundo], true), vec![1, 2]);
    let a = paso(
        1,
        "Write",
        json!({"file_path":"/p/a.rs","content":"texto"}),
        false,
    );
    let b = paso(
        2,
        "Write",
        json!({"file_path":"/p/b.rs","content":"texto"}),
        true,
    );
    assert_eq!(conservados(&[a, b], true), vec![1, 2]);
}

#[test]
fn datos_ausentes_abortos_y_cambios_de_herramienta_no_prueban_reemplazo() {
    let primero = bash(1, "cargo test", false);
    for vacio in ["", " ", "\t\n"] {
        for campo in ["cwd", "tool_use_id", "tool_name"] {
            let mut a = primero.clone();
            let mut b = bash(2, "cargo test", true);
            a["metadatos"]["payload_hook"][campo] = json!(vacio);
            b["metadatos"]["payload_hook"][campo] = json!(vacio);
            assert_eq!(conservados(&[a, b], true), vec![1, 2]);
        }
        assert_eq!(
            conservados(&[bash(1, vacio, false), bash(2, vacio, true)], true),
            vec![1, 2]
        );
        let mut a = editar(1);
        let mut b = escribir(2, "a");
        a["metadatos"]["payload_hook"]["tool_input"]["file_path"] = json!(vacio);
        b["metadatos"]["payload_hook"]["tool_input"]["file_path"] = json!(vacio);
        assert_eq!(conservados(&[a, b], true), vec![1, 2]);
    }
    for campo in ["cwd", "tool_use_id", "tool_name", "tool_input"] {
        let mut a = primero.clone();
        let mut b = bash(2, "cargo test", true);
        a["metadatos"]["payload_hook"][campo] = Value::Null;
        b["metadatos"]["payload_hook"][campo] = Value::Null;
        assert_eq!(conservados(&[a, b], true), vec![1, 2], "{campo}");
    }
    for campo in ["cwd", "tool_use_id"] {
        for primero_ausente in [true, false] {
            let mut a = primero.clone();
            let mut b = bash(2, "cargo test", true);
            if primero_ausente {
                a["metadatos"]["payload_hook"][campo] = Value::Null;
            } else {
                b["metadatos"]["payload_hook"][campo] = Value::Null;
            }
            assert_eq!(
                conservados(&[a, b], true),
                vec![1, 2],
                "{campo} {primero_ausente}"
            );
        }
    }
    for intervalo in [Value::Null, json!("7")] {
        let mut a = primero.clone();
        let mut b = bash(2, "cargo test", true);
        a["metadatos"]["intervalo_captura"] = intervalo.clone();
        b["metadatos"]["intervalo_captura"] = intervalo;
        assert_eq!(conservados(&[a, b], true), vec![1, 2]);
    }
    for (campo, valor) in [
        ("hook_event_name", json!("PreToolUse")),
        ("is_interrupt", json!(true)),
        ("tool_name", json!("Otra")),
    ] {
        let mut b = bash(2, "cargo test", true);
        b["metadatos"]["payload_hook"][campo] = valor;
        assert_eq!(
            conservados(&[primero.clone(), b], true),
            vec![1, 2],
            "{campo}"
        );
    }
    let mut b = bash(2, "cargo test", true);
    b["metadatos"]["payload_hook"]["tool_input"]["run_in_background"] = json!(true);
    assert_eq!(conservados(&[primero, b], true), vec![1, 2]);
    let mut a = editar(1);
    a["metadatos"]["payload_hook"]["hook_event_name"] = Value::Null;
    assert_eq!(conservados(&[a, escribir(2, "a")], true), vec![1, 2]);
    let mut a = editar(1);
    a["grupo_evidencia"] = json!("");
    let mut b = escribir(2, "a");
    b["grupo_evidencia"] = json!("");
    assert_eq!(conservados(&[a, b], true), vec![1, 2]);
    let mut a = editar(1);
    a["metadatos"]["payload_hook"]["tool_input"]["file_path"] = Value::Null;
    let mut b = escribir(2, "a");
    b["metadatos"]["payload_hook"]["tool_input"]["file_path"] = Value::Null;
    assert_eq!(conservados(&[a, b], true), vec![1, 2]);
    let crudo = json!({"hook_event_name":"PostToolUseFailure","tool_name":"Read"});
    assert_eq!(
        clasificar_pasos(&[crudo], true)[0].motivo,
        "consulta_fallida"
    );
}

#[test]
fn una_escritura_completa_reemplaza_ediciones_sin_consumidor_intermedio() {
    let v = vec![editar(1), escribir(2, "b"), escribir(3, "a")];
    let d = clasificar_pasos(&v, true);
    assert_eq!(conservados(&v, true), vec![3]);
    for fila in &d[..2] {
        assert_eq!(fila.motivo, "reescritura_completa_posterior");
        assert_eq!(fila.apoyo_orden, Some(3));
        assert_eq!(fila.certeza, "alta");
    }
    let lector = paso(2, "Read", json!({"file_path":"/p/lib.rs"}), true);
    assert_eq!(
        conservados(&[editar(1), lector, escribir(3, "a")], true),
        vec![1, 2, 3]
    );
    let v = vec![
        editar(1),
        paso(2, "Read", json!({"file_path":"/p/no-existe"}), false),
        escribir(3, ""),
    ];
    assert_eq!(conservados(&v, true), vec![3]);
    let mut sin_cuerpo = escribir(2, "a");
    sin_cuerpo["metadatos"]["payload_hook"]["tool_input"]["content"] = Value::Null;
    assert_eq!(conservados(&[editar(1), sin_cuerpo], true), vec![1, 2]);
    let mut fallida = escribir(2, "a");
    fallida["metadatos"]["payload_hook"]["hook_event_name"] = json!("PostToolUseFailure");
    assert_eq!(conservados(&[editar(1), fallida], true), vec![1, 2]);
    let mut mismo_id = escribir(2, "a");
    mismo_id["metadatos"]["payload_hook"]["tool_use_id"] = json!("op1");
    assert_eq!(conservados(&[editar(1), mismo_id], true), vec![1, 2]);
}

#[test]
fn una_inversa_parcial_o_un_comando_git_no_prueban_reversion_completa() {
    let vuelta = paso(
        2,
        "Edit",
        json!({"file_path":"/p/lib.rs","old_string":"b","new_string":"a"}),
        true,
    );
    assert_eq!(conservados(&[editar(1), vuelta], true), vec![1, 2]);
    assert_eq!(
        conservados(&[editar(1), bash(2, "git restore lib.rs", true)], true),
        vec![1, 2]
    );
}

#[test]
fn consultas_dudosas_se_conservan_y_los_fallos_tienen_certeza_media() {
    for tool in ["Read", "Grep", "Glob"] {
        let entrada = if tool == "Read" {
            json!({"file_path":"/p/x.rs"})
        } else {
            json!({"path":"/p/x.rs","pattern":"x"})
        };
        let d = clasificar_pasos(&[paso(1, tool, entrada.clone(), false)], true);
        assert!(!d[0].en_minima);
        assert_eq!(d[0].motivo, "consulta_fallida");
        assert_eq!(d[0].certeza, "media");
        assert_eq!(d[0].apoyo_orden, None);
        let v = vec![paso(1, tool, entrada, true), bash(2, "cargo test", true)];
        let d = clasificar_pasos(&v, true);
        assert!(d[0].en_minima);
        assert_eq!(d[0].motivo, "candidata_sin_reuso_observado");
        assert_eq!(d[0].certeza, "baja");
    }
    let v = vec![
        paso(1, "Read", json!({"file_path":"/p/lib.rs"}), true),
        escribir(2, "a"),
    ];
    assert_eq!(
        clasificar_pasos(&v, true)[0].motivo,
        "conservado_sin_evidencia"
    );
    assert_eq!(conservados(&[bash(1, "cargo test", false)], true), vec![1]);
    let mut interrumpida = paso(1, "Read", json!({"file_path":"/p/x"}), false);
    interrumpida["metadatos"]["payload_hook"]["is_interrupt"] = json!(true);
    assert_eq!(conservados(&[interrumpida], true), vec![1]);
}

fn arbitrario() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        any::<String>().prop_map(Value::from),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from)
    ]
    .prop_recursive(3, 24, 5, |v| {
        prop_oneof![
            prop::collection::vec(v.clone(), 0..5).prop_map(Value::from),
            prop::collection::btree_map(
                prop::sample::select(vec![
                    "metadatos",
                    "payload_hook",
                    "grupo_evidencia",
                    "tool_name",
                    "tool_input",
                    "hook_event_name",
                    "cwd",
                    "file_path",
                    "path",
                    "content",
                    "tool_use_id"
                ])
                .prop_map(String::from),
                v,
                0..6
            )
            .prop_map(|m| Value::Object(m.into_iter().collect()))
        ]
    })
}
fn un_paso() -> impl Strategy<Value = Value> {
    prop_oneof![
        arbitrario(),
        (0i64..8, 0u8..8, any::<bool>()).prop_map(|(id, tipo, exito)| match tipo {
            0 => editar(id),
            1 => escribir(id, "a"),
            2 => escribir(id, "b"),
            3 => bash(id, "cargo test", exito),
            4 => bash(id, "cargo build", exito),
            5 => paso(id, "Read", json!({"file_path":"/p/lib.rs"}), exito),
            6 => paso(id, "Grep", json!({"path":"/p/lib.rs","pattern":"a"}), exito),
            _ => paso(
                id,
                "Glob",
                json!({"path":"/p/otro","pattern":"*.rs"}),
                exito
            ),
        })
    ]
}
proptest! {
    #[test]
    fn la_minima_es_subsecuencia_y_particiona_con_los_descartes(v in prop::collection::vec(un_paso(),0..35), exito in any::<bool>()) {
        let d=clasificar_pasos(&v,exito);prop_assert_eq!(d.len(),v.len());
        let min:Vec<_>=d.iter().filter(|r|r.en_minima).map(|r|r.orden_original).collect();
        let desc:Vec<_>=d.iter().filter(|r|!r.en_minima).map(|r|r.orden_original).collect();
        prop_assert!(min.windows(2).all(|w|w[0]<w[1]));
        let mut todos=min.clone();todos.extend(&desc);todos.sort_unstable();prop_assert_eq!(todos,(1..=v.len()).collect::<Vec<_>>());
        for r in &d {if let Some(j)=r.apoyo_orden {prop_assert!(j>r.orden_original && j<=d.len());prop_assert!(d[j-1].en_minima);}}
        if !exito {prop_assert!(min.is_empty());prop_assert!(d.iter().all(|r|r.motivo=="episodio_sin_exito"));}
    }
    #[test]
    fn la_clasificacion_es_total_y_determinista(v in prop::collection::vec(un_paso(),0..35), exito in any::<bool>()) {
        prop_assert_eq!(clasificar_pasos(&v,exito),clasificar_pasos(&v,exito));
    }
    #[test]
    fn agregar_al_final_un_paso_descartado_no_cambia_la_minima(v in prop::collection::vec(un_paso(),0..30), ultimo in un_paso()) {
        let antes=conservados(&v,true);let mut despues=v;despues.push(ultimo);
        let d=clasificar_pasos(&despues,true);
        if !d.last().unwrap().en_minima {prop_assert_eq!(conservados(&despues,true),antes);}
    }
    #[test]
    fn una_consulta_fallida_al_final_siempre_es_descartable(v in prop::collection::vec(un_paso(),0..30)) {
        let antes=conservados(&v,true);let mut despues=v;despues.push(paso(99,"Read",json!({"file_path":"/p/no-existe"}),false));
        let d=clasificar_pasos(&despues,true);prop_assert!(!d.last().unwrap().en_minima);prop_assert_eq!(conservados(&despues,true),antes);
    }
}

fn crear_manual(s: &mut Store, v: &[Value], exito: bool) -> Value {
    s.record_task(&TaskRecord::TareaCrear {
        tarea: "t".into(),
        proyecto: "p".into(),
        definicion: "def/1".into(),
        descripcion: "compilar".into(),
        criterios: vec![Criterio {
            id: "prueba".into(),
            version: "1".into(),
            descripcion: "suite".into(),
            obligatorio: true,
        }],
    })
    .unwrap();
    for evento in v {
        s.capture(&evento.to_string()).unwrap();
        s.record_task(&TaskRecord::TareaEnlazar {
            tarea: "t".into(),
            evento: evento["clave_evento"].as_str().unwrap().into(),
            evidencia: "explicita".into(),
        })
        .unwrap();
    }
    s.capture(&json!({"version":1,"clave_evento":"evidencia","grupo_evidencia":"verificacion","secuencia":1,"timestamp":100,"origen":"usuario","rol":"otro","contenido":"verificado",
        "metadatos":{"proyecto":"p","procedencia":"real","evaluacion":{"version":1,"fuente":"usuario_explicito","procedencia":"real","criterio":"prueba","criterio_version":"1",
            "alcance":"tarea","objetivo":null,"resultado":i64::from(exito),"detalle":{"tarea":"t","declaracion":"observacion externa"},"nota":"fixture","anterior":null}}}).to_string()).unwrap();
    s.record_task(&TaskRecord::TareaResultado {
        clave: "res".into(),
        tarea: "t".into(),
        criterio: "prueba".into(),
        criterio_version: "1".into(),
        evidencia: "evidencia".into(),
        alcance: "tarea".into(),
        objetivo: None,
        anterior: None,
    })
    .unwrap();
    s.record_task(&TaskRecord::TareaMarcar {
        clave: "cierre".into(),
        tarea: "t".into(),
        tipo: "cierre".into(),
        detalle: json!({}),
    })
    .unwrap();
    s.construir_episodio("ep", "p", "t", "res").unwrap()
}

#[test]
fn deriva_manual_realista_sin_cambiar_el_episodio_y_lo_relee_idempotente() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let v = vec![
        paso(1, "Read", json!({"file_path":"/p/lib.rs"}), true),
        editar(2),
        escribir(3, "a"),
        bash(4, "cargo test", false),
        bash(5, "cargo test", true),
    ];
    let ep = crear_manual(&mut s, &v, true);
    let calculada = s
        .calcular_secuencia_minima("p", "manual_tarea", "ep")
        .unwrap();
    let min = calculada["minima"].as_array().unwrap();
    let desc = calculada["descartados"].as_array().unwrap();
    assert_eq!(
        min.iter()
            .map(|p| p["orden_original"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 3, 5]
    );
    assert_eq!(
        desc.iter()
            .map(|p| p["orden_original"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![2, 4]
    );
    assert_eq!(desc[0]["motivo"], "reescritura_completa_posterior");
    assert_eq!(desc[1]["motivo"], "comprobacion_fallida_superada");
    let esperado=br#"[{"herramienta":"Read","tipo_destino":"archivo:rs"},{"herramienta":"Write","tipo_destino":"archivo:rs"},{"herramienta":"Bash","tipo_destino":"comando:cargo"}]"#;
    assert_eq!(
        calculada["derivacion"]["hash_minima"],
        format!("{:x}", Sha256::digest(esperado))
    );
    assert_eq!(
        calculada["derivacion"]["regla_version"],
        "minima-conservadora-v1"
    );
    assert_eq!(s.leer_episodio("p", "ep").unwrap(), ep);
    assert_eq!(
        s.calcular_secuencia_minima("p", "manual_tarea", "ep")
            .unwrap(),
        calculada
    );
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert_eq!(
        s.leer_secuencia_minima("p", "manual_tarea", "ep").unwrap(),
        calculada
    );
    assert!(s
        .calcular_secuencia_minima("ajeno", "manual_tarea", "ep")
        .is_err());
    assert!(s
        .leer_secuencia_minima("p", "automatico_lista", "ep")
        .is_err());
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM secuencia_minima", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(c
        .query_row(
            "SELECT json_original FROM captura_evento WHERE instancia_id=2",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap()
        .contains("old_string"));
}

fn lista(id: i64, estado: &str) -> Value {
    paso(
        id,
        "TodoWrite",
        json!({"todos":[{"content":"compilar","status":estado,"activeForm":"compilando"}]}),
        true,
    )
}

fn crear_automatico(s: &mut Store, v: &[Value], cerrar: bool) -> Value {
    s.capture(&lista(100, "in_progress").to_string()).unwrap();
    for evento in v {
        s.capture(&evento.to_string()).unwrap();
    }
    if cerrar {
        s.capture(&lista(200, "completed").to_string()).unwrap();
    }
    s.procesar_episodios_automaticos("p", "g", &[]).unwrap()
}

#[test]
fn deriva_automaticos_con_reintentos_y_conserva_el_sello_original() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let v = vec![
        editar(101),
        escribir(102, "a"),
        bash(103, "cargo test", false),
        bash(104, "cargo test", true),
    ];
    let original = crear_automatico(&mut s, &v, true);
    let id = original["episodios"][0]["id"].to_string();
    assert!(s
        .leer_secuencia_minima("p", "automatico_lista", &id)
        .is_err());
    let calculada = s
        .calcular_secuencia_minima("p", "automatico_lista", &id)
        .unwrap();
    assert_eq!(calculada["derivacion"]["origen"], "automatico_lista");
    assert_eq!(calculada["derivacion"]["episodio_manual_id"], Value::Null);
    assert_eq!(calculada["derivacion"]["cantidad_original"], 4);
    assert_eq!(calculada["derivacion"]["cantidad_minima"], 2);
    assert_eq!(calculada["minima"][0]["orden_original"], 2);
    assert_eq!(calculada["minima"][1]["orden_original"], 4);
    assert_eq!(calculada["descartados"][0]["apoyo_orden"], 2);
    assert_eq!(calculada["descartados"][1]["apoyo_orden"], 4);
    assert_eq!(
        s.listar_episodios_automaticos("p", "g").unwrap()["episodios"],
        original["episodios"]
    );
    assert_eq!(
        s.calcular_secuencia_minima("p", "automatico_lista", &id)
            .unwrap(),
        calculada
    );
    assert!(s
        .calcular_secuencia_minima("ajeno", "automatico_lista", &id)
        .is_err());
    assert!(s
        .leer_secuencia_minima("ajeno", "automatico_lista", &id)
        .is_err());
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM tarea", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM secuencia_minima", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn fallo_o_ausencia_de_comprobacion_no_fabrican_minima_ni_hash() {
    for resultado in [None, Some(false)] {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path().join("db")).unwrap();
        let v = if resultado.is_some() {
            vec![bash(101, "pytest", false)]
        } else {
            vec![editar(101)]
        };
        let ep = crear_automatico(&mut s, &v, true)["episodios"][0].clone();
        assert_eq!(
            ep["resultado"]["resultado"],
            resultado.map(|_| json!(0)).unwrap_or(Value::Null)
        );
        let id = ep["id"].to_string();
        let derivada = s
            .calcular_secuencia_minima("p", "automatico_lista", &id)
            .unwrap();
        assert_eq!(derivada["minima"], Value::Null);
        assert_eq!(derivada["derivacion"]["hash_minima"], Value::Null);
        assert_eq!(derivada["derivacion"]["llego"], 0);
        assert_eq!(derivada["derivacion"]["cantidad_minima"], 0);
        assert_eq!(derivada["descartados"].as_array().unwrap().len(), 1);
        assert_eq!(derivada["descartados"][0]["motivo"], "episodio_sin_exito");
        assert_eq!(
            s.leer_secuencia_minima("p", "automatico_lista", &id)
                .unwrap(),
            derivada
        );
    }
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    crear_manual(&mut s, &[editar(1)], false);
    let derivada = s
        .calcular_secuencia_minima("p", "manual_tarea", "ep")
        .unwrap();
    assert_eq!(derivada["minima"], Value::Null);
    assert_eq!(derivada["descartados"][0]["motivo"], "episodio_sin_exito");
}

#[test]
fn minimo_vacio_exitoso_tiene_hash_y_se_distingue_de_no_haber_llegado() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    crear_manual(&mut s, &[], true);
    let derivada = s
        .calcular_secuencia_minima("p", "manual_tarea", "ep")
        .unwrap();
    assert_eq!(derivada["minima"], json!([]));
    assert_eq!(derivada["descartados"], json!([]));
    assert_eq!(
        derivada["derivacion"]["hash_minima"],
        format!("{:x}", Sha256::digest(b"[]"))
    );
    assert_eq!(derivada["derivacion"]["llego"], 1);
}

#[test]
fn los_argumentos_literales_conservados_no_cambian_el_hash_normalizado() {
    let mut hashes = Vec::new();
    for (archivo, comando) in [("a.rs", "cargo test a"), ("b.rs", "cargo test b")] {
        let d = tempfile::tempdir().unwrap();
        let mut s = Store::open(d.path().join("db")).unwrap();
        crear_manual(
            &mut s,
            &[
                paso(1, "Read", json!({"file_path":archivo}), true),
                bash(2, comando, true),
            ],
            true,
        );
        hashes.push(
            s.calcular_secuencia_minima("p", "manual_tarea", "ep")
                .unwrap()["derivacion"]["hash_minima"]
                .clone(),
        );
    }
    assert_eq!(hashes[0], hashes[1]);
}

#[test]
fn rechaza_abiertos_referencias_invalidas_y_datos_ajenos_sin_dejar_derivaciones() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    assert!(
        crear_automatico(&mut s, &[bash(101, "cargo test", true)], false)["episodios"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for (p, o, id) in [
        ("p", "automatico_lista", "1"),
        ("p", "automatico_lista", "invalido"),
        ("p", "manual_tarea", "ausente"),
        ("p", "otro", "1"),
        (" ", "manual_tarea", "ep"),
        ("p", "manual_tarea", ""),
    ] {
        assert!(s.calcular_secuencia_minima(p, o, id).is_err());
        assert!(s.leer_secuencia_minima(p, o, id).is_err());
    }
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM secuencia_minima", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn base_exige_particion_completa_evidencia_conservada_y_derivacion_inmutable() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let ep = crear_manual(
        &mut s,
        &[
            bash(1, "cargo test", false),
            bash(2, "cargo test", false),
            bash(3, "cargo test", true),
        ],
        true,
    );
    let ep_id = ep["episodio"]["id"].as_i64().unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let cabecera="INSERT INTO secuencia_minima VALUES (100,'manual_tarea',?1,NULL,'minima-conservadora-v1','herramienta-destino-v1',1,?2,3,1,0)";
    c.execute(cabecera, params![ep_id, "0".repeat(64)]).unwrap();
    assert!(c.execute("INSERT INTO secuencia_minima_paso VALUES (100,2,2,1,'conservado_sin_evidencia','no_atribuida',NULL)",[]).is_err());
    assert!(c.execute("INSERT INTO secuencia_minima_paso VALUES (100,1,2,1,'conservado_sin_evidencia','no_atribuida',NULL)",[]).is_err());
    assert!(c
        .execute(
            "INSERT INTO secuencia_minima_paso VALUES (100,1,1,0,'episodio_sin_exito','alta',NULL)",
            []
        )
        .is_err());
    assert!(c
        .execute("INSERT INTO secuencia_minima_sello VALUES (100)", [])
        .is_err());
    // La evidencia posterior puede insertarse en la misma transaccion, nunca faltar al sellar.
    c.execute_batch("BEGIN").unwrap();
    c.execute("INSERT INTO secuencia_minima_paso VALUES (100,1,1,0,'comprobacion_fallida_superada','alta',2)",[]).unwrap();
    assert!(c
        .execute("INSERT INTO secuencia_minima_sello VALUES (100)", [])
        .is_err());
    c.execute(
        "INSERT INTO secuencia_minima_paso VALUES (100,2,2,0,'comprobacion_fallida_superada','alta',3)",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO secuencia_minima_paso VALUES (100,3,3,1,'conservado_sin_evidencia','no_atribuida',NULL)",[]).unwrap();
    assert!(c
        .execute("INSERT INTO secuencia_minima_sello VALUES (100)", [])
        .is_err());
    c.execute_batch("ROLLBACK").unwrap();
    c.execute_batch("BEGIN").unwrap();
    c.execute("INSERT INTO secuencia_minima_paso VALUES (100,1,1,0,'comprobacion_fallida_superada','alta',3)",[]).unwrap();
    c.execute("INSERT INTO secuencia_minima_paso VALUES (100,2,2,0,'comprobacion_fallida_superada','alta',3)",[]).unwrap();
    c.execute("INSERT INTO secuencia_minima_paso VALUES (100,3,3,1,'conservado_sin_evidencia','no_atribuida',NULL)",[]).unwrap();
    c.execute("INSERT INTO secuencia_minima_sello VALUES (100)", [])
        .unwrap();
    c.execute_batch("COMMIT").unwrap();
    for sql in ["UPDATE secuencia_minima SET creada_en=1","DELETE FROM secuencia_minima",
        "UPDATE secuencia_minima_paso SET certeza='alta'","DELETE FROM secuencia_minima_paso",
        "UPDATE secuencia_minima_sello SET derivacion_id=100","DELETE FROM secuencia_minima_sello",
        "INSERT INTO secuencia_minima_paso VALUES (100,4,3,1,'conservado_sin_evidencia','no_atribuida',NULL)"] {
        assert!(c.execute(sql,[]).is_err(),"{sql}");
    }
    assert_eq!(
        s.leer_secuencia_minima("p", "manual_tarea", "ep").unwrap()["minima"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn una_escritura_fallida_revierte_toda_la_derivacion_y_puede_reintentarse() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    crear_manual(&mut s, &[editar(1), escribir(2, "a")], true);
    let c = Connection::open(&path).unwrap();
    c.execute_batch("CREATE TRIGGER falla_fixture BEFORE INSERT ON secuencia_minima_sello BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    assert!(s
        .calcular_secuencia_minima("p", "manual_tarea", "ep")
        .is_err());
    for tabla in [
        "secuencia_minima",
        "secuencia_minima_paso",
        "secuencia_minima_sello",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {tabla}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    c.execute_batch("DROP TRIGGER falla_fixture").unwrap();
    assert_eq!(
        s.calcular_secuencia_minima("p", "manual_tarea", "ep")
            .unwrap()["descartados"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn cli_calcula_y_lee_ambos_origenes_en_procesos_separados() {
    let d = tempfile::tempdir().unwrap();
    let mut r = msp::control::Registry::open(d.path()).unwrap();
    let memoria = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut s = r.open_memory(&memoria).unwrap();
    crear_manual(&mut s, &[editar(1), escribir(2, "a")], true);
    let auto = crear_automatico(&mut s, &[bash(101, "cargo test", true)], true)["episodios"][0]
        ["id"]
        .to_string();
    drop(s);
    let corre = |comando: &str, origen: &str, id: &str| {
        Command::new(env!("CARGO_BIN_EXE_msp"))
            .args([
                "--control",
                comando,
                "--memoria",
                &memoria,
                "--proyecto",
                "p",
                "--origen",
                origen,
                "--episodio",
                id,
            ])
            .env("MSP_RAIZ", d.path())
            .output()
            .unwrap()
    };
    for (origen, id) in [("manual_tarea", "ep"), ("automatico_lista", auto.as_str())] {
        assert!(!corre("secuencia-leer", origen, id).status.success());
        let salida = corre("secuencia-calcular", origen, id);
        assert!(
            salida.status.success(),
            "{}",
            String::from_utf8_lossy(&salida.stderr)
        );
        let valor: Value = serde_json::from_slice(&salida.stdout).unwrap();
        assert_eq!(valor["derivacion"]["origen"], origen);
        assert_eq!(valor["minima"].as_array().unwrap().len(), 1);
        assert_eq!(
            salida.stdout,
            corre("secuencia-calcular", origen, id).stdout
        );
        assert_eq!(salida.stdout, corre("secuencia-leer", origen, id).stdout);
    }
    for (origen, id) in [
        ("otro", "ep"),
        ("automatico_lista", "invalido"),
        ("manual_tarea", "ausente"),
    ] {
        let salida = corre("secuencia-calcular", origen, id);
        assert_eq!(salida.status.code(), Some(1));
        assert!(salida.stdout.is_empty());
    }
}

#[test]
fn migrar_008_a_009_preserva_datos_y_hashes_y_verifica_la_nueva_migracion() {
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
        include_str!("../migrations/008_cortes_automaticos.sql"),
    ];
    for (i, sql) in migraciones.iter().enumerate() {
        c.execute_batch(sql).unwrap();
        c.execute(
            "INSERT INTO schema_migrations VALUES (?1,?2,13)",
            params![
                i as i64 + 1,
                format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
            ],
        )
        .unwrap();
    }
    c.execute("INSERT INTO tarea VALUES (19,'anterior','p',22,'{}')", [])
        .unwrap();
    let crudos = [
        lista(100, "in_progress"),
        bash(101, "cargo test", true),
        lista(200, "completed"),
    ];
    for v in &crudos {
        let id = v["secuencia"].as_i64().unwrap();
        c.execute("INSERT INTO instancia(id,clave_evento,grupo_evidencia,origen,rol,contenido,metadatos_json,timestamp) VALUES (?1,?2,'g','herramienta','otro',?3,?4,?1)",
            params![id,v["clave_evento"].as_str(),v["contenido"].as_str(),v["metadatos"].to_string()]).unwrap();
        c.execute(
            "INSERT INTO captura_evento VALUES (?1,'g',?1,?2,22)",
            params![id, v.to_string()],
        )
        .unwrap();
    }
    let hash = format!(
        "{:x}",
        Sha256::digest(br#"[{"herramienta":"Bash","tipo_destino":"comando:cargo"}]"#)
    );
    c.execute("INSERT INTO episodio_automatico VALUES (31,'automatico_lista','p','g','compilar','TodoWrite',100,200,NULL,NULL,'completado',101,'cortes-lista-v1','herramienta-destino-v1',?1,1,22)",[&hash]).unwrap();
    c.execute(
        "INSERT INTO episodio_automatico_paso VALUES (31,1,101,'Bash','comando:cargo')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO episodio_automatico_resultado VALUES (31,101,1,'ultima-comprobacion-v1')",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO episodio_automatico_sello VALUES (31)", [])
        .unwrap();
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.list_tasks("p").unwrap()["tareas"][0]["clave"], "anterior");
    assert_eq!(
        c.query_row("SELECT creada_en FROM tarea WHERE id=19", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        22
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    let ep = s.listar_episodios_automaticos("p", "g").unwrap()["episodios"][0].clone();
    assert_eq!(ep["id"], 31);
    assert_eq!(ep["creado_en"], 22);
    assert_eq!(ep["secuencia_sha256"], hash);
    assert_eq!(ep["resultado"]["resultado"], 1);
    assert_eq!(ep["pasos"][0]["instancia_id"], 101);
    let derivada = s
        .calcular_secuencia_minima("p", "automatico_lista", "31")
        .unwrap();
    assert_eq!(derivada["derivacion"]["hash_minima"], hash);
    assert_eq!(
        s.listar_episodios_automaticos("p", "g").unwrap()["episodios"][0],
        ep
    );
    for v in &crudos {
        assert_eq!(
            c.query_row(
                "SELECT json_original FROM captura_evento WHERE instancia_id=?1",
                [v["secuencia"].as_i64().unwrap()],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            v.to_string()
        );
    }
    for (i, sql) in migraciones.iter().enumerate() {
        assert_eq!(
            c.query_row(
                "SELECT sha256,aplicado_en FROM schema_migrations WHERE version=?1",
                [i as i64 + 1],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
            (
                format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes())),
                13
            )
        );
    }
    assert!(s
        .leer_secuencia_minima("p", "manual_tarea", "ausente")
        .is_err());
    drop(s);
    Store::open(&path).unwrap();
    c.execute(
        "UPDATE schema_migrations SET sha256='alterado' WHERE version=9",
        [],
    )
    .unwrap();
    assert!(Store::open(&path).is_err());
}
