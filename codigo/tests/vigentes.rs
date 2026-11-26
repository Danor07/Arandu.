//! Vigencia provisional sobre episodios y memorias temporales, sin juez LLM.
use msp::adapter::{envolver, envolver_vigente, sobre_bytes, Adapter};
use msp::control::Registry;
use msp::retrieval::{PrepareRequest, SearchRequest};
use msp::tasks::{Criterio, TaskRecord};
use msp::vigentes::{competir, tipo_objetivo};
use msp::Store;
use proptest::prelude::*;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::process::Command;

const OBJETIVO: &str = "Compilar módulo";
fn capturar(s: &mut Store, p: &str, chat: &str, n: i64, fecha: i64, payload: Value) -> i64 {
    s.capture(&json!({"version":1,"clave_evento":format!("{chat}/{n}"),"grupo_evidencia":chat,"secuencia":n,
        "timestamp":fecha,"origen":"herramienta","rol":"otro","contenido":"captura de herramienta",
        "metadatos":{"proyecto":p,"intervalo_captura":7,"payload_hook":payload}}).to_string()).unwrap().instancia_id
}
fn episodio_auto(
    s: &mut Store,
    p: &str,
    chat: &str,
    herramienta: &str,
    resultado: Option<bool>,
    fecha: i64,
    derivar: bool,
) -> Value {
    let lista = |estado: &str| {
        json!({"hook_event_name":"PostToolUse","tool_name":"TodoWrite",
        "tool_input":{"todos":[{"content":OBJETIVO,"status":estado,"activeForm":"compilando"}]}})
    };
    capturar(s, p, chat, 1, fecha, lista("in_progress"));
    // Varias herramientas separadas por coma son varios pasos del mismo episodio.
    let mut n = 1;
    for herramienta in herramienta.split(',') {
        n += 1;
        capturar(
            s,
            p,
            chat,
            n,
            fecha,
            json!({"hook_event_name":"PostToolUse","tool_name":herramienta,"cwd":"/p","tool_use_id":format!("{chat}/{n}"),
            "tool_input":{"file_path":"/p/lib.rs","content":"a"}}),
        );
    }
    if let Some(ok) = resultado {
        let mut payload = json!({"hook_event_name":if ok {"PostToolUse"}else{"PostToolUseFailure"},"tool_name":"Bash",
            "tool_use_id":format!("{chat}/{}", n + 1),"cwd":"/p","tool_input":{"command":"cargo test"},
            "tool_response":{"stdout":"salida","stderr":"","interrupted":false}});
        if !ok {
            payload.as_object_mut().unwrap().remove("tool_response");
            payload["error"] = json!("Exit code 1");
            payload["is_interrupt"] = json!(false);
        }
        capturar(s, p, chat, n + 1, fecha, payload);
    }
    capturar(s, p, chat, n + 2, fecha, lista("completed"));
    let ep = s.procesar_episodios_automaticos(p, chat, &[]).unwrap()["episodios"][0].clone();
    if derivar {
        s.calcular_secuencia_minima(p, "automatico_lista", &ep["id"].to_string())
            .unwrap();
    }
    ep
}
fn problema(s: &mut Store, p: &str, chat: &str, fecha: i64, texto: &str) -> i64 {
    s.capture(&json!({"version":1,"clave_evento":format!("pregunta/{chat}"),"grupo_evidencia":chat,"secuencia":100,
        "timestamp":fecha,"origen":"usuario","rol":"problema","contenido":texto,"metadatos":{"proyecto":p}}).to_string()).unwrap().instancia_id
}
fn peticion(id: i64, p: &str, presupuesto: usize) -> PrepareRequest {
    PrepareRequest {
        problema_id: id,
        busqueda: SearchRequest {
            proyecto: p.into(),
            consulta: "compilar módulo".into(),
            origen: None,
            rol: None,
            limite: 100,
        },
        presupuesto_bytes: presupuesto,
    }
}

#[test]
fn tipos_explicitos_ordenan_terminos_sin_volver_a_normalizar_unicode() {
    let a = tipo_objetivo("p", "MÓDULO: compilar, compilar!")
        .unwrap()
        .unwrap();
    assert_eq!(a, tipo_objetivo("p", "compilar módulo").unwrap().unwrap());
    assert_eq!(a.terminos, vec!["compilar", "módulo"]);
    assert_eq!(a.regla_version, "objetivo-terminos-v1");
    assert_ne!(
        a.clave,
        tipo_objetivo("otro", "compilar módulo")
            .unwrap()
            .unwrap()
            .clave
    );
    assert_ne!(
        a.clave,
        tipo_objetivo("p", "no compilar módulo")
            .unwrap()
            .unwrap()
            .clave
    );
    assert_eq!(
        tipo_objetivo("p", "İ").unwrap().unwrap().terminos,
        vec!["i\u{307}"]
    );
    assert_eq!(tipo_objetivo("p", " () 🚀").unwrap(), None);
    assert!(tipo_objetivo(" ", "x").is_err());
}

