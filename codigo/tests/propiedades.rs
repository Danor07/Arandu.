//! Propiedades de las entradas que vienen de afuera: el evento crudo de un
//! hook, el prompt del usuario y el payload completo del adaptador. Cada
//! propiedad vale para cualquier entrada, no sólo para los ejemplos.
use msp::adapter::{consulta_literal, envolver, sobre_bytes, Adapter};
use msp::control::Registry;
use msp::Store;
use proptest::prelude::*;
use serde_json::{json, Value};

fn evento(clave: &str, grupo: &str, secuencia: i64, contenido: &str) -> String {
    json!({"version":1,"clave_evento":clave,"grupo_evidencia":grupo,"secuencia":secuencia,
        "origen":"usuario","rol":"problema","timestamp":1,"contenido":contenido,
        "metadatos":{"tipo":"mensaje"}})
    .to_string()
}

fn json_arbitrario() -> impl Strategy<Value = Value> {
    let hoja = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        ".*".prop_map(Value::from),
        // Nombres reales de eventos, para llegar más allá del primer descarte.
        prop::sample::select(vec![
            "SessionStart",
            "SessionEnd",
            "UserPromptSubmit",
            "PostToolUse",
            "PostToolUseFailure",
            "Stop",
            "SubagentStop",
            "PreCompact",
            "msp:: estado",
        ])
        .prop_map(Value::from),
    ];
    hoja.prop_recursive(4, 32, 6, |interno| {
        prop_oneof![
            prop::collection::vec(interno.clone(), 0..6).prop_map(Value::from),
            prop::collection::btree_map(
                prop::sample::select(vec![
                    "hook_event_name",
                    "session_id",
                    "cwd",
                    "prompt",
                    "tool_name",
                    "tool_input",
                    "tool_response",
                    "transcript_path",
                    "otro",
                ])
                .prop_map(String::from),
                interno,
                0..8,
            )
            .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

proptest! {
    #[test]
    fn la_consulta_es_acotada_unica_literal_y_determinista(prompt in ".{0,20000}") {
        let consulta = consulta_literal(&prompt);
        let terminos: Vec<&str> = consulta.split(' ').filter(|t| !t.is_empty()).collect();
        prop_assert!(terminos.len() <= 64);
        let mut vistos = std::collections::HashSet::new();
        for t in &terminos {
            prop_assert!(vistos.insert(*t), "termino repetido: {t}");
            // Las minúsculas Unicode pueden agregar marcas combinantes (İ).
            prop_assert!(t.chars().all(|c| !c.is_whitespace()
                && (!c.is_ascii() || c.is_ascii_alphanumeric())));
        }
        prop_assert_eq!(consulta_literal(&prompt), consulta);
    }

    #[test]
    fn el_sobre_suma_bytes_fijos_y_no_se_cierra_desde_adentro(texto in ".*") {
        let sobre = envolver(&texto);
        prop_assert_eq!(sobre.len(), texto.len() + sobre_bytes());
        prop_assert!(sobre.contains(&texto));
        prop_assert_eq!(envolver(&texto), sobre.clone());
        // El cierre real usa un id derivado del contenido: el texto citado no
        // puede contener el cierre de su propio sobre.
        let cierre = sobre.lines().last().unwrap().to_string();
        prop_assert!(!texto.contains(&cierre));
    }

    #[test]
    fn capturar_dos_veces_lo_mismo_no_duplica(
        clave in "[a-z0-9/]{1,20}", grupo in "[a-z]{1,8}", secuencia in 0i64..1000,
        contenido in ".*", otro in ".*",
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.sqlite")).unwrap();
        let crudo = evento(&clave, &grupo, secuencia, &contenido);
        let primero = store.capture(&crudo).unwrap();
        let segundo = store.capture(&crudo).unwrap();
        prop_assert!(primero.insertado);
        prop_assert!(!segundo.insertado);
        prop_assert_eq!(primero.instancia_id, segundo.instancia_id);
        // La misma clave con otros bytes es un conflicto, nunca una sobrescritura.
        if otro != contenido {
            prop_assert!(store.capture(&evento(&clave, &grupo, secuencia, &otro)).is_err());
        }
    }

    #[test]
    fn un_evento_crudo_cualquiera_nunca_rompe_la_captura(crudo in ".*") {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.sqlite")).unwrap();
        let _ = store.capture(&crudo);
    }

    #[test]
    fn un_payload_cualquiera_nunca_rompe_el_adaptador(payload in json_arbitrario()) {
        let dir = tempfile::tempdir().unwrap();
        let registro = Registry::open(dir.path()).unwrap();
        let mut adaptador = Adapter::new(registro, "prop".to_string(), 4096);
        let _ = adaptador.hook(&payload);
    }
}

#[test]
fn la_i_con_punto_conserva_la_marca_combinante_sin_habilitar_sintaxis_fts() {
    assert_eq!(consulta_literal("İ İ \"OR\" (hola)*"), "i\u{307} or hola");
}

#[test]
fn la_consulta_se_recorta_en_el_ultimo_limite_de_caracter() {
    // 16_384 bytes es el tope: una «é» que lo cruza queda afuera entera.
    let largo = "a".repeat(16_383);
    assert_eq!(consulta_literal(&format!("{largo}ézz")), largo);
    // Justo en el tope no se pierde nada; un byte más, sí.
    let exacto = "a".repeat(16_384);
    assert_eq!(consulta_literal(&format!("{exacto}b")), exacto);
    assert_eq!(consulta_literal(&exacto), exacto);
}
