//! Tanda 5: tareas medibles, modos, informes, instantáneas y corridas aisladas.
use msp::adapter::Adapter;
use msp::control::Registry;
use msp::measure::{
    crear_snapshot, preparar_corridas, ruta_manifiesto, tabla_comparativa, validar_snapshot,
};
use msp::tasks::{Criterio, TaskRecord};
use msp::Store;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Entorno {
    _dir: tempfile::TempDir,
    raiz: PathBuf,
    datos: PathBuf,
    proyecto: PathBuf,
}

fn entorno() -> Entorno {
    let dir = tempfile::tempdir().unwrap();
    let datos = dir.path().join("datos de medicion");
    let proyecto = dir.path().join("proyecto ñ");
    std::fs::create_dir_all(&datos).unwrap();
    std::fs::create_dir_all(&proyecto).unwrap();
    Entorno {
        raiz: dir.path().to_path_buf(),
        _dir: dir,
        datos,
        proyecto,
    }
}

impl Entorno {
    fn registro(&self) -> Registry {
        Registry::open(&self.datos).unwrap()
    }
    fn adaptador(&self) -> Adapter {
        Adapter::new(self.registro(), "inst".into(), 4096)
    }
    fn proyecto_clave(&self) -> String {
        self.registro().register_project(&self.proyecto).unwrap()["raiz_normalizada"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn memoria(&self, nombre: &str) -> String {
        self.registro().create_memory(nombre).unwrap()["memoria_id"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn archivo(&self, id: &str) -> PathBuf {
        self.datos.join("memorias").join(format!("{id}.sqlite"))
    }
}

fn criterios(obligatorio: bool) -> Vec<Criterio> {
    vec![
        Criterio {
            id: "c-tests".into(),
            version: "1".into(),
            descripcion: "la suite pasa".into(),
            obligatorio: true,
        },
        Criterio {
            id: "c-docs".into(),
            version: "1".into(),
            descripcion: "README actualizado".into(),
            obligatorio,
        },
    ]
}

fn crear_tarea(store: &mut Store, clave: &str, proyecto: &str) -> Value {
    store
        .record_task(&TaskRecord::TareaCrear {
            tarea: clave.into(),
            proyecto: proyecto.into(),
            definicion: format!("{clave}/def/1"),
            descripcion: "arreglar el login".into(),
            criterios: criterios(false),
        })
        .unwrap()
}

fn hook(evento: &str, sesion: &str, cwd: &Path, extra: Value) -> Value {
    let mut base = json!({"hook_event_name":evento,"session_id":sesion,"cwd":cwd,
        "transcript_path":cwd.join("t.jsonl"),"permission_mode":"default"});
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
    base
}

fn prompt(sesion: &str, cwd: &Path, id: &str, texto: &str) -> Value {
    hook(
        "UserPromptSubmit",
        sesion,
        cwd,
        json!({"prompt_id": id, "prompt": texto}),
    )
}

/// Evidencia externa dedicada con el contrato de tanda 3, alcance de tarea.
fn evidencia(
    clave: &str,
    seq: i64,
    proyecto: &str,
    tarea: &str,
    criterio: &str,
    resultado: Option<i64>,
) -> String {
    let fuente = if resultado.is_some() {
        "verificador_externo"
    } else {
        "usuario_explicito"
    };
    let origen = if resultado.is_some() {
        "herramienta"
    } else {
        "usuario"
    };
    let detalle = match resultado {
        Some(r) => json!({"nombre":"suite-local","version":"1","tarea":tarea,
            "entrada":{"comando":"cargo test"},"salida":{"cumple":r==1},
            "regla":{"campo":"/cumple","si_true":1,"si_false":0},"ejecucion":"real_local"}),
        None => json!({"tarea":tarea,"declaracion":"el usuario no sabe todavia"}),
    };
    json!({"version":1,"clave_evento":clave,"grupo_evidencia":"verificaciones","secuencia":seq,
        "origen":origen,"rol":"otro","timestamp":1_800_000_000_000_000i64+seq,"contenido":"verificacion",
        "metadatos":{"proyecto":proyecto,"procedencia":"real","evaluacion":{
            "version":1,"fuente":fuente,"procedencia":"real","criterio":criterio,
            "criterio_version":"1","alcance":"tarea","objetivo":Value::Null,"resultado":resultado,
            "detalle":detalle,"nota":"verificacion local","anterior":Value::Null}}})
    .to_string()
}

fn resultado(
    clave: &str,
    tarea: &str,
    criterio: &str,
    evidencia: &str,
    anterior: Option<&str>,
) -> TaskRecord {
    TaskRecord::TareaResultado {
        clave: clave.into(),
        tarea: tarea.into(),
        criterio: criterio.into(),
        criterio_version: "1".into(),
        evidencia: evidencia.into(),
        alcance: "tarea".into(),
        objetivo: None,
        anterior: anterior.map(str::to_string),
    }
}

fn integro(path: &Path) {
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    c.execute_batch("PRAGMA foreign_key_check").unwrap();
}

#[test]
fn tabla_compara_el_estado_del_codigo_entre_condiciones_del_mismo_caso() {
    let e = entorno();
    let mut corridas = Vec::new();
    for (condicion, commit) in [
        ("sin_recuperacion", "abc"),
        ("memoria_inicial", "abc"),
        ("memoria_acumulada", "def"),
    ] {
        let dir = e.raiz.join("caso").join(condicion).join("r1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("informe.json"),
            json!({"modos_observados":["solo_registro"],"calidad":{}}).to_string(),
        )
        .unwrap();
        corridas.push(
            json!({"caso":{"id":"caso","criterios":[{"id":"c","version":"1"}],
            "estado_inicial_codigo":{"commit":commit},"configuracion":{"modelo":"modelo-prueba"},
            "exposicion_previa":false},"condicion":{"nombre":condicion},"repeticion":1,
            "antecedentes_elegibles":true}),
        );
    }
    std::fs::write(
        e.raiz.join("indice.json"),
        json!({"corridas":corridas}).to_string(),
    )
    .unwrap();
    let tabla = tabla_comparativa(&e.raiz).unwrap();
    assert_eq!(tabla["filas"][0]["comparable"], true);
    assert_eq!(tabla["filas"][1]["comparable"], true);
    assert_eq!(tabla["filas"][2]["comparable"], false);
    assert_eq!(
        tabla["filas"][2]["motivos_no_comparable"],
        json!(["estado inicial del codigo distinto al de otra condicion"])
    );
}

#[test]
fn ventanas_de_tiempo_unen_solapamientos_y_dejan_visible_la_abierta() {
    let e = entorno();
    let mut s = Store::open(e.raiz.join("tiempo.sqlite")).unwrap();
    crear_tarea(&mut s, "t/x", "p");
    let vacio = s.task_report("p", "t/x").unwrap();
    assert_eq!(vacio["tiempo"]["ventanas_cerradas"], json!([]));
    assert_eq!(vacio["tiempo"]["intervalo_abierto_desde"], Value::Null);
    assert_eq!(vacio["tiempo"]["ventana_observada_total"], 0);
    let mut marcar = |tipo: &str, clave: &str| {
        std::thread::sleep(std::time::Duration::from_millis(1));
        s.record_task(&TaskRecord::TareaMarcar {
            clave: clave.into(),
            tarea: "t/x".into(),
            tipo: tipo.into(),
            detalle: json!({}),
        })
        .unwrap()["registrado_en"]
            .as_i64()
            .unwrap()
    };
    // Un cierre suelto no consume un inicio futuro. Dos chats se solapan.
    marcar("pausa", "suelta");
    let a = marcar("inicio", "a");
    marcar("inicio", "b");
    marcar("pausa", "c");
    let d = marcar("cierre", "d");
    marcar("cierre", "suelto");
    let f = marcar("reanudacion", "f");
    let g = marcar("pausa", "g");
    let h = marcar("reanudacion", "h");
    let informe = s.task_report("p", "t/x").unwrap();
    assert_eq!(
        informe["tiempo"]["ventanas_cerradas"],
        json!([
        {"desde":a,"hasta":d}, {"desde":f,"hasta":g}])
    );
    assert_eq!(
        informe["tiempo"]["ventana_observada_total"],
        (d - a) + (g - f)
    );
    assert_eq!(informe["tiempo"]["intervalo_abierto_desde"], h);
}

#[test]
fn herramientas_sin_id_declaran_cobertura_parcial() {
    let e = entorno();
    let mut s = Store::open(e.raiz.join("sin-id.sqlite")).unwrap();
    crear_tarea(&mut s, "t/x", "p");
    let crudo = json!({"version":1,"clave_evento":"herramienta","grupo_evidencia":"g",
        "secuencia":1,"timestamp":1,"origen":"herramienta","rol":"otro","contenido":"ok",
        "metadatos":{"proyecto":"p","tipo":"herramienta-ok"}});
    s.capture(&crudo.to_string()).unwrap();
    s.record_task(&TaskRecord::TareaEnlazar {
        tarea: "t/x".into(),
        evento: "herramienta".into(),
        evidencia: "enlace explicito".into(),
    })
    .unwrap();
    let informe = s.task_report("p", "t/x").unwrap();
    assert_eq!(
        informe["herramientas"]["observaciones_sin_identidad_de_operacion"],
        1
    );
    assert_eq!(informe["herramientas"]["cobertura"], "parcial");
    assert_eq!(informe["herramientas"]["operaciones"], json!([]));
}

#[test]
fn controles_del_chat_seleccionan_la_tarea_exacta_y_quitan_el_predeterminado() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("Memoria de pruebas");
    let mut ad = e.adaptador();
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    let mut s = ad.registry.open_memory(&m).unwrap();
    crear_tarea(&mut s, "t/a", &proyecto);
    crear_tarea(&mut s, "t/b", &proyecto);
    s.record_task(&TaskRecord::TareaRevisar {
        tarea: "t/b".into(),
        definicion: "t/b/def/2".into(),
        descripcion: "criterios revisados".into(),
        criterios: criterios(false),
    })
    .unwrap();
    let orden = |texto: &str| prompt("c1", &e.proyecto, "control", texto);
    let lista = ad.hook(&orden("msp:: tareas")).unwrap().contexto.unwrap();
    assert!(lista.contains("t/a") && lista.contains("t/b"), "{lista}");
    let seleccion = ad
        .hook(&orden("msp:: tarea t/b"))
        .unwrap()
        .contexto
        .unwrap();
    assert!(!seleccion.contains("\"error\""), "{seleccion}");
    let estado = ad.registry.status("inst", "c1", None).unwrap();
    assert_eq!(estado["tarea"]["clave"], "t/b");
    assert_eq!(estado["tarea"]["definicion_version"], 2);
    let antes = ad
        .registry
        .gate("inst", "c1", Some(&e.proyecto))
        .unwrap()
        .unwrap();
    // Volver a seleccionar la misma tarea invalida los callbacks anteriores.
    ad.hook(&orden("msp:: tarea t/b")).unwrap();
    assert!(!ad.registry.task_still_selected(&antes).unwrap());
    let desconocida = ad
        .hook(&orden("msp:: tarea inexistente"))
        .unwrap()
        .contexto
        .unwrap();
    assert!(desconocida.contains("ausente"), "{desconocida}");
    ad.hook(&orden("msp:: tarea quitar")).unwrap();
    assert!(ad
        .registry
        .gate("inst", "c1", Some(&e.proyecto))
        .unwrap()
        .unwrap()
        .tarea
        .is_none());
    let poner = format!("msp:: predeterminado {m}");
    ad.hook(&orden(&poner)).unwrap();
    assert_eq!(
        ad.registry
            .init_session("inst", "con-predeterminado", &e.proyecto)
            .unwrap()["estado"],
        "encendida"
    );
    let quitar = ad
        .hook(&orden("msp:: predeterminado quitar"))
        .unwrap()
        .contexto
        .unwrap();
    assert!(!quitar.contains("\"error\""), "{quitar}");
    assert_eq!(
        ad.registry
            .init_session("inst", "nueva", &e.proyecto)
            .unwrap()["estado"],
        "apagada"
    );
}

#[test]
fn informes_distinguen_entregas_completas_inciertas_y_presupuesto_vacio() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    crear_tarea(&mut ad.registry.open_memory(&m).unwrap(), "t/x", &proyecto);
    ad.registry
        .use_memory("inst", "antecedente", &e.proyecto, &m)
        .unwrap();
    ad.hook(&prompt(
        "antecedente",
        &e.proyecto,
        "a",
        "aguja antecedente",
    ))
    .unwrap();
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    ad.registry
        .select_task("inst", "c1", Some("t/x"), Some(1))
        .unwrap();
    for (i, confirmar) in [(1, true), (2, true), (3, false)] {
        let salida = ad
            .hook(&prompt("c1", &e.proyecto, &format!("p{i}"), "aguja"))
            .unwrap();
        assert_eq!(salida.informe["entrega"]["estado"], "preparada");
        assert!(salida.informe["entrega"]["seleccionados"].as_u64().unwrap() > 0);
        assert_eq!(
            salida.informe["entrega"]["bytes_salida"],
            salida.contexto.as_ref().unwrap().len()
        );
        if confirmar {
            ad.confirmar_escritura(&salida, "replay_offline").unwrap();
        }
    }
    let informe = ad
        .registry
        .open_memory(&m)
        .unwrap()
        .task_report(&proyecto, "t/x")
        .unwrap();
    assert_eq!(informe["memoria"]["entregas_escritura_completada"], 2);
    assert_eq!(informe["memoria"]["entregas_inciertas"], 1);
    // Cada mitad del OR es suficiente por sí sola para no preparar un paquete.
    let salida = ad.hook(&prompt("c1", &e.proyecto, "vacio", "!!!")).unwrap();
    assert_eq!(
        salida.informe["entrega"]["motivo"],
        "la consulta literal quedo vacia"
    );
    assert!(salida.informe["entrega"]["clave_paquete"].is_null());
    let mut chico = Adapter::new(e.registro(), "inst".into(), 0);
    let salida = chico
        .hook(&prompt("c1", &e.proyecto, "sin-presupuesto", "aguja"))
        .unwrap();
    assert_eq!(
        salida.informe["entrega"]["motivo"],
        "el sobre no cabe en el presupuesto de integracion"
    );
    assert!(salida.informe["entrega"]["clave_paquete"].is_null());
}

#[test]
fn session_end_cierra_el_intervalo_y_lo_informa_sin_capturar() {
    let e = entorno();
    let m = e.memoria("A");
    e.proyecto_clave();
    let mut ad = e.adaptador();
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    let salida = ad
        .hook(&hook(
            "SessionEnd",
            "c1",
            &e.proyecto,
            json!({"reason":"salida"}),
        ))
        .unwrap();
    assert_eq!(
        salida.informe,
        json!({"hook":"SessionEnd","accion":"intervalo_cerrado"})
    );
    assert!(salida.contexto.is_none());
    let c = Connection::open(e.datos.join("registro.sqlite")).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM intervalo_captura WHERE estado='abierto'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM asignacion_evento", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

// ---------------------------------------------------------------------------

#[test]
fn una_tarea_reune_varios_chats_sin_cambiar_identidades_ni_inventar_correlacion() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    crear_tarea(
        &mut ad.registry.open_memory(&m).unwrap(),
        "t/login",
        &proyecto,
    );

    for sesion in ["c1", "c2"] {
        ad.registry
            .use_memory("inst", sesion, &e.proyecto, &m)
            .unwrap();
        let estado = ad
            .registry
            .select_task("inst", sesion, Some("t/login"), Some(1))
            .unwrap();
        // Seleccionar tarea no enciende nada ni cambia el modo.
        assert_eq!(estado["tarea"]["clave"], "t/login");
        assert_eq!(estado["modo"], "memoria_activa");
        ad.hook(&prompt(
            sesion,
            &e.proyecto,
            "p1",
            "el login falla con OAuth",
        ))
        .unwrap();
    }
    let mut store = ad.registry.open_memory(&m).unwrap();
    let informe = store.task_report(&proyecto, "t/login").unwrap();
    assert_eq!(informe["eventos_vinculados"], 2);
    assert_eq!(informe["sesiones"].as_array().unwrap().len(), 2);
    // Los grupos de evidencia siguen siendo uno por chat: la tarea agrupa, no
    // convierte dos chats en un episodio ni en dos ensayos independientes.
    assert_eq!(
        store.episode("claude-code/inst/c1").unwrap()["eventos"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(store.episode("claude-code/inst/c2").is_ok());

    // Un evento de otro proyecto no se puede enlazar a esta tarea.
    let ajeno = json!({"version":1,"clave_evento":"ajeno","grupo_evidencia":"g","secuencia":1,
        "origen":"usuario","rol":"otro","timestamp":1,"contenido":"x",
        "metadatos":{"proyecto":"otro-proyecto"}})
    .to_string();
    store.capture(&ajeno).unwrap();
    let error = store
        .record_task(&TaskRecord::TareaEnlazar {
            tarea: "t/login".into(),
            evento: "ajeno".into(),
            evidencia: "prueba".into(),
        })
        .unwrap_err()
        .to_string();
    assert!(error.contains("proyecto"), "{error}");
}

#[test]
fn criterios_versionados_cierre_sin_exito_y_resultado_sin_ejecucion_ficticia() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let registro = e.registro();
    let mut store = registro.open_memory(&m).unwrap();
    crear_tarea(&mut store, "t/login", &proyecto);

    // Revisar agrega una versión; la anterior sigue existiendo.
    store
        .record_task(&TaskRecord::TareaRevisar {
            tarea: "t/login".into(),
            definicion: "t/login/def/2".into(),
            descripcion: "arreglar el login y documentarlo".into(),
            criterios: criterios(true),
        })
        .unwrap();
    // Cerrar es estado operativo: no confirma calidad.
    store
        .record_task(&TaskRecord::TareaMarcar {
            clave: "t/login/cierre".into(),
            tarea: "t/login".into(),
            tipo: "cierre".into(),
            detalle: json!({"motivo":"entregado"}),
        })
        .unwrap();
    let informe = store.task_report(&proyecto, "t/login").unwrap();
    assert_eq!(informe["definiciones"].as_array().unwrap().len(), 2);
    assert_eq!(informe["definicion_vigente"], 2);
    assert_eq!(
        informe["calidad"]["todos_los_obligatorios_verificados"],
        false
    );
    assert_eq!(
        informe["calidad"]["desconocidos"].as_array().unwrap().len(),
        2
    );
    assert!(informe["calidad"]["cumplidos"]
        .as_array()
        .unwrap()
        .is_empty());

    // Resultado externo con evidencia dedicada y alcance de tarea: no se fabrica
    // ninguna ejecución ni recuperación para poder registrarlo.
    store
        .capture(&evidencia(
            "ver/1",
            1,
            &proyecto,
            "t/login",
            "c-tests",
            Some(1),
        ))
        .unwrap();
    store
        .record_task(&resultado(
            "t/login/res/1",
            "t/login",
            "c-tests",
            "ver/1",
            None,
        ))
        .unwrap();
    let informe = store.task_report(&proyecto, "t/login").unwrap();
    assert_eq!(informe["calidad"]["cumplidos"], json!(["c-tests/1"]));
    assert_eq!(
        informe["calidad"]["todos_los_obligatorios_verificados"],
        false
    );
    assert_eq!(
        informe["intentos_correlacionados"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let conn = Connection::open(e.archivo(&m)).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM ejecucion", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );

    // Un criterio que no está en la definición vigente se rechaza.
    store
        .capture(&evidencia(
            "ver/x",
            9,
            &proyecto,
            "t/login",
            "c-inventado",
            Some(1),
        ))
        .unwrap();
    assert!(store
        .record_task(&resultado(
            "t/login/res/x",
            "t/login",
            "c-inventado",
            "ver/x",
            None
        ))
        .is_err());
}

#[test]
fn correcciones_null_y_conflictos_no_se_convierten_en_un_estado_global() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let registro = e.registro();
    let mut store = registro.open_memory(&m).unwrap();
    crear_tarea(&mut store, "t/login", &proyecto);

    // Fallo, luego corregido a éxito: la historia queda, sin votación.
    for (clave, seq, r) in [
        ("ver/1", 1, Some(0)),
        ("ver/2", 2, Some(1)),
        ("ver/3", 3, None),
    ] {
        store
            .capture(&evidencia(clave, seq, &proyecto, "t/login", "c-tests", r))
            .unwrap();
    }
    store
        .record_task(&resultado("res/1", "t/login", "c-tests", "ver/1", None))
        .unwrap();
    store
        .record_task(&resultado(
            "res/2",
            "t/login",
            "c-tests",
            "ver/2",
            Some("res/1"),
        ))
        .unwrap();
    let informe = store.task_report(&proyecto, "t/login").unwrap();
    let grupo = &informe["por_criterio"][0];
    assert_eq!(grupo["estado"], "resultado_bajo_criterio");
    assert_eq!(grupo["hojas"].as_array().unwrap().len(), 1);
    assert_eq!(grupo["hojas"][0]["resultado"], 1);

    // Una corrección NULL no crea un resultado conocido nuevo: se conserva el
    // último conocido y se declara que viene del antecedente.
    store
        .record_task(&resultado(
            "res/3",
            "t/login",
            "c-tests",
            "ver/3",
            Some("res/2"),
        ))
        .unwrap();
    let informe = store.task_report(&proyecto, "t/login").unwrap();
    let grupo = &informe["por_criterio"][0];
    assert_eq!(grupo["hojas"][0]["resultado"], Value::Null);
    assert_eq!(grupo["hojas"][0]["resultado_conocido_de_la_cadena"], 1);
    assert_eq!(grupo["hojas"][0]["heredado_de_correccion_anterior"], true);
    assert_eq!(grupo["estado"], "resultado_bajo_criterio");

    // Una hoja independiente con el resultado opuesto es conflicto, no promedio.
    store
        .capture(&evidencia(
            "ver/4",
            4,
            &proyecto,
            "t/login",
            "c-tests",
            Some(0),
        ))
        .unwrap();
    store
        .record_task(&resultado("res/4", "t/login", "c-tests", "ver/4", None))
        .unwrap();
    let informe = store.task_report(&proyecto, "t/login").unwrap();
    let grupo = &informe["por_criterio"][0];
    assert_eq!(grupo["estado"], "conflicto");
    assert_eq!(informe["calidad"]["conflictos"], json!(["c-tests/1"]));
    assert_eq!(
        informe["calidad"]["todos_los_obligatorios_verificados"],
        false
    );

    // Una corrección con otro criterio o alcance se rechaza; el padre admite un
    // solo sucesor.
    store
        .capture(&evidencia(
            "ver/5",
            5,
            &proyecto,
            "t/login",
            "c-docs",
            Some(1),
        ))
        .unwrap();
    assert!(store
        .record_task(&resultado(
            "res/5",
            "t/login",
            "c-docs",
            "ver/5",
            Some("res/2")
        ))
        .is_err());
}

#[test]
fn un_resultado_puede_referenciar_una_evaluacion_de_tanda3_sin_duplicar_el_voto() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let registro = e.registro();
    let mut store = registro.open_memory(&m).unwrap();
    crear_tarea(&mut store, "t/login", &proyecto);

    // Problema, intento y evidencia de tanda 3 dentro del mismo episodio.
    let problema = json!({"version":1,"clave_evento":"p1","grupo_evidencia":"ep","secuencia":1,
        "origen":"usuario","rol":"problema","timestamp":10,"contenido":"sumar 2 y 3",
        "metadatos":{"proyecto":proyecto,"procedencia":"real"}})
    .to_string();
    let mut ver = serde_json::from_str::<Value>(&evidencia(
        "v1",
        2,
        &proyecto,
        "t/login",
        "c-tests",
        Some(1),
    ))
    .unwrap();
    ver["grupo_evidencia"] = json!("ep");
    ver["metadatos"]["evaluacion"]["alcance"] = json!("intento");
    ver["referencias"] = json!([{"tipo":"evalua","clave_evento":"p1"}]);
    ver["metadatos"]["evaluacion"]["detalle"]["entrada"] =
        json!({"objetivo":"p1","contenido":"sumar 2 y 3"});
    store.capture(&problema).unwrap();
    store.capture(&ver.to_string()).unwrap();
    store
        .record_execution(&msp::execution::Record::Intento {
            intento: "intento-1".into(),
            proyecto: proyecto.clone(),
            problema: "p1".into(),
        })
        .unwrap();
    store
        .record_execution(&msp::execution::Record::Evaluar {
            clave: "eval-1".into(),
            intento: "intento-1".into(),
            evidencia: "v1".into(),
        })
        .unwrap();

    // La evidencia tiene que pertenecer a la tarea antes de reutilizar su voto.
    let sin_vinculo = store
        .record_task(&TaskRecord::TareaResultadoDeEjecucion {
            clave: "res/1".into(),
            tarea: "t/login".into(),
            criterio: "c-tests".into(),
            criterio_version: "1".into(),
            evaluacion: "eval-1".into(),
            anterior: None,
        })
        .unwrap_err()
        .to_string();
    assert!(sin_vinculo.contains("no esta vinculado"), "{sin_vinculo}");

    store
        .record_task(&TaskRecord::TareaEnlazar {
            tarea: "t/login".into(),
            evento: "v1".into(),
            evidencia: "evidencia externa de este intento".into(),
        })
        .unwrap();
    store
        .record_task(&TaskRecord::TareaResultadoDeEjecucion {
            clave: "res/1".into(),
            tarea: "t/login".into(),
            criterio: "c-tests".into(),
            criterio_version: "1".into(),
            evaluacion: "eval-1".into(),
            anterior: None,
        })
        .unwrap();
    let informe = store.task_report(&proyecto, "t/login").unwrap();
    assert_eq!(informe["calidad"]["cumplidos"], json!(["c-tests/1"]));
    assert!(informe["resultados"][0]["evaluacion_ejecucion_id"]
        .as_i64()
        .is_some());
    // La misma evaluación no puede contarse dos veces bajo otra clave.
    assert!(store
        .record_task(&TaskRecord::TareaResultadoDeEjecucion {
            clave: "res/2".into(),
            tarea: "t/login".into(),
            criterio: "c-tests".into(),
            criterio_version: "1".into(),
            evaluacion: "eval-1".into(),
            anterior: None,
        })
        .is_err());
}

#[test]
fn cambiar_la_seleccion_deja_pendiente_la_salida_tardia_y_conserva_el_crudo() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    {
        let mut store = ad.registry.open_memory(&m).unwrap();
        crear_tarea(&mut store, "t/a", &proyecto);
        crear_tarea(&mut store, "t/b", &proyecto);
    }
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    ad.registry
        .select_task("inst", "c1", Some("t/a"), Some(1))
        .unwrap();

    // Callback admitido con t/a congelada; antes de enlazarlo cambia la tarea.
    let activo = ad
        .registry
        .gate("inst", "c1", Some(&e.proyecto))
        .unwrap()
        .unwrap();
    assert_eq!(activo.tarea.as_ref().unwrap().clave, "t/a");
    ad.registry
        .select_task("inst", "c1", Some("t/b"), Some(1))
        .unwrap();
    assert!(!ad.registry.task_still_selected(&activo).unwrap());

    let salida = ad
        .hook(&prompt("c1", &e.proyecto, "p1", "sigo trabajando"))
        .unwrap();
    // El crudo se conserva; la asociación queda pendiente y reparable.
    assert_eq!(salida.informe["capturado"], true);
    assert_eq!(salida.informe["tarea"]["clave"], "t/b");
    assert_eq!(salida.informe["tarea"]["estado"], "vinculado");

    // Un evento sin tarea seleccionada se repara con enlace manual explícito.
    ad.registry.select_task("inst", "c1", None, None).unwrap();
    let suelto = ad
        .hook(&prompt("c1", &e.proyecto, "p2", "otro mensaje"))
        .unwrap();
    assert_eq!(suelto.informe["tarea"]["estado"], "sin_tarea_seleccionada");
    let clave = suelto.informe["clave_evento"].as_str().unwrap().to_string();
    let mut store = ad.registry.open_memory(&m).unwrap();
    store
        .record_task(&TaskRecord::TareaEnlazar {
            tarea: "t/a".into(),
            evento: clave.clone(),
            evidencia: "reparacion manual: el usuario confirma que era de t/a".into(),
        })
        .unwrap();
    let informe = store.task_report(&proyecto, "t/a").unwrap();
    assert_eq!(informe["eventos_vinculados"], 1);
    // Un segundo enlace con otro motivo es conflicto, no sobrescritura.
    assert!(store
        .record_task(&TaskRecord::TareaEnlazar {
            tarea: "t/a".into(),
            evento: clave,
            evidencia: "otro motivo".into(),
        })
        .is_err());
}

#[test]
fn solo_registro_mide_sin_recuperar_y_apagado_sigue_sin_capturar() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    crear_tarea(&mut ad.registry.open_memory(&m).unwrap(), "t/x", &proyecto);

    // Un chat anterior deja el antecedente con memoria activa.
    ad.registry
        .use_memory("inst", "viejo", &e.proyecto, &m)
        .unwrap();
    ad.hook(&prompt(
        "viejo",
        &e.proyecto,
        "p1",
        "el indice compuesto de la migracion",
    ))
    .unwrap();

    // El chat anterior, con memoria activa, si preparo su paquete.
    let paquetes = |e: &Entorno, m: &str| -> i64 {
        Connection::open(e.archivo(m))
            .unwrap()
            .query_row("SELECT count(*) FROM paquete_recuperacion", [], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let antes = paquetes(&e, &m);
    assert_eq!(antes, 1);

    // Solo registro: captura y mide, pero no busca, no prepara ni emite.
    let estado = ad
        .registry
        .use_memory_with_mode("inst", "reg", &e.proyecto, &m, "solo_registro")
        .unwrap();
    assert_eq!(estado["modo"], "solo_registro");
    ad.registry
        .select_task("inst", "reg", Some("t/x"), Some(1))
        .unwrap();
    let salida = ad
        .hook(&prompt(
            "reg",
            &e.proyecto,
            "q1",
            "recordame el indice compuesto",
        ))
        .unwrap();
    assert_eq!(salida.informe["capturado"], true);
    assert_eq!(salida.informe["modo"], "solo_registro");
    assert_eq!(salida.informe["entrega"]["estado"], "sin_recuperacion");
    assert_eq!(salida.informe["entrega"]["paquete_preparado"], false);
    assert!(salida.contexto.is_none());
    assert_eq!(salida.informe["tarea"]["estado"], "vinculado");

    // Ni un paquete ni una entrega nuevos por ese prompt.
    assert_eq!(paquetes(&e, &m), antes);
    let store = ad.registry.open_memory(&m).unwrap();
    assert_eq!(
        store.deliveries(&proyecto).unwrap()["entregas"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    drop(store);

    // Cambiar de modo cierra el intervalo y abre otro: queda verificable.
    let cambio = ad
        .registry
        .set_mode("inst", "reg", "memoria_activa")
        .unwrap();
    assert_eq!(cambio["modo"], "memoria_activa");
    assert_eq!(cambio["intervalos_totales"], 2);
    ad.hook(&prompt(
        "reg",
        &e.proyecto,
        "q2",
        "recordame el indice compuesto",
    ))
    .unwrap();
    let mut store = ad.registry.open_memory(&m).unwrap();
    let informe = store.task_report(&proyecto, "t/x").unwrap();
    // Dos modos en el mismo chat: la corrida no sería comparable.
    assert_eq!(
        informe["modos_observados"],
        json!(["memoria_activa", "solo_registro"])
    );
    drop(store);

    // Apagado sigue intacto: ni captura ni entrega, y solo registro no se activa
    // al apagar.
    ad.registry.turn_off("inst", "reg").unwrap();
    let apagado = ad
        .hook(&prompt("reg", &e.proyecto, "q3", "y ahora"))
        .unwrap();
    assert_eq!(apagado.informe["estado"], "apagada");
    assert_eq!(apagado.informe["capturado"], false);
    assert_eq!(
        ad.registry.status("inst", "reg", None).unwrap()["modo"],
        "apagada"
    );
}

#[test]
fn dos_tareas_y_sesiones_concurrentes_no_duplican_metricas() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    {
        let mut store = ad.registry.open_memory(&m).unwrap();
        crear_tarea(&mut store, "t/a", &proyecto);
        crear_tarea(&mut store, "t/b", &proyecto);
    }
    for (sesion, t) in [("c1", "t/a"), ("c2", "t/b")] {
        ad.registry
            .use_memory("inst", sesion, &e.proyecto, &m)
            .unwrap();
        ad.registry
            .select_task("inst", sesion, Some(t), Some(1))
            .unwrap();
    }
    // Intercaladas, y una misma operación observada dos veces (replay + cambio).
    for (sesion, id, tool) in [("c1", "p1", "t1"), ("c2", "q1", "t2"), ("c1", "p2", "t3")] {
        ad.hook(&prompt(sesion, &e.proyecto, id, "trabajo"))
            .unwrap();
        let uso = hook(
            "PostToolUse",
            sesion,
            &e.proyecto,
            json!({"prompt_id":id,"tool_use_id":tool,"tool_name":"Bash",
                   "tool_input":{},"tool_response":"salida"}),
        );
        ad.hook(&uso).unwrap();
        // Replay exacto: no agrega una observación.
        ad.hook(&uso).unwrap();
    }
    let mut store = ad.registry.open_memory(&m).unwrap();
    let a = store.task_report(&proyecto, "t/a").unwrap();
    let b = store.task_report(&proyecto, "t/b").unwrap();
    assert_eq!(a["eventos_vinculados"], 4);
    assert_eq!(b["eventos_vinculados"], 2);
    assert_eq!(
        a["herramientas"]["operaciones"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        b["herramientas"]["operaciones"].as_array().unwrap().len(),
        1
    );
    for op in a["herramientas"]["operaciones"].as_array().unwrap() {
        assert_eq!(op["observaciones"], 1, "el replay sumo una observacion");
    }
    assert_eq!(a["herramientas"]["cobertura"], "por_id_de_operacion");
    assert_eq!(a["tokens"]["valor"], Value::Null);
}

#[test]
fn la_cobertura_de_identidad_declara_cuando_no_puede_prometer_conteos_exactos() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    crear_tarea(&mut ad.registry.open_memory(&m).unwrap(), "t/x", &proyecto);
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    ad.registry
        .select_task("inst", "c1", Some("t/x"), Some(1))
        .unwrap();

    // Sin ID de fuente: dos eventos legítimos idénticos son indistinguibles de
    // un replay. Se declara, no se resuelve con un timestamp nuevo.
    let compacta = hook("PreCompact", "c1", &e.proyecto, json!({"trigger":"auto"}));
    assert_eq!(ad.hook(&compacta).unwrap().informe["insertado"], true);
    assert_eq!(ad.hook(&compacta).unwrap().informe["insertado"], false);

    // Misma operación con payload distinto: se conserva como otra observación,
    // no se pierde ni se confunde con la primera.
    let base = |salida: &str| {
        hook(
            "PostToolUse",
            "c1",
            &e.proyecto,
            json!({"prompt_id":"p1","tool_use_id":"t1","tool_name":"Bash",
                   "tool_input":{},"tool_response":salida}),
        )
    };
    let primera = ad.hook(&base("parcial")).unwrap();
    let segunda = ad.hook(&base("completa")).unwrap();
    assert_eq!(primera.informe["observacion_secundaria"], false);
    assert_eq!(segunda.informe["observacion_secundaria"], true);
    assert_ne!(
        segunda.informe["clave_evento"],
        primera.informe["clave_evento"]
    );
    assert!(segunda.informe["clave_evento"]
        .as_str()
        .unwrap()
        .contains('#'));

    let mut store = ad.registry.open_memory(&m).unwrap();
    let informe = store.task_report(&proyecto, "t/x").unwrap();
    let cobertura = &informe["cobertura_de_identidad"];
    assert_eq!(cobertura["conteos_exactos"], false);
    assert_eq!(cobertura["eventos_con_identidad_derivada_del_payload"], 1);
    // La operación t1 se deduplica: dos observaciones de la misma operación.
    let ops = informe["herramientas"]["operaciones"].as_array().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0]["observaciones"], 2);
}

#[test]
fn snapshot_consistente_valida_hash_no_sobrescribe_y_deja_el_original_intacto() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    crear_tarea(&mut ad.registry.open_memory(&m).unwrap(), "t/x", &proyecto);
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    ad.hook(&prompt("c1", &e.proyecto, "p1", "antecedente uno"))
        .unwrap();

    // Con escrituras WAL en curso el snapshot sigue siendo consistente.
    let destino = e.raiz.join("snaps").join("inicial.sqlite");
    let manifiesto = crear_snapshot(&ad.registry, &m, &destino, "hito").unwrap();
    assert_eq!(manifiesto["corte"]["instancias"], 1);
    assert!(manifiesto["sha256"].as_str().unwrap().len() == 64);
    assert!(destino.is_file());
    assert!(ruta_manifiesto(&destino).is_file());
    // No queda ningun `.parcial` con nombre de snapshot valido.
    let mut parcial = destino.as_os_str().to_os_string();
    parcial.push(".parcial");
    assert!(!Path::new(&parcial).exists());
    integro(&destino);
    assert_eq!(validar_snapshot(&destino).unwrap()["valido"], true);

    // El original no cambia y sigue creciendo por su cuenta.
    ad.hook(&prompt("c1", &e.proyecto, "p2", "antecedente dos"))
        .unwrap();
    assert_eq!(validar_snapshot(&destino).unwrap()["valido"], true);
    let copia = Connection::open(&destino).unwrap();
    assert_eq!(
        copia
            .query_row("SELECT count(*) FROM instancia", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    // La copia conserva la identidad sellada y las migraciones.
    assert_eq!(
        copia
            .query_row("SELECT memoria_id FROM memoria_identidad", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        m
    );
    assert_eq!(manifiesto["migraciones"].as_array().unwrap().len(), 13);
    drop(copia);

    // No sobrescribe un destino existente.
    assert!(crear_snapshot(&ad.registry, &m, &destino, "otra").is_err());
    // Un archivo alterado se detecta por hash antes de usarse.
    let segundo = e.raiz.join("snaps").join("otro.sqlite");
    crear_snapshot(&ad.registry, &m, &segundo, "").unwrap();
    let conn = Connection::open(&segundo).unwrap();
    conn.execute("CREATE TABLE intruso(x)", []).unwrap();
    drop(conn);
    let error = validar_snapshot(&segundo).unwrap_err().to_string();
    assert!(error.contains("hash"), "{error}");
}

#[test]
fn corridas_aisladas_no_contaminan_produccion_ni_entre_si() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    crear_tarea(&mut ad.registry.open_memory(&m).unwrap(), "t/x", &proyecto);
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    ad.registry
        .select_task("inst", "c1", Some("t/x"), Some(1))
        .unwrap();
    ad.hook(&prompt(
        "c1",
        &e.proyecto,
        "p1",
        "el indice compuesto de la migracion",
    ))
    .unwrap();
    let inicial = e.raiz.join("snaps").join("inicial.sqlite");
    let manifiesto = crear_snapshot(&ad.registry, &m, &inicial, "inicial").unwrap();
    let desde = manifiesto["corte"]["instancia_id_maxima"].as_i64().unwrap();

    let casos = e.raiz.join("casos.json");
    std::fs::write(
        &casos,
        json!({"version":1,"raiz_trabajo":e.proyecto,"repeticiones":1,
            "condiciones":[
                {"nombre":"sin_recuperacion","modo":"solo_registro","snapshot":"snaps/inicial.sqlite"},
                {"nombre":"memoria_inicial","modo":"memoria_activa","snapshot":"snaps/inicial.sqlite"}],
            "casos":[{"id":"caso-1","prompt":"arregla la migracion","criterios":[{"id":"c-tests","version":"1"}],
                "estado_inicial_codigo":{"commit":"abc"},"configuracion":{"modelo":"m","herramientas":[]},
                "exposicion_previa":false}]})
        .to_string(),
    )
    .unwrap();
    let salida = e.raiz.join("corridas");
    let indice = preparar_corridas(&casos, &salida).unwrap();
    assert_eq!(indice["preparadas"], 2);

    // Cada corrida tiene su raíz aislada y su propia copia de trabajo.
    let dir = salida.join("caso-1").join("memoria_inicial").join("r1");
    let corrida: Value =
        serde_json::from_slice(&std::fs::read(dir.join("corrida.json")).unwrap()).unwrap();
    assert_eq!(corrida["antecedentes_elegibles"], true);
    assert_eq!(corrida["solo_eventos_desde_instancia_id"], desde);
    assert_ne!(corrida["corrida_id"], Value::Null);
    let copia = dir
        .join("datos")
        .join("memorias")
        .join(format!("{m}.sqlite"));
    assert!(copia.is_file());

    // La corrida captura en su copia, sin tocar produccion ni el snapshot.
    let mut corredor = Adapter::new(
        Registry::open(dir.join("datos")).unwrap(),
        "corrida".into(),
        4096,
    );
    corredor
        .registry
        .use_memory("corrida", "nuevo", &e.proyecto, &m)
        .unwrap();
    corredor
        .registry
        .select_task("corrida", "nuevo", Some("t/x"), Some(1))
        .unwrap();
    let salida_corrida = corredor
        .hook(&prompt(
            "nuevo",
            &e.proyecto,
            "r1",
            "recordame el indice compuesto",
        ))
        .unwrap();
    // Recupera el antecedente de la copia, no de produccion.
    assert!(salida_corrida
        .contexto
        .as_deref()
        .unwrap()
        .contains("indice compuesto"));

    // El informe de la corrida cuenta solo lo suyo, no los totales de la copia.
    let mut store = corredor.registry.open_memory(&m).unwrap();
    let propio = store.task_report_desde(&proyecto, "t/x", desde).unwrap();
    let todo = store.task_report(&proyecto, "t/x").unwrap();
    assert_eq!(propio["eventos_vinculados"], 1);
    assert_eq!(todo["eventos_vinculados"], 2);
    assert_eq!(propio["desde_instancia_id"], desde);
    drop(store);

    // Produccion y snapshot intactos.
    assert_eq!(validar_snapshot(&inicial).unwrap()["valido"], true);
    let mut produccion = ad.registry.open_memory(&m).unwrap();
    assert_eq!(
        produccion.task_report(&proyecto, "t/x").unwrap()["eventos_vinculados"],
        1
    );
    // La otra corrida parte de una copia limpia y no ve lo de la primera.
    let otra = salida.join("caso-1").join("sin_recuperacion").join("r1");
    let conn = Connection::open(
        otra.join("datos")
            .join("memorias")
            .join(format!("{m}.sqlite")),
    )
    .unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM instancia", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    // Una preparacion no sobrescribe una salida con contenido.
    assert!(preparar_corridas(&casos, &salida).is_err());
}

#[test]
fn la_tabla_declara_faltantes_desigualdades_y_exposicion_previa() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    crear_tarea(&mut ad.registry.open_memory(&m).unwrap(), "t/x", &proyecto);
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    ad.hook(&prompt("c1", &e.proyecto, "p1", "antecedente"))
        .unwrap();
    let inicial = e.raiz.join("snaps").join("inicial.sqlite");
    crear_snapshot(&ad.registry, &m, &inicial, "").unwrap();

    // Raíz de trabajo distinta de la de los antecedentes, modelo desconocido y
    // exposición previa: cada cosa se declara, ninguna se oculta.
    let otra_raiz = e.raiz.join("otro arbol");
    std::fs::create_dir_all(&otra_raiz).unwrap();
    let casos = e.raiz.join("casos.json");
    std::fs::write(
        &casos,
        json!({"version":1,"raiz_trabajo":otra_raiz,"repeticiones":1,
            "condiciones":[{"nombre":"memoria_inicial","modo":"memoria_activa","snapshot":"snaps/inicial.sqlite"}],
            "casos":[{"id":"caso-1","prompt":"x","criterios":[{"id":"c-tests","version":"1"}],
                "estado_inicial_codigo":{"commit":"abc"},"configuracion":{"modelo":null},
                "exposicion_previa":true}]})
        .to_string(),
    )
    .unwrap();
    let salida = e.raiz.join("corridas");
    let indice = preparar_corridas(&casos, &salida).unwrap();
    assert_eq!(indice["corridas"][0]["antecedentes_elegibles"], false);
    assert!(indice["corridas"][0]["motivo_no_elegible"].is_string());

    let tabla = tabla_comparativa(&salida).unwrap();
    assert_eq!(tabla["pendientes"], 1);
    let fila = &tabla["filas"][0];
    assert_eq!(fila["estado"], "pendiente");
    assert_eq!(fila["comparable"], false);
    let motivos = fila["motivos_no_comparable"].as_array().unwrap();
    let texto = motivos
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(texto.contains("pendiente"), "{texto}");
    assert!(texto.contains("no son elegibles"), "{texto}");
    assert!(
        texto.contains("modelo/configuracion desconocidos"),
        "{texto}"
    );
    assert!(texto.contains("exposicion previa"), "{texto}");
    // Nada se completa por suposición: sin informe, la calidad es nula.
    assert_eq!(fila["calidad"], Value::Null);
}

#[test]
fn migrar_v5_y_control_v1_preserva_datos_e_invariantes() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let mut ad = e.adaptador();
    ad.registry
        .use_memory("inst", "c1", &e.proyecto, &m)
        .unwrap();
    ad.hook(&prompt("c1", &e.proyecto, "p1", "historia previa"))
        .unwrap();
    drop(ad);

    // Reabrir aplica todo lo pendiente sin tocar lo anterior.
    let registro = e.registro();
    let conn = Connection::open(e.archivo(&m)).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        13
    );
    drop(conn);
    let control = Connection::open(e.datos.join("registro.sqlite")).unwrap();
    assert_eq!(
        control
            .query_row("SELECT count(*) FROM registro_migraciones", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        5
    );
    // El historial de control anterior sobrevivió a la recreación de su tabla.
    let acciones: i64 = control
        .query_row("SELECT count(*) FROM control_historial", [], |r| r.get(0))
        .unwrap();
    assert!(acciones > 0);
    // Y sigue siendo append-only.
    assert!(control
        .execute("DELETE FROM control_historial", [])
        .is_err());
    // Los intervalos anteriores quedaron declarados como memoria activa, que es
    // lo que de hecho hacían: no se reescribió la historia.
    assert_eq!(
        control
            .query_row("SELECT modo FROM intervalo_captura WHERE id=1", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        "memoria_activa"
    );
    drop(control);
    integro(&e.archivo(&m));
    integro(&e.datos.join("registro.sqlite"));

    // Las tablas de tarea son append-only e inmutables.
    let mut store = registro.open_memory(&m).unwrap();
    crear_tarea(&mut store, "t/x", &proyecto);
    drop(store);
    let conn = Connection::open(e.archivo(&m)).unwrap();
    for sql in [
        "DELETE FROM tarea",
        "UPDATE tarea SET proyecto='otro'",
        "DELETE FROM tarea_evento",
        "UPDATE tarea_evento SET descripcion='x'",
    ] {
        assert!(
            conn.execute(sql, []).is_err(),
            "{sql} deberia estar bloqueado"
        );
    }
    // Un evento no puede referenciar una definicion inexistente.
    assert!(conn
        .execute(
            "INSERT INTO tarea_evento(clave,tarea_id,tipo,definicion_version,detalle_json,registrado_en,contrato_json)
             VALUES ('x',1,'inicio',99,'{}',1,'{}')",
            [],
        )
        .is_err());
}

#[test]
fn cli_de_tanda5_en_procesos_separados() {
    let e = entorno();
    let proyecto = e.proyecto_clave();
    let m = e.memoria("A");
    let bin = env!("CARGO_BIN_EXE_msp");
    let corre = |args: &[&str]| {
        Command::new(bin)
            .args(args)
            .env("MSP_RAIZ", &e.datos)
            .output()
            .unwrap()
    };
    let contrato = e.raiz.join("crear.json");
    std::fs::write(
        &contrato,
        json!({"operacion":"tarea_crear","tarea":"t/cli","proyecto":proyecto,
            "definicion":"t/cli/def/1","descripcion":"desde la CLI",
            "criterios":[{"id":"c1","version":"1","descripcion":"pasa","obligatorio":true}]})
        .to_string(),
    )
    .unwrap();
    let salida = corre(&[
        "--control",
        "tarea-registrar",
        "--memoria",
        &m,
        "--contrato",
        contrato.to_str().unwrap(),
    ]);
    assert!(
        salida.status.success(),
        "{}",
        String::from_utf8_lossy(&salida.stderr)
    );
    // Repetir el contrato es idempotente en otro proceso.
    let repetida = corre(&[
        "--control",
        "tarea-registrar",
        "--memoria",
        &m,
        "--contrato",
        contrato.to_str().unwrap(),
    ]);
    assert!(repetida.status.success());
    assert_eq!(salida.stdout, repetida.stdout);

    let listado: Value = serde_json::from_slice(
        &corre(&[
            "--control",
            "tarea-listar",
            "--memoria",
            &m,
            "--proyecto",
            &proyecto,
        ])
        .stdout,
    )
    .unwrap();
    assert_eq!(listado["tareas"].as_array().unwrap().len(), 1);

    // El modo exige una sesión encendida y explícita.
    let sin_sesion = corre(&[
        "--control",
        "sesion-modo",
        "--sesion",
        "x",
        "--modo",
        "solo_registro",
    ]);
    assert!(!sin_sesion.status.success());
    assert!(corre(&[
        "--control",
        "sesion-usar",
        "--instalacion",
        "i",
        "--sesion",
        "s1",
        "--cwd",
        e.proyecto.to_str().unwrap(),
        "--memoria",
        &m,
        "--modo",
        "solo_registro",
    ])
    .status
    .success());
    let estado: Value = serde_json::from_slice(
        &corre(&[
            "--control",
            "sesion-estado",
            "--instalacion",
            "i",
            "--sesion",
            "s1",
        ])
        .stdout,
    )
    .unwrap();
    assert_eq!(estado["modo"], "solo_registro");

    // Informe a archivos JSON y Markdown.
    let json_path = e.raiz.join("informe.json");
    let md_path = e.raiz.join("informe.md");
    assert!(corre(&[
        "--control",
        "tarea-informe",
        "--memoria",
        &m,
        "--proyecto",
        &proyecto,
        "--tarea",
        "t/cli",
        "--json",
        json_path.to_str().unwrap(),
        "--markdown",
        md_path.to_str().unwrap(),
    ])
    .status
    .success());
    let informe: Value = serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
    assert_eq!(informe["tarea"], "t/cli");
    let markdown = std::fs::read_to_string(&md_path).unwrap();
    assert!(markdown.contains("# Tarea t/cli"));
    assert!(markdown.contains("Desconocidos | c1/1"));
    assert!(markdown.contains("Tokens: null"));

    // Snapshot y validación por procesos separados.
    let destino = e.raiz.join("s.sqlite");
    assert!(corre(&[
        "--control",
        "snapshot-crear",
        "--memoria",
        &m,
        "--destino",
        destino.to_str().unwrap(),
        "--nota",
        "cli",
    ])
    .status
    .success());
    let validado: Value = serde_json::from_slice(
        &corre(&[
            "--control",
            "snapshot-validar",
            "--archivo",
            destino.to_str().unwrap(),
        ])
        .stdout,
    )
    .unwrap();
    assert_eq!(validado["valido"], true);
}