// Cada caso copia una base cerrada. Evita repetir diez migraciones por presupuesto.
thread_local! {
    static BASE_PRESUPUESTO: tempfile::TempDir = {
        let d=tempfile::tempdir().unwrap();let mut s=Store::open(d.path().join("db")).unwrap();
        episodio_auto(&mut s,"p","a","Read",Some(true),10,true);drop(s);d
    };
}
fn copia_presupuesto() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    BASE_PRESUPUESTO
        .with(|base| std::fs::copy(base.path().join("db"), d.path().join("db")).unwrap());
    d
}
proptest! {
    #[test]
    fn el_tipo_es_determinista_y_no_depende_del_orden_de_episodios(texto in any::<String>(), orden in prop::collection::vec(any::<u32>(),0..40)) {
        let antes=tipo_objetivo("p",&texto).unwrap();
        let mut episodios:Vec<_>=orden.iter().map(|_|tipo_objetivo("p",&texto).unwrap()).collect();
        episodios.reverse();prop_assert!(episodios.iter().all(|t|t==&antes));
        prop_assert_eq!(&antes,&tipo_objetivo("p",&texto).unwrap());
    }
    #[test]
    fn competir_no_depende_del_orden_en_que_llegan_las_observaciones(
        filas in prop::collection::vec((0u8..3, prop::option::of(any::<bool>()), 0i64..20), 0..40),
        semilla in any::<u64>(),
    ) {
        // Fechas repetidas a propósito: el desempate es la ID de la evidencia.
        let mut observadas: Vec<Value> = filas.iter().enumerate().map(|(i, (camino, resultado, fecha))| {
            let hash = format!("{camino:064}");
            let exito = *resultado == Some(true);
            json!({"resultado":resultado.map(i64::from),"fecha":fecha,"evidencia_id":i,
                "hash_minima":exito.then_some(&hash),"derivacion_id":exito.then_some(i),"secuencia_sha256":hash})
        }).collect();
        let caminos = competir(&observadas).unwrap();
        observadas.sort_by_key(|f| (f["evidencia_id"].as_u64().unwrap() ^ semilla).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        prop_assert_eq!(&caminos, &competir(&observadas).unwrap());
        prop_assert!(caminos.iter().filter(|c| c.estado == "vigente").count() <= 1);
        prop_assert_eq!(caminos.is_empty(), !filas.iter().any(|f| f.1 == Some(true)));
        for c in &caminos {
            prop_assert!(c.exitos > 0);
            prop_assert_eq!(c.exitos + c.fallos, c.serie.len());
            prop_assert_eq!(c.estado == "archivado", c.sucesor.is_some());
            prop_assert!(c.primera_verificacion <= c.ultima_verificacion);
        }
    }
    #[test]
    fn el_sobre_mantiene_sus_bytes_con_cualquier_texto(texto in any::<String>()) {
        let nuevo=envolver_vigente(&texto);prop_assert_eq!(nuevo.len(),texto.len()+sobre_bytes());
        prop_assert_eq!(nuevo.len(),envolver(&texto).len());prop_assert!(nuevo.contains("autoridad=\"ninguna\""));
        prop_assert!(nuevo.contains(&texto));
    }
    #[test]
    fn recuperar_nunca_excede_el_presupuesto_del_sobre(presupuesto in 0usize..6000, extra in any::<String>()) {
        let d=copia_presupuesto();let mut s=Store::open(d.path().join("db")).unwrap();
        s.capture(&json!({"version":1,"clave_evento":"texto","grupo_evidencia":"texto","secuencia":1,"timestamp":1,
            "origen":"modelo","rol":"solucion","contenido":format!("{OBJETIVO} {extra}"),"metadatos":{"proyecto":"p"}}).to_string()).unwrap();
        let id=problema(&mut s,"p","actual",i64::MAX,OBJETIVO);
        let paquete=s.prepare_memory("paquete",&peticion(id,"p",presupuesto.saturating_sub(sobre_bytes()))).unwrap();
        let bloque=paquete["texto_memoria"].as_str().unwrap();
        prop_assert_eq!(paquete["bytes_utf8"].as_u64().unwrap() as usize,bloque.len());
        prop_assert!(bloque.len()<=presupuesto.saturating_sub(sobre_bytes()));
        if !bloque.is_empty() {let sobre=if paquete["vigente"].is_object(){envolver_vigente(bloque)}else{envolver(bloque)};prop_assert!(sobre.len()<=presupuesto);}
    }
}

#[test]
fn la_primera_sigue_vigente_y_el_tipo_muestra_fallos_y_fechas() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let ep = episodio_auto(&mut s, "p", "a", "Read", Some(true), 10, true);
    episodio_auto(&mut s, "p", "b", "Read", Some(true), 20, true);
    episodio_auto(&mut s, "p", "c", "Write", Some(true), 30, true);
    episodio_auto(&mut s, "p", "d", "Write", Some(false), 40, false);
    episodio_auto(&mut s, "p", "e", "Read", None, 50, false);
    let elegida = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(elegida["vigente"]["camino"]["exitos"], 2);
    assert_eq!(elegida["vigente"]["camino"]["primera_verificacion"], 10);
    assert_eq!(elegida["vigente"]["camino"]["ultima_verificacion"], 20);
    assert_eq!(elegida["vigente"]["pasos"][0]["herramienta"], "Read");
    assert_eq!(elegida["evidencia_tipo"]["exitos"], 3);
    assert_eq!(elegida["evidencia_tipo"]["fallos"], 1);
    assert_eq!(elegida["evidencia_tipo"]["ultimo_fallo"], 40);
    assert_eq!(elegida["evidencia_tipo"]["exitos_sin_minima"], 0);
    assert_eq!(elegida["evidencia_tipo"]["sin_verificacion"], 1);
    assert_eq!(elegida["evidencia_tipo"]["primera_fecha"], 10);
    assert_eq!(elegida["caminos"].as_array().unwrap().len(), 2);
    assert!(elegida["provisional"].as_bool().unwrap());
    assert_eq!(
        s.listar_episodios_automaticos("p", "a").unwrap()["episodios"][0],
        ep
    );
    assert_eq!(
        s.listar_tipos_objetivo("p").unwrap()["tipos"],
        json!([elegida.clone()])
    );
    drop(s);
    let mut s = Store::open(&path).unwrap();
    assert_eq!(s.consultar_vigente("p", OBJETIVO).unwrap(), elegida);
    assert_eq!(
        s.listar_tipos_objetivo("ajeno").unwrap()["tipos"],
        json!([])
    );
    assert!(s.consultar_vigente("ajeno", OBJETIVO).unwrap()["vigente"].is_null());
    assert!(s.listar_tipos_objetivo(" ").is_err());
}

#[test]
fn entrega_vigente_antes_del_texto_y_el_paquete_queda_congelado() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    episodio_auto(&mut s, "p", "a", "Read", Some(true), 10, true);
    s.capture(&json!({"version":1,"clave_evento":"texto","grupo_evidencia":"texto","secuencia":1,"timestamp":1,
        "origen":"usuario","rol":"problema","contenido":OBJETIVO,"metadatos":{"proyecto":"p"}}).to_string()).unwrap();
    let id = problema(&mut s, "p", "actual", i64::MAX, OBJETIVO);
    let req = peticion(id, "p", 10000);
    let a = s.prepare_memory("paquete", &req).unwrap();
    let lineas: Vec<_> = a["texto_memoria"]
        .as_str()
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect();
    assert_eq!(lineas[0]["tipo_registro"], "solucion_vigente_provisional");
    assert_eq!(lineas[1]["clave_evento"], "texto");
    let tam = lineas[0].to_string().len() + 1;
    let exacta = s.prepare_memory("justa", &peticion(id, "p", tam)).unwrap();
    assert_eq!(exacta["bytes_utf8"], tam);
    assert_eq!(exacta["seleccion"], json!([]));
    let corta = s
        .prepare_memory("corta", &peticion(id, "p", tam - 1))
        .unwrap();
    assert!(corta["vigente"].is_null());
    assert_eq!(corta["seleccion"].as_array().unwrap().len(), 1);
    episodio_auto(&mut s, "p", "nuevo", "Write", Some(true), 50, true);
    episodio_auto(&mut s, "p", "nuevo2", "Write", Some(true), 60, true);
    assert_eq!(s.prepare_memory("paquete", &req).unwrap(), a);
    assert_eq!(s.memory_packet("p", "paquete").unwrap(), a);
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM recuperacion_vigente", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row(
            "SELECT texto_entregado FROM recuperacion_vigente ORDER BY evento_id LIMIT 1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        lineas[0].to_string() + "\n"
    );
}

#[test]
fn sin_vigente_el_textual_conserva_bloques_seleccion_y_configuracion() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    episodio_auto(&mut s, "p", "fallo", "Read", Some(false), 10, true);
    s.capture(&json!({"version":1,"clave_evento":"texto","grupo_evidencia":"otro","secuencia":1,"timestamp":1,
        "origen":"modelo","rol":"solucion","contenido":"compilar módulo","metadatos":{"proyecto":"p"}}).to_string()).unwrap();
    let id = problema(&mut s, "p", "actual", i64::MAX, OBJETIVO);
    let paquete = s
        .prepare_memory("sin_exito", &peticion(id, "p", 2000))
        .unwrap();
    assert!(paquete.get("vigente").is_none());
    let row = &paquete["seleccion"][0];
    let esperado=json!({"instancia_id":row["candidato"]["instancia_id"],"clave_evento":"texto","grupo_evidencia":"otro","secuencia":1,"origen":"modelo","rol":"solucion","contenido":"compilar módulo"}).to_string()+"\n";
    assert_eq!(paquete["texto_memoria"], esperado);
    assert_eq!(row["texto_entregado"], esperado);
    assert_eq!(row["orden"], 1);
    assert_eq!(paquete["omitidos"], json!([]));
    assert_eq!(
        s.consultar_vigente("p", OBJETIVO).unwrap()["evidencia_tipo"]["fallos"],
        1
    );
    let c = Connection::open(path).unwrap();
    let config:String=c.query_row("SELECT c.parametros_json FROM configuracion c JOIN evento_recuperacion r ON r.configuracion_id=c.id WHERE r.clave_evento='sin_exito'",[],|r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&config).unwrap(),
        json!({"algoritmo":"episodica-fts5-v1",
        "tokenizer":"unicode61 remove_diacritics 2","consulta":"terminos-literales-OR",
        "ranking":"bm25 ASC,clave_evento BINARY ASC,id ASC","fragmento_terminos":24,
        "paquete":"eventos-completos-jsonl-v1","unidad":"bytes_utf8",
        "corte":"id<problema.id y timestamp<=problema.timestamp; otro episodio"})
    );
}

#[test]
fn excluye_chat_actual_y_capturas_o_minimas_posteriores_al_pedido() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    let ep = episodio_auto(&mut s, "p", "actual", "Read", Some(true), 10, false);
    assert_eq!(
        s.consultar_vigente("p", OBJETIVO).unwrap()["evidencia_tipo"]["exitos_sin_minima"],
        1
    );
    let c = Connection::open(&path).unwrap();
    let fecha: i64 = c
        .query_row(
            "SELECT creado_en FROM episodio_automatico WHERE id=?1",
            [ep["id"].as_i64().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let id = problema(&mut s, "p", "consulta", fecha, OBJETIVO);
    s.calcular_secuencia_minima("p", "automatico_lista", &ep["id"].to_string())
        .unwrap();
    assert!(s
        .prepare_memory("minima_tardia", &peticion(id, "p", 10000))
        .unwrap()["vigente"]
        .is_null());
    let propio = problema(&mut s, "p", "actual", i64::MAX, OBJETIVO);
    assert!(s
        .prepare_memory("propio", &peticion(propio, "p", 10000))
        .unwrap()["vigente"]
        .is_null());
    let antes = problema(&mut s, "p", "antes", i64::MAX, OBJETIVO);
    episodio_auto(&mut s, "p", "posterior", "Write", Some(true), 20, true);
    let packet = s
        .prepare_memory("corte", &peticion(antes, "p", 10000))
        .unwrap();
    assert_eq!(
        packet["vigente"]["vigente"]["pasos"][0]["herramienta"],
        "Read"
    );
    assert_eq!(packet["vigente"]["evidencia_tipo"]["exitos"], 1);
}

#[test]
fn filtros_explicitos_y_limite_cero_mantienen_la_ruta_textual() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    episodio_auto(&mut s, "p", "a", "Read", Some(true), 10, true);
    let id = problema(&mut s, "p", "actual", i64::MAX, OBJETIVO);
    for n in 0..3 {
        let mut r = peticion(id, "p", 10000);
        match n {
            0 => r.busqueda.limite = 0,
            1 => r.busqueda.origen = Some("modelo".into()),
            _ => r.busqueda.rol = Some("solucion".into()),
        };
        assert!(s.prepare_memory(&format!("filtro{n}"), &r).unwrap()["vigente"].is_null());
    }
}

#[test]
fn adaptador_entrega_en_activa_y_solo_registro_no_recupera_aunque_haya_vigente() {
    let d = tempfile::tempdir().unwrap();
    let cwd = d.path().join("proyecto");
    std::fs::create_dir(&cwd).unwrap();
    let raiz = d.path().join("memoria");
    let mut registry = Registry::open(&raiz).unwrap();
    let p = registry.register_project(&cwd).unwrap()["raiz_normalizada"]
        .as_str()
        .unwrap()
        .to_string();
    let m = registry.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut s = registry.open_memory(&m).unwrap();
    episodio_auto(&mut s, &p, "anterior", "Read", Some(true), 10, true);
    drop(s);
    registry
        .use_memory_with_mode("i", "activa", &cwd, &m, "memoria_activa")
        .unwrap();
    registry
        .use_memory_with_mode("i", "registro", &cwd, &m, "solo_registro")
        .unwrap();
    let mut a = Adapter::new(registry, "i".into(), 4096);
    let mut prompt = json!({"hook_event_name":"UserPromptSubmit","session_id":"activa","cwd":cwd,"prompt_id":"p1","prompt":OBJETIVO});
    prompt["session_id"] = json!("registro");
    let solo = a.hook(&prompt).unwrap();
    assert!(solo.contexto.is_none());
    assert_eq!(solo.informe["entrega"]["busqueda"], false);
    let c = Connection::open(raiz.join("memorias").join(format!("{m}.sqlite"))).unwrap();
    for tabla in [
        "evento_recuperacion",
        "tipo_objetivo",
        "recuperacion_vigente",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {tabla}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    prompt["session_id"] = json!("activa");
    let activa = a.hook(&prompt).unwrap();
    let texto = activa.contexto.unwrap();
    assert!(texto.contains("solucion_vigente_provisional"));
    assert!(texto.contains("autoridad=\"ninguna\""));
    assert!(texto.contains("de tipo o texto."));
    assert!(texto.len() <= 4096);
    let r = Registry::open(&raiz).unwrap();
    let s = r.open_memory(&m).unwrap();
    assert_eq!(
        s.deliveries(&p).unwrap()["entregas"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

fn resultado_manual(
    s: &mut Store,
    clave: &str,
    valor: Option<i64>,
    anterior: Option<&str>,
) -> String {
    let evidencia = format!("evidencia/{clave}");
    s.capture(&json!({"version":1,"clave_evento":evidencia,"grupo_evidencia":"verificacion","secuencia":if anterior.is_none(){1}else{2},"timestamp":20,
        "origen":"usuario","rol":"otro","contenido":"verificacion externa","metadatos":{"proyecto":"p","procedencia":"real","evaluacion":{
            "version":1,"fuente":"usuario_explicito","procedencia":"real","criterio":"prueba","criterio_version":"1","alcance":"tarea","objetivo":null,
            "resultado":valor,"detalle":{"tarea":"t","declaracion":"suite externa"},"nota":"fixture","anterior":null}}}).to_string()).unwrap();
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
    clave.into()
}
fn manual(s: &mut Store) -> Value {
    s.record_task(&TaskRecord::TareaCrear {
        tarea: "t".into(),
        proyecto: "p".into(),
        definicion: "def".into(),
        descripcion: OBJETIVO.into(),
        criterios: vec![Criterio {
            id: "prueba".into(),
            version: "1".into(),
            descripcion: "suite pasa".into(),
            obligatorio: true,
        }],
    })
    .unwrap();
    capturar(
        s,
        "p",
        "manual",
        1,
        10,
        json!({"tool_name":"Read","tool_input":{"file_path":"lib.rs"},"hook_event_name":"PostToolUse"}),
    );
    s.record_task(&TaskRecord::TareaEnlazar {
        tarea: "t".into(),
        evento: "manual/1".into(),
        evidencia: "explicita".into(),
    })
    .unwrap();
    resultado_manual(s, "res", Some(1), None);
    s.record_task(&TaskRecord::TareaMarcar {
        clave: "cierre".into(),
        tarea: "t".into(),
        tipo: "cierre".into(),
        detalle: json!({}),
    })
    .unwrap();
    let ep = s.construir_episodio("ep", "p", "t", "res").unwrap();
    s.calcular_secuencia_minima("p", "manual_tarea", "ep")
        .unwrap();
    ep
}

#[test]
fn manual_usa_el_objetivo_congelado_y_no_duplica_votos_por_instantaneas_del_mismo_resultado() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    let ep = manual(&mut s);
    s.construir_episodio("copia", "p", "t", "res").unwrap();
    s.calcular_secuencia_minima("p", "manual_tarea", "copia")
        .unwrap();
    s.record_task(&TaskRecord::TareaRevisar {
        tarea: "t".into(),
        definicion: "def2".into(),
        descripcion: "otro objetivo".into(),
        criterios: vec![Criterio {
            id: "prueba".into(),
            version: "1".into(),
            descripcion: "otra suite".into(),
            obligatorio: true,
        }],
    })
    .unwrap();
    let vigente = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(vigente["evidencia_tipo"]["exitos"], 1);
    assert_eq!(vigente["vigente"]["camino"]["exitos"], 1);
    assert_eq!(vigente["observaciones"].as_array().unwrap().len(), 1);
    assert_eq!(vigente["vigente"]["referencia"]["origen"], "manual_tarea");
    assert_eq!(vigente["vigente"]["referencia"]["criterio_id"], "prueba");
    assert!(s.consultar_vigente("p", "otro objetivo").unwrap()["vigente"].is_null());
    assert_eq!(s.leer_episodio("p", "ep").unwrap(), ep);
}

#[test]
fn una_correccion_conocida_retira_el_exito_pero_una_futura_no_cambia_el_corte() {
    for valor in [None, Some(0)] {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("db");
        let mut s = Store::open(&path).unwrap();
        manual(&mut s);
        let id = problema(&mut s, "p", "actual", i64::MAX, OBJETIVO);
        resultado_manual(&mut s, "corregido", valor, Some("res"));
        assert!(s.consultar_vigente("p", OBJETIVO).unwrap()["vigente"].is_null());
        // Aunque la consulta se haga despues, la correccion aun no existia para ese pedido.
        assert!(s
            .prepare_memory("historico", &peticion(id, "p", 10000))
            .unwrap()["vigente"]
            .is_object());
        let nuevo = problema(&mut s, "p", "nuevo", i64::MAX, OBJETIVO);
        assert!(s
            .prepare_memory("nuevo", &peticion(nuevo, "p", 10000))
            .unwrap()["vigente"]
            .is_null());
    }
}

#[test]
fn tambien_retira_una_evaluacion_de_ejecucion_referenciada_que_se_corrigio() {
    use msp::execution::Record;
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    s.record_task(&TaskRecord::TareaCrear {
        tarea: "t".into(),
        proyecto: "p".into(),
        definicion: "def".into(),
        descripcion: OBJETIVO.into(),
        criterios: vec![Criterio {
            id: "prueba".into(),
            version: "1".into(),
            descripcion: "suite".into(),
            obligatorio: true,
        }],
    })
    .unwrap();
    s.capture(&json!({"version":1,"clave_evento":"pregunta/verificacion","grupo_evidencia":"verificacion","secuencia":0,
        "timestamp":10,"origen":"usuario","rol":"problema","contenido":OBJETIVO,"metadatos":{"proyecto":"p","procedencia":"real"}}).to_string()).unwrap();
    s.record_execution(&Record::Intento {
        intento: "intento".into(),
        proyecto: "p".into(),
        problema: "pregunta/verificacion".into(),
    })
    .unwrap();
    for (n, valor, anterior) in [(1, 1, None), (2, 0, Some("eval1"))] {
        let clave = format!("evidencia{n}");
        s.capture(&json!({"version":1,"clave_evento":clave,"grupo_evidencia":"verificacion","secuencia":n,"timestamp":20,
            "origen":"usuario","rol":"otro","contenido":"comprobacion explicita","referencias":[{"tipo":"evalua","clave_evento":"pregunta/verificacion"}],
            "metadatos":{"proyecto":"p","procedencia":"real","evaluacion":{"version":1,"fuente":"usuario_explicito","procedencia":"real",
                "criterio":"prueba","criterio_version":"1","alcance":"intento","objetivo":null,"resultado":valor,
                "detalle":{"declaracion":"observacion del usuario"},"nota":"fixture","anterior":anterior}}}).to_string()).unwrap();
        s.record_execution(&Record::Evaluar {
            clave: format!("eval{n}"),
            intento: "intento".into(),
            evidencia: clave.clone(),
        })
        .unwrap();
        if n == 1 {
            s.record_task(&TaskRecord::TareaEnlazar {
                tarea: "t".into(),
                evento: clave,
                evidencia: "explicita".into(),
            })
            .unwrap();
            s.record_task(&TaskRecord::TareaResultadoDeEjecucion {
                clave: "res".into(),
                tarea: "t".into(),
                criterio: "prueba".into(),
                criterio_version: "1".into(),
                evaluacion: "eval1".into(),
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
            s.construir_episodio("ep", "p", "t", "res").unwrap();
            s.calcular_secuencia_minima("p", "manual_tarea", "ep")
                .unwrap();
            assert!(s.consultar_vigente("p", OBJETIVO).unwrap()["vigente"].is_object());
            problema(&mut s, "p", "anterior", i64::MAX, OBJETIVO);
        }
    }
    assert!(s.consultar_vigente("p", OBJETIVO).unwrap()["vigente"].is_null());
    let c = Connection::open(d.path().join("db")).unwrap();
    let id: i64 = c
        .query_row(
            "SELECT id FROM instancia WHERE clave_evento='pregunta/anterior'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(s
        .prepare_memory("historico", &peticion(id, "p", 10000))
        .unwrap()["vigente"]
        .is_object());
}

#[test]
fn rollback_y_restricciones_guardan_el_tipo_y_la_vigente_con_el_paquete() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    episodio_auto(&mut s, "p", "a", "Read", Some(true), 10, true);
    let id = problema(&mut s, "p", "actual", i64::MAX, OBJETIVO);
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON; CREATE TRIGGER falla_fixture BEFORE INSERT ON paquete_recuperacion BEGIN SELECT RAISE(ABORT,'fixture'); END").unwrap();
    assert!(s.prepare_memory("p", &peticion(id, "p", 10000)).is_err());
    for tabla in [
        "tipo_objetivo",
        "episodio_tipo_objetivo",
        "recuperacion_vigente",
        "evento_recuperacion",
    ] {
        assert_eq!(
            c.query_row(&format!("SELECT count(*) FROM {tabla}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    c.execute_batch("DROP TRIGGER falla_fixture").unwrap();
    let packet = s.prepare_memory("p", &peticion(id, "p", 10000)).unwrap();
    let clave = packet["vigente"]["tipo"]["clave"].as_str().unwrap();
    assert!(c
        .execute(
            "INSERT INTO episodio_tipo_objetivo VALUES ('manual_tarea',999,?1)",
            [clave]
        )
        .is_err());
    for tabla in [
        "tipo_objetivo",
        "episodio_tipo_objetivo",
        "recuperacion_vigente",
    ] {
        assert!(c.execute(&format!("DELETE FROM {tabla}"), []).is_err());
        let columna = if tabla == "tipo_objetivo" {
            "clave"
        } else {
            "tipo_clave"
        };
        assert!(c
            .execute(&format!("UPDATE {tabla} SET {columna}={columna}"), [])
            .is_err());
    }
    assert!(c.execute("INSERT INTO recuperacion_vigente SELECT evento_id+100,tipo_clave,derivacion_id,regla_version,texto_entregado,evidencia_json FROM recuperacion_vigente",[]).is_err());
}

#[test]
fn una_verificacion_con_fecha_futura_no_se_ofrece_al_pedido_actual() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    episodio_auto(&mut s, "p", "a", "Read", Some(true), i64::MAX, true);
    let fecha = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    let id = problema(&mut s, "p", "actual", fecha, OBJETIVO);
    assert!(s.consultar_vigente("p", OBJETIVO).unwrap()["vigente"].is_object());
    assert!(s
        .prepare_memory("sin_futuro", &peticion(id, "p", 10000))
        .unwrap()["vigente"]
        .is_null());
}

#[test]
fn cli_consulta_y_lista_los_tipos_con_su_evidencia() {
    let d = tempfile::tempdir().unwrap();
    let mut r = Registry::open(d.path()).unwrap();
    let m = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut s = r.open_memory(&m).unwrap();
    episodio_auto(&mut s, "p", "a", "Read", Some(true), 10, true);
    drop(s);
    let corre = |orden: &str, extras: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_msp"))
            .args(["--control", orden, "--memoria", &m, "--proyecto", "p"])
            .args(extras)
            .env("MSP_RAIZ", d.path())
            .output()
            .unwrap()
    };
    let consulta = corre("vigente-consultar", &["--objetivo", OBJETIVO]);
    assert!(
        consulta.status.success(),
        "{}",
        String::from_utf8_lossy(&consulta.stderr)
    );
    let valor: Value = serde_json::from_slice(&consulta.stdout).unwrap();
    assert_eq!(valor["vigente"]["camino"]["exitos"], 1);
    assert_eq!(
        consulta.stdout,
        corre("vigente-consultar", &["--objetivo", OBJETIVO]).stdout
    );
    let lista = corre("objetivos-listar", &[]);
    assert!(lista.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&lista.stdout).unwrap()["tipos"],
        json!([valor])
    );
    let vacia = corre("vigente-consultar", &["--objetivo", "!!!"]);
    assert!(vacia.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&vacia.stdout).unwrap()["tipo"],
        Value::Null
    );
    let sin_objetivo = corre("vigente-consultar", &[]);
    assert_eq!(sin_objetivo.status.code(), Some(1));
    assert!(sin_objetivo.stdout.is_empty());
}

#[test]
fn migrar_009_a_010_conserva_datos_y_hashes_y_verifica_la_migracion_nueva() {
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
        include_str!("../migrations/008_cortes_automaticos.sql"),
        include_str!("../migrations/009_secuencias_minimas.sql"),
    ];
    for (n, sql) in migraciones.iter().enumerate() {
        c.execute_batch(sql).unwrap();
        c.execute(
            "INSERT INTO schema_migrations VALUES (?1,?2,13)",
            params![
                n as i64 + 1,
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
    assert_eq!(
        c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    for (n, sql) in migraciones.iter().enumerate() {
        assert_eq!(
            c.query_row(
                "SELECT sha256,aplicado_en FROM schema_migrations WHERE version=?1",
                [n as i64 + 1],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
            (
                format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes())),
                13
            )
        );
    }
    assert_eq!(s.listar_tipos_objetivo("p").unwrap()["tipos"], json!([]));
    drop(s);
    Store::open(&path).unwrap();
    c.execute(
        "UPDATE schema_migrations SET sha256='alterado' WHERE version=10",
        [],
    )
    .unwrap();
    assert!(Store::open(&path).is_err());
}

// ---------- Entrega 6: competencia, archivo y alerta temprana ----------

fn hash_vigente(v: &Value) -> String {
    v["vigente"]["camino"]["hash_minima"]
        .as_str()
        .unwrap()
        .to_string()
}
fn camino<'a>(v: &'a Value, hash: &str) -> &'a Value {
    v["caminos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["hash_minima"] == hash)
        .unwrap()
}
fn estados(v: &Value, hash: &str) -> Vec<String> {
    v["archivo"]["historial"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["hash_minima"] == hash)
        .map(|f| f["estado"].as_str().unwrap().to_string())
        .collect()
}

/// Read falla una vez (neto 0) y Write junta `exitos`: con K = 2 hacen falta tres para relevarlo.
fn read_falla_y_write_acierta(s: &mut Store, exitos: i64) {
    episodio_auto(s, "p", "rf", "Read", Some(false), 15, false);
    for n in 0..exitos {
        episodio_auto(s, "p", &format!("w{n}"), "Write", Some(true), 20 + n, true);
    }
}

#[test]
fn la_retadora_desplaza_la_vieja_se_archiva_con_sus_vinculos_y_vuelve_por_evidencia() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    episodio_auto(&mut s, "p", "r0", "Read", Some(true), 10, true);
    let pregunta = problema(&mut s, "p", "pregunta1", i64::MAX, OBJETIVO);
    let paquete = s
        .prepare_memory("uno", &peticion(pregunta, "p", 10000))
        .unwrap();
    let read = hash_vigente(&paquete["vigente"]);
    assert_eq!(
        paquete["vigente"]["regla_vigente"],
        "competencia-ventana-v1"
    );
    assert_eq!(paquete["vigente"]["vigente"]["camino"]["estado"], "vigente");
    // Con un fallo de la vigente, K de ventaja todavía no alcanza.
    read_falla_y_write_acierta(&mut s, 2);
    let antes = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(hash_vigente(&antes), read);
    assert_eq!(estados(&antes, &read), ["vigente"]);
    assert_eq!(antes["archivo"]["historial"].as_array().unwrap().len(), 1);
    assert_eq!(antes["archivo"]["vinculos_migrados"], json!([]));
    episodio_auto(&mut s, "p", "w2", "Write", Some(true), 22, true);
    let despues = s.consultar_vigente("p", OBJETIVO).unwrap();
    let write = hash_vigente(&despues);
    assert_ne!(write, read);
    assert_eq!(despues["vigente"]["pasos"][0]["herramienta"], "Write");
    let vieja = camino(&despues, &read);
    assert_eq!(vieja["estado"], "archivado");
    assert_eq!(vieja["sucesor"], write);
    assert_eq!(vieja["exitos"], 1);
    assert_eq!(
        (vieja["fallos"].as_i64(), vieja["neto"].as_i64()),
        (Some(1), Some(0))
    );
    assert_eq!(camino(&despues, &write)["exitos"], 3);
    assert_eq!(camino(&despues, &write)["neto"], 3);
    assert_eq!(estados(&despues, &read), ["vigente", "archivado"]);
    assert_eq!(estados(&despues, &write), ["vigente"]);
    assert_eq!(despues["archivo"]["historial"][1]["sucesor_hash"], write);
    let vinculos = despues["archivo"]["vinculos_migrados"].as_array().unwrap();
    assert_eq!(vinculos.len(), 1);
    assert_eq!(vinculos[0]["evento_id"], paquete["evento_recuperacion_id"]);
    assert_eq!(vinculos[0]["sucesor_hash"], write);
    assert_eq!(
        vinculos[0]["sucesor_derivacion_id"],
        despues["vigente"]["camino"]["derivacion_id"]
    );
    // Consultar de nuevo no agrega historia.
    assert_eq!(s.consultar_vigente("p", OBJETIVO).unwrap(), despues);
    // El paquete preparado no cambia y el corte histórico decide igual que entonces.
    assert_eq!(
        s.prepare_memory("uno", &peticion(pregunta, "p", 10000))
            .unwrap(),
        paquete
    );
    let historico = s
        .prepare_memory("uno_bis", &peticion(pregunta, "p", 10000))
        .unwrap();
    assert_eq!(historico["vigente"], paquete["vigente"]);
    let nueva = problema(&mut s, "p", "pregunta2", i64::MAX, OBJETIVO);
    let dos = s
        .prepare_memory("dos", &peticion(nueva, "p", 10000))
        .unwrap();
    assert_eq!(hash_vigente(&dos["vigente"]), write);

    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let tipo = despues["tipo"]["clave"].as_str().unwrap();
    let insertar = "INSERT INTO camino_estado(tipo_clave,hash_minima,estado,sucesor_hash,regla_version,evidencia_json,registrado_en) VALUES (?1,?2,?3,?4,'x','{}',1)";
    // La entrega «dos» ofreció Write: archivarlo sin migrar ese vínculo se rechaza.
    let error = c
        .execute(insertar, params![tipo, write, "archivado", read])
        .unwrap_err()
        .to_string();
    assert!(error.contains("migrar los vinculos"), "{error}");
    for (hash, estado, sucesor) in [
        ("f".repeat(64), "vigente", None),
        (write.clone(), "archivado", Some("f".repeat(64))),
        (write.clone(), "archivado", None),
        (write.clone(), "vigente", Some(read.clone())),
        (write.clone(), "archivado", Some(write.clone())),
        (write.clone(), "borrado", None),
    ] {
        assert!(c
            .execute(insertar, params![tipo, hash, estado, sucesor])
            .is_err());
    }
    let vincular = "INSERT INTO camino_vinculo_migrado VALUES (?1,?2,?3,1)";
    let evento = dos["evento_recuperacion_id"].as_i64().unwrap();
    let derivacion = |hash: &str| camino(&despues, hash)["derivacion_id"].as_i64().unwrap();
    // Sucesor igual al camino entregado, hash que no es el de la derivación, o entrega inexistente.
    assert!(c
        .execute(vincular, params![evento, write, derivacion(&write)])
        .is_err());
    assert!(c
        .execute(vincular, params![evento, read, derivacion(&write)])
        .is_err());
    assert!(c
        .execute(vincular, params![9999, read, derivacion(&read)])
        .is_err());
    // Ataque M8: un INSERT OR REPLACE no dispara los triggers de DELETE.
    let leer = "SELECT estado,(SELECT min(registrado_en) FROM camino_vinculo_migrado) FROM camino_estado WHERE id=2";
    let fila = |c: &Connection| -> (String, i64) {
        c.query_row(leer, [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
    };
    let intacto = fila(&c);
    assert!(c
        .execute("INSERT OR REPLACE INTO camino_estado(id,tipo_clave,hash_minima,estado,sucesor_hash,regla_version,evidencia_json,registrado_en) VALUES (2,?1,?2,'vigente',NULL,'x','{}',1)", params![tipo, read])
        .is_err());
    c.execute(
        "INSERT OR REPLACE INTO camino_vinculo_migrado VALUES (?1,?2,?3,1)",
        params![
            paquete["evento_recuperacion_id"].as_i64(),
            write,
            derivacion(&write)
        ],
    )
    .unwrap();
    assert_eq!(fila(&c), intacto);
    assert_eq!(intacto.0, "archivado");
    assert_ne!(intacto.1, 1);
    for tabla in ["camino_estado", "camino_vinculo_migrado"] {
        assert!(c.execute(&format!("DELETE FROM {tabla}"), []).is_err());
        assert!(c
            .execute(
                &format!("UPDATE {tabla} SET registrado_en=registrado_en"),
                []
            )
            .is_err());
    }

    // La nueva vigente falla, la archivada junta más de K de ventaja y vuelve; la otra
    // pasa al archivo. Nada se borró.
    episodio_auto(&mut s, "p", "wf", "Write", Some(false), 29, false);
    assert_eq!(
        hash_vigente(&s.consultar_vigente("p", OBJETIVO).unwrap()),
        write
    );
    for n in 0..5 {
        episodio_auto(
            &mut s,
            "p",
            &format!("r{}", n + 1),
            "Read",
            Some(true),
            30 + n,
            true,
        );
    }
    let vuelta = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(hash_vigente(&vuelta), read);
    assert_eq!(camino(&vuelta, &write)["estado"], "archivado");
    assert_eq!(camino(&vuelta, &write)["sucesor"], read);
    assert_eq!(camino(&vuelta, &read)["sucesor"], Value::Null);
    assert_eq!(estados(&vuelta, &read), ["vigente", "archivado", "vigente"]);
    assert_eq!(estados(&vuelta, &write), ["vigente", "archivado"]);
    let migrados: Vec<_> = vuelta["archivo"]["vinculos_migrados"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["evento_id"].as_i64().unwrap(), v["sucesor_hash"].clone()))
        .collect();
    assert_eq!(
        migrados,
        [
            (
                paquete["evento_recuperacion_id"].as_i64().unwrap(),
                json!(write)
            ),
            (
                historico["evento_recuperacion_id"].as_i64().unwrap(),
                json!(write)
            ),
            (evento, json!(read)),
        ]
    );
    assert_eq!(s.memory_packet("p", "uno").unwrap(), paquete);
    assert_eq!(s.memory_packet("p", "dos").unwrap(), dos);
    assert_eq!(
        c.query_row("SELECT count(*) FROM camino_estado_actual", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn atribuye_fallos_solo_con_la_misma_secuencia_y_avisa_si_la_vigente_falla_intermitente() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    let serie = [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 1, 0, 1, 1, 1];
    for (n, ok) in serie.into_iter().enumerate() {
        let ok = ok == 1;
        episodio_auto(
            &mut s,
            "p",
            &format!("r{n}"),
            "Read",
            Some(ok),
            n as i64,
            ok,
        );
    }
    // Un fallo con otra secuencia, sin ningún éxito de ese camino, queda en el tipo.
    episodio_auto(&mut s, "p", "otro", "Edit", Some(false), 50, false);
    let v = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(v["vigente"]["camino"]["exitos"], 17);
    assert_eq!(v["vigente"]["camino"]["fallos"], 3);
    assert_eq!(v["vigente"]["camino"]["neto"], 14);
    assert_eq!(v["vigente"]["camino"]["estado"], "vigente");
    assert_eq!(v["vigente"]["en_falla"], false);
    assert_eq!(v["evidencia_tipo"]["fallos"], 4);
    assert_eq!(v["evidencia_tipo"]["fallos_atribuidos"], 3);
    assert_eq!(v["caminos"].as_array().unwrap().len(), 1);
    let alerta = &v["vigente"]["alerta"];
    assert_eq!(alerta["nivel"], "varianza");
    assert_eq!(
        alerta["regla_version"],
        "alerta-varianza-autocorrelacion-v2"
    );
    assert_eq!(alerta["varianza_anterior"], 0.0);
    assert!((alerta["varianza_reciente"].as_f64().unwrap() - 0.21).abs() < 1e-12);
    // La alerta viaja también en el bloque entregado.
    let id = problema(&mut s, "p", "actual", i64::MAX, OBJETIVO);
    let paquete = s
        .prepare_memory("alerta", &peticion(id, "p", 10000))
        .unwrap();
    assert_eq!(paquete["vigente"]["vigente"]["alerta"], *alerta);
    assert!(paquete["texto_memoria"]
        .as_str()
        .unwrap()
        .contains("\"nivel\":\"varianza\""));
}

// Ataques M6 y M1. El éxito se contaba por la mínima y el fallo por la secuencia completa:
// los fallos de la misma secuencia que el éxito no se atribuían, y un camino con un
// éxito y muchos fallos se seguía ofreciendo al host con alerta «ninguna».
#[test]
fn regresion_los_fallos_van_al_camino_de_su_misma_secuencia_y_una_vigente_rota_no_se_ofrece() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    let exito = episodio_auto(&mut s, "p", "ok", "Write,Write", Some(true), 1, true);
    let id = problema(&mut s, "p", "antes", i64::MAX, OBJETIVO);
    let sano = s.prepare_memory("sano", &peticion(id, "p", 10000)).unwrap();
    assert_eq!(sano["vigente"]["vigente"]["en_falla"], false);
    for n in 0..5 {
        let chat = format!("mal{n}");
        let fallo = episodio_auto(
            &mut s,
            "p",
            &chat,
            "Write,Write",
            Some(false),
            10 + n,
            false,
        );
        assert_eq!(fallo["secuencia_sha256"], exito["secuencia_sha256"]);
    }
    // Otro episodio, más corto, que falla: su secuencia no es la de ningún éxito.
    episodio_auto(&mut s, "p", "corto", "Write", Some(false), 30, false);
    let v = s.consultar_vigente("p", OBJETIVO).unwrap();
    let camino = &v["vigente"]["camino"];
    assert_ne!(camino["hash_minima"], exito["secuencia_sha256"]);
    assert_eq!(
        (camino["exitos"].as_i64(), camino["fallos"].as_i64()),
        (Some(1), Some(5))
    );
    assert_eq!(camino["neto"], -4);
    assert_eq!(v["evidencia_tipo"]["fallos"], 6);
    assert_eq!(v["evidencia_tipo"]["fallos_atribuidos"], 5);
    // Sigue vigente porque no hay otra, pero rota: se dice y no se le ofrece al host.
    assert_eq!(camino["estado"], "vigente");
    assert_eq!(v["vigente"]["en_falla"], true);
    assert_eq!(v["vigente"]["alerta"]["nivel"], "falla_sostenida");
    let id = problema(&mut s, "p", "despues", i64::MAX, OBJETIVO);
    let roto = s.prepare_memory("roto", &peticion(id, "p", 10000)).unwrap();
    assert!(roto["vigente"].is_null(), "{}", roto["vigente"]);
    // El paquete preparado cuando estaba sana no cambia.
    assert_eq!(s.memory_packet("p", "sano").unwrap(), sano);
}

// Ataque M9: la entrega de un camino archivado quedaba redirigida a un sucesor que
// después también se archivó, y a ninguna vigente.
#[test]
fn regresion_los_vinculos_heredados_migran_cuando_el_sucesor_tambien_se_archiva() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    episodio_auto(&mut s, "p", "r0", "Read", Some(true), 10, true);
    let id = problema(&mut s, "p", "pregunta", i64::MAX, OBJETIVO);
    let entrega = s.prepare_memory("uno", &peticion(id, "p", 10000)).unwrap();
    let read = hash_vigente(&entrega["vigente"]);
    read_falla_y_write_acierta(&mut s, 3);
    let write = hash_vigente(&s.consultar_vigente("p", OBJETIVO).unwrap());
    episodio_auto(&mut s, "p", "wf", "Write", Some(false), 29, false);
    for n in 0..5 {
        episodio_auto(
            &mut s,
            "p",
            &format!("e{n}"),
            "Edit",
            Some(true),
            40 + n,
            true,
        );
    }
    let v = s.consultar_vigente("p", OBJETIVO).unwrap();
    let edit = hash_vigente(&v);
    assert!(edit != read && edit != write);
    assert_eq!(camino(&v, &read)["sucesor"], write);
    assert_eq!(camino(&v, &write)["sucesor"], edit);
    let destinos: Vec<&Value> = v["archivo"]["vinculos_migrados"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|g| g["evento_id"] == entrega["evento_recuperacion_id"])
        .map(|g| &g["sucesor_hash"])
        .collect();
    assert_eq!(destinos.len(), 2);
    assert!(destinos.contains(&&json!(write)) && destinos.contains(&&json!(edit)));
    assert_eq!(s.consultar_vigente("p", OBJETIVO).unwrap(), v);
}

#[test]
fn un_camino_que_pierde_su_exito_por_una_correccion_vuelve_a_candidato_sin_borrarse() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    manual(&mut s);
    let antes = s.consultar_vigente("p", OBJETIVO).unwrap();
    let hash = hash_vigente(&antes);
    resultado_manual(&mut s, "corregido", Some(0), Some("res"));
    let despues = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert!(despues["vigente"].is_null());
    assert_eq!(estados(&despues, &hash), ["vigente", "candidato"]);
    assert_eq!(s.consultar_vigente("p", OBJETIVO).unwrap(), despues);
}

#[test]
fn migrar_011_a_012_conserva_las_vigentes_entregadas_y_admite_la_regla_nueva() {
    use sha2::{Digest, Sha256};
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,sha256 TEXT NOT NULL,aplicado_en INTEGER NOT NULL)").unwrap();
    let migraciones = [
        include_str!("../migrations/001_erd.sql"),
        include_str!("../migrations/002_captura_tecnica.sql"),
        include_str!("../migrations/003_recuperacion_textual.sql"),
        include_str!("../migrations/004_ejecucion_evaluacion.sql"),
        include_str!("../migrations/005_entrega_contexto_host.sql"),
        include_str!("../migrations/006_tareas.sql"),
        include_str!("../migrations/007_episodios.sql"),
        include_str!("../migrations/008_cortes_automaticos.sql"),
        include_str!("../migrations/009_secuencias_minimas.sql"),
        include_str!("../migrations/010_objetivos_y_vigente.sql"),
        include_str!("../migrations/011_arandu_secuencias.sql"),
    ];
    let sha = |sql: &str| format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()));
    for (n, sql) in migraciones.iter().enumerate() {
        c.execute_batch(sql).unwrap();
        c.execute(
            "INSERT INTO schema_migrations VALUES (?1,?2,13)",
            params![n as i64 + 1, sha(sql)],
        )
        .unwrap();
    }
    // Una entrega de la regla anterior. Sus fuentes no importan acá: se apagan las
    // claves foráneas y los dos triggers de validación sólo mientras se arma la fila.
    c.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    let clave = "a".repeat(64);
    let mut repuestos = Vec::new();
    for nombre in ["secuencia_minima_fuente", "recuperacion_vigente_validar"] {
        repuestos.push(
            c.query_row(
                "SELECT sql FROM sqlite_master WHERE name=?1",
                [nombre],
                |r| r.get::<_, String>(0),
            )
            .unwrap(),
        );
        c.execute_batch(&format!("DROP TRIGGER {nombre}")).unwrap();
    }
    c.execute(
        "INSERT INTO tipo_objetivo VALUES (?1,'p','objetivo-terminos-v1','[\"x\"]')",
        [&clave],
    )
    .unwrap();
    c.execute("INSERT INTO evento_recuperacion(id,tarea_hash,clave_evento,problema_instancia_id,configuracion_id,timestamp) VALUES (7,'h','k',1,1,1)", []).unwrap();
    c.execute("INSERT INTO secuencia_minima VALUES (3,'automatico_lista',NULL,1,'minima-conservadora-v1','herramienta-destino-v1',1,?1,0,0,5)", [&clave]).unwrap();
    c.execute("INSERT INTO recuperacion_vigente VALUES (7,?1,3,'vigente-mayoria-reciente-v1','bloque\n','{\"a\":1}')", [&clave]).unwrap();
    c.execute_batch(&repuestos[0]).unwrap();
    drop(c);

    Store::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    for (n, sql) in migraciones.iter().enumerate() {
        assert_eq!(
            c.query_row(
                "SELECT sha256,aplicado_en FROM schema_migrations WHERE version=?1",
                [n as i64 + 1],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            )
            .unwrap(),
            (sha(sql), 13)
        );
    }
    let fila: (i64, String, i64, String, String, String) = c
        .query_row("SELECT * FROM recuperacion_vigente", [], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap();
    assert_eq!(
        fila,
        (
            7,
            clave.clone(),
            3,
            "vigente-mayoria-reciente-v1".into(),
            "bloque\n".into(),
            "{\"a\":1}".into()
        )
    );
    // La tabla reconstruida conserva sus cuatro guardas y no quedó la temporal.
    let nombres: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE name LIKE '%vigente%' AND type<>'index' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        nombres,
        [
            "paquete_vigente_validar",
            "recuperacion_vigente",
            "recuperacion_vigente_delete",
            "recuperacion_vigente_update",
            "recuperacion_vigente_validar",
        ]
    );
    assert!(c.execute("DELETE FROM recuperacion_vigente", []).is_err());
    assert!(c
        .execute(
            "UPDATE recuperacion_vigente SET derivacion_id=derivacion_id",
            []
        )
        .is_err());
    // Sigue validando la fuente antes de aceptar una entrega, también con la regla nueva.
    let error = c
        .execute(
            "INSERT INTO recuperacion_vigente VALUES (8,?1,3,'competencia-pliegue-v1','b','{}')",
            [&clave],
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("vigente ajena"), "{error}");
    drop(c);
    Store::open(&path).unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute(
        "UPDATE schema_migrations SET sha256='alterado' WHERE version=12",
        [],
    )
    .unwrap();
    assert!(Store::open(&path).is_err());
}

// Regresión: la entrega con corte histórico ofrecía un camino que hoy está archivado y
// quedaba sin vínculo al sucesor hasta la próxima consulta.
#[test]
fn una_entrega_historica_de_un_camino_ya_archivado_recibe_su_vinculo_al_sellarse() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("db");
    let mut s = Store::open(&path).unwrap();
    episodio_auto(&mut s, "p", "r0", "Read", Some(true), 10, true);
    let pregunta = problema(&mut s, "p", "pregunta1", i64::MAX, OBJETIVO);
    read_falla_y_write_acierta(&mut s, 3);
    let hoy = s.consultar_vigente("p", OBJETIVO).unwrap();
    let write = hash_vigente(&hoy);
    let paquete = s
        .prepare_memory("vieja", &peticion(pregunta, "p", 10000))
        .unwrap();
    let read = hash_vigente(&paquete["vigente"]);
    assert_eq!(camino(&hoy, &read)["estado"], "archivado");
    let c = Connection::open(&path).unwrap();
    let vinculo: (String, i64) = c
        .query_row(
            "SELECT sucesor_hash,sucesor_derivacion_id FROM camino_vinculo_migrado WHERE evento_id=?1",
            [paquete["evento_recuperacion_id"].as_i64()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        vinculo,
        (
            write.clone(),
            hoy["vigente"]["camino"]["derivacion_id"].as_i64().unwrap()
        )
    );
    // Una entrega sin consulta previa también deja el archivo al día.
    let mut otra = Store::open(d.path().join("db2")).unwrap();
    episodio_auto(&mut otra, "p", "r0", "Read", Some(true), 10, true);
    read_falla_y_write_acierta(&mut otra, 3);
    let nueva = problema(&mut otra, "p", "pregunta2", i64::MAX, OBJETIVO);
    let entregado = otra
        .prepare_memory("nueva", &peticion(nueva, "p", 10000))
        .unwrap();
    assert_eq!(hash_vigente(&entregado["vigente"]), write);
    let c = Connection::open(d.path().join("db2")).unwrap();
    let estados: Vec<(String, String)> = c
        .prepare("SELECT hash_minima,estado FROM camino_estado_actual ORDER BY estado")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        estados,
        [(read, "archivado".into()), (write, "vigente".into())]
    );
}

#[test]
fn cli_muestra_el_relevo_el_archivo_y_la_alerta_sin_agregar_historia_al_repetir() {
    let d = tempfile::tempdir().unwrap();
    let mut r = Registry::open(d.path()).unwrap();
    let m = r.create_memory("Prueba").unwrap()["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut s = r.open_memory(&m).unwrap();
    episodio_auto(&mut s, "p", "r0", "Read", Some(true), 10, true);
    read_falla_y_write_acierta(&mut s, 3);
    drop(s);
    let corre = |orden: &str, extras: &[&str]| {
        let salida = Command::new(env!("CARGO_BIN_EXE_msp"))
            .args(["--control", orden, "--memoria", &m, "--proyecto", "p"])
            .args(extras)
            .env("MSP_RAIZ", d.path())
            .output()
            .unwrap();
        assert!(
            salida.status.success(),
            "{}",
            String::from_utf8_lossy(&salida.stderr)
        );
        salida.stdout
    };
    let salida = corre("vigente-consultar", &["--objetivo", OBJETIVO]);
    let v: Value = serde_json::from_slice(&salida).unwrap();
    assert_eq!(v["regla_vigente"], "competencia-ventana-v1");
    assert_eq!(v["vigente"]["pasos"][0]["herramienta"], "Write");
    assert_eq!(v["vigente"]["camino"]["estado"], "vigente");
    assert_eq!(v["vigente"]["camino"]["exitos"], 3);
    assert_eq!(v["vigente"]["camino"]["fallos"], 0);
    assert_eq!(v["vigente"]["camino"]["neto"], 3);
    assert_eq!(v["vigente"]["en_falla"], false);
    assert_eq!(v["vigente"]["alerta"]["nivel"], "sin_datos");
    assert_eq!(v["vigente"]["alerta"]["ventana"], 10);
    let write = hash_vigente(&v);
    let archivados: Vec<&Value> = v["caminos"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["estado"] == "archivado")
        .collect();
    assert_eq!(archivados.len(), 1);
    assert_eq!(archivados[0]["sucesor"], write);
    assert_eq!(archivados[0]["neto"], 0);
    assert_eq!(v["archivo"]["historial"].as_array().unwrap().len(), 2);
    assert_eq!(v["archivo"]["vinculos_migrados"], json!([]));
    // Repetir la consulta o listar no agrega filas al historial.
    assert_eq!(
        corre("vigente-consultar", &["--objetivo", OBJETIVO]),
        salida
    );
    let lista: Value = serde_json::from_slice(&corre("objetivos-listar", &[])).unwrap();
    assert_eq!(lista["regla_vigente"], "competencia-ventana-v1");
    assert_eq!(lista["tipos"], json!([v]));
}

// ---------- La urna observada: lectura, no decisión ----------

fn urna_de<'a>(v: &'a Value, hash: &str) -> &'a Value {
    v["urna"]["caminos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["hash_minima"] == hash)
        .unwrap()
}

/// Regla de admisión: sólo cuenta como profundidad el éxito de un chat al que la memoria
/// no le había ofrecido ese camino. El resto mide cuánto se sigue lo ofrecido.
#[test]
fn la_urna_separa_convergencia_independiente_de_seguimiento_inducido() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    episodio_auto(&mut s, "p", "solo", "Read", Some(true), 10, true);
    // Tres chats reciben la vigente (Read): uno la sigue, otro toma otro camino, otro no hace nada.
    let ofrecer = |s: &mut Store, chat: &str| {
        let id = problema(s, "p", chat, i64::MAX, OBJETIVO);
        let paquete = s.prepare_memory(chat, &peticion(id, "p", 10000)).unwrap();
        hash_vigente(&paquete["vigente"])
    };
    let read = ofrecer(&mut s, "sigue");
    episodio_auto(&mut s, "p", "sigue", "Read", Some(true), 20, true);
    assert_eq!(ofrecer(&mut s, "otro"), read);
    episodio_auto(&mut s, "p", "otro", "Write", Some(true), 30, true);
    assert_eq!(ofrecer(&mut s, "nada"), read);

    let v = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(v["urna"]["regla"], "urna-observada-v1");
    assert_eq!(hash_vigente(&v), read);
    let propio = urna_de(&v, &read);
    // Dos éxitos de Read, pero uno fue después de ofrecerlo a ese chat.
    assert_eq!(camino(&v, &read)["exitos"], 2);
    assert_eq!(
        (&propio["profundidad"], &propio["exitos_inducidos"]),
        (&json!(1), &json!(1))
    );
    assert_eq!(propio["pasos_no_compartidos_con_vigente"], 0);
    let write = v["urna"]["caminos"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["hash_minima"] != read.as_str())
        .unwrap();
    // A «otro» se le ofreció Read, no Write: su éxito con Write es independiente.
    assert_eq!(
        (&write["profundidad"], &write["exitos_inducidos"]),
        (&json!(1), &json!(0))
    );
    assert_eq!(write["pasos_no_compartidos_con_vigente"], 1);
    // La mínima incluye la comprobación: los dos caminos comparten ese paso y difieren en uno.
    let pasos = |c: &Value| -> Vec<String> {
        c["pasos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().split('/').next().unwrap().to_string())
            .collect()
    };
    assert_eq!(pasos(write), ["Bash", "Write"]);
    assert_eq!(pasos(propio), ["Bash", "Read"]);
    let total: f64 = [propio, write]
        .iter()
        .map(|c| c["cuota"].as_f64().unwrap())
        .sum();
    assert!((total - 1.0).abs() < 1e-12);
    assert!((propio["cuota"].as_f64().unwrap() - 2.0 / 3.0).abs() < 1e-12);
    assert_eq!(propio["cuota_anterior"], Value::Null);
    assert_eq!(v["urna"]["vigente_en_desuso"], false);
    assert_eq!(
        v["urna"]["seguimiento"],
        json!({"entregas":3,"seguidas":1,"otro_camino":1,"sin_episodio":1,"p_seguir":0.5})
    );
    // Es lectura: la decisión, el archivo y lo entregado no cambian por consultarla.
    assert_eq!(s.consultar_vigente("p", OBJETIVO).unwrap(), v);
    let id = problema(&mut s, "p", "despues", i64::MAX, OBJETIVO);
    let paquete = s
        .prepare_memory("despues", &peticion(id, "p", 10000))
        .unwrap();
    assert!(paquete["vigente"]["urna"].is_null());
    assert!(!paquete["texto_memoria"].as_str().unwrap().contains("urna"));
}

/// Desintermediación: la vigente deja de recibir trayectorias sin fallar nunca. La alerta
/// de fallos no lo ve y la regla no la releva; la urna sí lo muestra.
#[test]
fn la_urna_ve_a_una_vigente_que_pierde_trafico_sin_fallar() {
    let d = tempfile::tempdir().unwrap();
    let mut s = Store::open(d.path().join("db")).unwrap();
    for n in 0..20 {
        episodio_auto(&mut s, "p", &format!("r{n}"), "Read", Some(true), n, true);
    }
    let antes = s.consultar_vigente("p", OBJETIVO).unwrap();
    let read = hash_vigente(&antes);
    assert_eq!(antes["urna"]["vigente_en_desuso"], false);
    assert_eq!(urna_de(&antes, &read)["cuota_reciente"], 1.0);
    for n in 0..20 {
        episodio_auto(
            &mut s,
            "p",
            &format!("w{n}"),
            "Write",
            Some(true),
            100 + n,
            true,
        );
        let v = s.consultar_vigente("p", OBJETIVO).unwrap();
        // Hasta completar dos tramos del tipo no hay cuota anterior con qué comparar.
        assert_eq!(v["urna"]["vigente_en_desuso"], n == 19, "{n}");
    }
    let v = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(hash_vigente(&v), read);
    assert_eq!(v["vigente"]["alerta"]["nivel"], "ninguna");
    assert_eq!(v["vigente"]["en_falla"], false);
    let propio = urna_de(&v, &read);
    assert_eq!(
        (&propio["cuota_reciente"], &propio["cuota_anterior"]),
        (&json!(0.0), &json!(1.0))
    );
    assert_eq!(propio["cuota"], 0.5);
    assert_eq!(v["urna"]["vigente_en_desuso"], true);
    assert_eq!(v["urna"]["tramo"], 20);
    // Si además falla, deja de ser desuso: es una vigente que falla, y eso ya se ve.
    for n in 0..2 {
        episodio_auto(
            &mut s,
            "p",
            &format!("rf{n}"),
            "Read",
            Some(false),
            200 + n,
            false,
        );
    }
    let v = s.consultar_vigente("p", OBJETIVO).unwrap();
    assert_eq!(v["urna"]["vigente_en_desuso"], false);
}
