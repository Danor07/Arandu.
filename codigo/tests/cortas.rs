//! Órdenes cortas del binario `arandu`: la traducción pura y el camino completo.
use msp::cortas::{traducir, Contexto, AYUDA, VERBOS};
use proptest::prelude::*;
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};

fn ctx() -> Contexto {
    Contexto {
        cwd: "/p/sub".into(),
        proyecto: Some("/p".into()),
        memoria: Some("mem".into()),
        sesion: Some("chat".into()),
    }
}

fn t(args: &[&str]) -> Result<Vec<String>, String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    traducir(&args, &ctx()).map_err(|e| e.to_string())
}

fn ok(args: &[&str]) -> String {
    t(args).unwrap().join(" ")
}

#[test]
fn cada_verbo_se_traduce_a_su_orden_larga() {
    assert_eq!(ok(&["init"]), "--control proyecto-registrar --raiz /p/sub");
    // Lo que nombra al proyecto usa su raíz de identidad, no la subcarpeta donde estás.
    assert_eq!(
        ok(&["folder", "/otra"]),
        "--control proyecto-carpeta --raiz /p --carpeta /otra"
    );
    assert_eq!(
        ok(&["folder", "/otra", "--main"]),
        "--control proyecto-principal --raiz /p --carpeta /otra"
    );
    assert_eq!(ok(&["project"]), "--control proyecto-ver --raiz /p");
    assert_eq!(
        ok(&["start"]),
        "--control sesion-iniciar --sesion chat --cwd /p/sub"
    );
    assert_eq!(
        t(&["rules", "--set", "todo pasa por revisión"]).unwrap()[2..],
        ["--raiz", "/p", "--texto", "todo pasa por revisión"]
    );
    assert_eq!(
        t(&[
            "define", "guion", "--gets", "la idea", "--gives", "el guion", "--done", "aprobado",
            "--next", "revisor"
        ])
        .unwrap()[1..],
        [
            "rol-definir",
            "--raiz",
            "/p",
            "--rol",
            "guion",
            "--recibe",
            "la idea",
            "--entrega",
            "el guion",
            "--terminado",
            "aprobado",
            "--pasa-a",
            "revisor"
        ]
    );
    assert_eq!(ok(&["new", "mia"]), "--control memoria-crear --nombre mia");
    assert_eq!(ok(&["memories"]), "--control memoria-listar");
    assert_eq!(ok(&["platforms"]), "--control plataformas");
    assert_eq!(
        ok(&["use", "m2"]),
        "--control proyecto-predeterminado --raiz /p --memoria m2 --modo memoria_activa"
    );
    assert_eq!(
        ok(&["use", "--record", "m2"]),
        "--control proyecto-predeterminado --raiz /p --memoria m2 --modo solo_registro"
    );
    assert_eq!(
        ok(&["status"]),
        "--control sesion-estado --sesion chat --cwd /p/sub"
    );
    assert_eq!(
        ok(&["goals"]),
        "--control objetivos-listar --memoria mem --proyecto /p"
    );
    assert_eq!(
        ok(&["links"]),
        "--control szz-listar --memoria mem --proyecto /p"
    );
    assert_eq!(
        ok(&["blame"]),
        "--control szz-rastrear --memoria mem --proyecto /p --repo /p/sub --arreglo HEAD"
    );
    assert_eq!(
        ok(&["blame", "abc123"]),
        "--control szz-rastrear --memoria mem --proyecto /p --repo /p/sub --arreglo abc123"
    );
    assert_eq!(
        ok(&["confirm", "3"]),
        "--control szz-confirmar --memoria mem --proyecto /p --vinculo 3"
    );
    assert_eq!(
        ok(&["discard", "3"]),
        "--control szz-descartar --memoria mem --proyecto /p --vinculo 3"
    );
    assert_eq!(
        ok(&["inbox"]),
        "--control canal-leer --sesion chat --canal proyecto"
    );
    assert_eq!(
        ok(&["inbox", "--general"]),
        "--control canal-leer --sesion chat --canal general"
    );
    assert_eq!(
        ok(&["log"]),
        "--control canal-historial --sesion chat --canal proyecto"
    );
    assert_eq!(
        ok(&["log", "--general"]),
        "--control canal-historial --sesion chat --canal general"
    );
    assert_eq!(ok(&["ping"]), "--control canal-latido --sesion chat");
    assert_eq!(ok(&["rules"]), "--control proyecto-leer --sesion chat");
    assert_eq!(
        ok(&["role", "revisor"]),
        "--control rol-tomar --sesion chat --rol revisor"
    );
    assert_eq!(
        ok(&["role", "--drop"]),
        "--control rol-soltar --sesion chat"
    );
    // -m y -s pisan lo deducido, en cualquier posición.
    assert_eq!(
        ok(&["goals", "-m", "otra"]),
        "--control objetivos-listar --memoria otra --proyecto /p"
    );
    assert_eq!(
        ok(&["ping", "-s", "x"]),
        "--control canal-latido --sesion x"
    );
}

#[test]
fn el_texto_viaja_entero_y_las_audiencias_no_se_adivinan() {
    assert_eq!(
        t(&["best", "compilar el módulo"]).unwrap(),
        [
            "--control",
            "vigente-consultar",
            "--memoria",
            "mem",
            "--proyecto",
            "/p",
            "--objetivo",
            "compilar el módulo"
        ]
    );
    assert_eq!(
        t(&["pass", "listo; rm -rf /"]).unwrap(),
        [
            "--control",
            "rol-pasar",
            "--sesion",
            "chat",
            "--texto",
            "listo; rm -rf /"
        ]
    );
    assert_eq!(
        ok(&["say", "hola"]),
        "--control canal-publicar --sesion chat --canal proyecto --texto hola"
    );
    assert_eq!(
        ok(&["say", "hola", "--general", "--active"]),
        "--control canal-publicar --sesion chat --canal general --texto hola --audiencia activos"
    );
    assert_eq!(ok(&["say", "--to", "b", "hola", "--re", "7"]), "--control canal-publicar --sesion chat --canal proyecto --texto hola --audiencia uno --para b --responde-a 7");
    // Un guion solo es texto; cualquier otra cosa que empiece con guion es una opción.
    assert_eq!(
        ok(&["say", "-"]),
        "--control canal-publicar --sesion chat --canal proyecto --texto -"
    );
}

#[test]
fn lo_mal_escrito_o_incompleto_se_rechaza_con_un_mensaje_que_dice_que_hacer() {
    for (args, parte) in [
        (vec!["say", "hola", "--activ"], "unknown option: --activ"),
        (
            vec!["say", "hola", "--audiencia", "uno"],
            "unknown option: --audiencia",
        ),
        (vec!["inbox", "--active"], "unknown option: --active"),
        (vec!["use", "m", "--drop"], "unknown option: --drop"),
        (vec!["best", "-x"], "unknown option: -x"),
        (vec!["say", "hola", "--to"], "--to needs a value"),
        (vec!["ping", "-s"], "-s needs a value"),
        (
            vec!["say", "a", "--to", "b", "--to", "c"],
            "--to given twice",
        ),
        (
            vec!["inbox", "--general", "--general"],
            "--general given twice",
        ),
        (
            vec!["say", "a", "--active", "--to", "b"],
            "cannot be combined",
        ),
        (vec!["best"], "usage: arandu best GOAL"),
        (vec!["best", "a", "b"], "usage: arandu best GOAL"),
        (vec!["init", "sobra"], "usage: arandu init"),
        (vec!["blame", "a", "b"], "usage: arandu blame [REV]"),
        (vec!["role"], "usage: arandu role NAME | role --drop"),
        (vec!["role", "x", "--drop"], "usage: arandu role --drop"),
        (vec!["confirm"], "usage: arandu confirm N"),
        (
            vec!["define", "r", "--gets", "a", "--gives", "b", "--done", "c"],
            "define needs --next",
        ),
        (vec!["define", "--gets", "a"], "usage: arandu define ROLE"),
        (vec!["folder"], "usage: arandu folder PATH [--main]"),
        (vec!["rules", "sobra"], "usage: arandu rules [--set TEXT]"),
        (vec!["rules", "--set"], "--set needs a value"),
        (vec!["commit"], "unknown command: commit"),
        (vec![], "missing command"),
    ] {
        let error = t(&args).unwrap_err();
        assert!(error.contains(parte), "{args:?}: {error}");
    }
    // Sin contexto, el error dice cómo conseguirlo, y lo que no lo necesita sigue andando.
    let vacio = Contexto {
        cwd: "/x".into(),
        ..Contexto::default()
    };
    let con = |args: &[&str]| {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        traducir(&args, &vacio)
            .map(|v| v.join(" "))
            .map_err(|e| e.to_string())
    };
    assert!(con(&["best", "x"]).unwrap_err().contains("arandu init"));
    // Con proyecto pero sin memoria predeterminada, el consejo es elegir una.
    let sin_memoria = Contexto {
        memoria: None,
        ..ctx()
    };
    let error = traducir(&["goals".to_string()], &sin_memoria)
        .unwrap_err()
        .to_string();
    assert!(error.contains("arandu use MEMORY"), "{error}");
    assert!(con(&["best", "x", "-m", "m"])
        .unwrap_err()
        .contains("arandu init"));
    assert!(con(&["inbox"]).unwrap_err().contains("ARANDU_SESSION"));
    assert_eq!(
        con(&["init"]).unwrap(),
        "--control proyecto-registrar --raiz /x"
    );
    assert!(con(&["project"]).unwrap_err().contains("arandu init"));
    assert!(con(&["rules", "--set", "x"])
        .unwrap_err()
        .contains("arandu init"));
    // `use` fuera de un proyecto nombra la carpeta actual; el registro decide si existe.
    assert_eq!(
        con(&["use", "m"]).unwrap(),
        "--control proyecto-predeterminado --raiz /x --memoria m --modo memoria_activa"
    );
    assert_eq!(
        con(&["ping", "-s", "z"]).unwrap(),
        "--control canal-latido --sesion z"
    );
}

#[test]
fn la_ayuda_nombra_todos_los_verbos() {
    for verbo in VERBOS {
        assert!(
            AYUDA
                .lines()
                .any(|l| l.trim_start().split([' ', '|']).any(|p| p == *verbo)),
            "{verbo}"
        );
    }
    let mut ordenados = VERBOS.to_vec();
    ordenados.sort();
    ordenados.dedup();
    assert_eq!(ordenados.len(), VERBOS.len());
}

proptest! {
    /// Con cualquier argumento, una traducción aceptada es una orden `--control` de pares
    /// opción-valor, y todo texto posicional llega intacto como valor, nunca como opción.
    #[test]
    fn una_traduccion_aceptada_es_siempre_una_orden_larga_bien_formada(
        verbo in 0usize..VERBOS.len(),
        resto in prop::collection::vec("(--general|--active|--record|--drop|--main|--set|--gets|--gives|--done|--next|--to|--re|-m|-s|-|--x|[a-zñ ;$\"']{0,8})", 0..5),
    ) {
        let mut args = vec![VERBOS[verbo].to_string()];
        args.extend(resto.iter().cloned());
        if let Ok(larga) = traducir(&args, &ctx()) {
            prop_assert_eq!(&larga[0], "--control");
            prop_assert!(!larga[1].starts_with('-'));
            prop_assert_eq!(larga.len() % 2, 0);
            for par in larga[2..].chunks(2) {
                prop_assert!(par[0].starts_with("--"), "{:?}", larga);
            }
            // Ninguna opción corta se filtra a la orden larga.
            let claves: Vec<&String> = larga[2..].iter().step_by(2).collect();
            for corta in [
                "--general", "--active", "--record", "--drop", "--main", "--set", "--gets", "--gives",
                "--done", "--next", "--to", "--re", "-m", "-s",
            ] {
                prop_assert!(!claves.iter().any(|c| *c == corta));
            }
        }
    }

    /// El texto libre de `best`, `say` y `pass` llega byte por byte.
    #[test]
    fn el_texto_libre_no_se_altera(texto in "[^-].{0,40}|") {
        for verbo in ["best", "say", "pass"] {
            let larga = traducir(&[verbo.to_string(), texto.clone()], &ctx()).unwrap();
            prop_assert_eq!(larga.last().unwrap(), &texto);
        }
    }
}

// ---------- De punta a punta con los dos binarios ----------

fn correr(binario: &str, datos: &Path, cwd: &Path, args: &[&str], sesion: Option<&str>) -> Output {
    let mut c = Command::new(binario);
    c.args(args)
        .current_dir(cwd)
        .env("MSP_RAIZ", datos)
        .env_remove("ARANDU_SESSION");
    if let Some(s) = sesion {
        c.env("ARANDU_SESSION", s);
    }
    c.output().unwrap()
}

fn json(salida: Output) -> Value {
    assert!(
        salida.status.success(),
        "{}",
        String::from_utf8_lossy(&salida.stderr)
    );
    serde_json::from_slice(&salida.stdout).unwrap()
}

#[test]
fn arandu_deduce_proyecto_memoria_y_sesion_y_msp_no_cambia() {
    let d = tempfile::tempdir().unwrap();
    let (datos, proyecto) = (d.path().join("datos"), d.path().join("proyecto"));
    let sub = proyecto.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let arandu = env!("CARGO_BIN_EXE_arandu");
    let a =
        |cwd: &Path, args: &[&str], sesion: Option<&str>| correr(arandu, &datos, cwd, args, sesion);

    // Fuera de un proyecto, lo que necesita proyecto falla diciendo qué hacer.
    let fuera = a(&proyecto, &["best", "compilar"], None);
    assert!(!fuera.status.success() && fuera.stdout.is_empty());
    assert!(String::from_utf8_lossy(&fuera.stderr).contains("arandu init"));

    json(a(&proyecto, &["init"], None));
    let memoria = json(a(&proyecto, &["new", "mia"], None))["memoria_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        json(a(&proyecto, &["memories"], None))["memorias"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Sin memoria predeterminada todavía.
    assert!(
        String::from_utf8_lossy(&a(&sub, &["goals"], None).stderr).contains("arandu use MEMORY")
    );
    json(a(&proyecto, &["use", &memoria, "--record"], None));

    // Desde una subcarpeta: mismo proyecto, misma memoria, sin nombrarlos.
    let objetivos = json(a(&sub, &["goals"], None));
    assert_eq!(objetivos["tipos"], serde_json::json!([]));
    assert!(objetivos["proyecto"]
        .as_str()
        .unwrap()
        .ends_with("proyecto"));
    let mejor = json(a(&sub, &["best", "compilar módulo"], None));
    assert!(mejor["vigente"].is_null());
    assert_eq!(
        json(a(&sub, &["links"], None))["vinculos"],
        serde_json::json!([])
    );

    // Canales y roles con la sesión en ARANDU_SESSION o en -s.
    let largo = |args: &[&str]| json(a(&proyecto, args, None));
    largo(&[
        "--control",
        "sesion-iniciar",
        "--sesion",
        "uno",
        "--cwd",
        proyecto.to_str().unwrap(),
    ]);
    largo(&[
        "--control",
        "sesion-iniciar",
        "--sesion",
        "dos",
        "--cwd",
        sub.to_str().unwrap(),
    ]);
    assert_eq!(json(a(&sub, &["status"], Some("dos")))["sesion"], "dos");
    assert_eq!(json(a(&sub, &["ping"], Some("dos")))["sesion"], "dos");
    let publicado = json(a(
        &proyecto,
        &["say", "¿alguien? --no es opción", "--active"],
        Some("uno"),
    ));
    assert_eq!(
        (
            publicado["audiencia"].as_str(),
            publicado["destinatarios"].as_i64()
        ),
        (Some("activos"), Some(1))
    );
    assert_eq!(json(a(&sub, &["inbox", "-s", "dos"], None))["mensajes"], 1);
    assert_eq!(json(a(&sub, &["inbox"], Some("dos")))["mensajes"], 0);
    assert_eq!(
        json(a(&sub, &["inbox", "--general"], Some("dos")))["canal"],
        "general"
    );
    assert_eq!(json(a(&sub, &["log"], Some("dos")))["mensajes"], 1);
    let sin_sesion = a(&sub, &["inbox"], None);
    assert!(!sin_sesion.status.success());
    assert!(String::from_utf8_lossy(&sin_sesion.stderr).contains("ARANDU_SESSION"));
    // Una opción mal escrita no publica nada.
    let mala = a(&proyecto, &["say", "hola", "--activ"], Some("uno"));
    assert!(!mala.status.success() && mala.stdout.is_empty());
    assert_eq!(json(a(&sub, &["log"], Some("dos")))["mensajes"], 1);

    // Las opciones comunes pasan a la orden larga, en cualquier lugar después del verbo:
    // otra raíz de datos es otro registro, sin memorias.
    let otra = d.path().join("otros-datos");
    let otra = otra.to_str().unwrap();
    for args in [
        vec!["memories", "--raiz-datos", otra],
        vec!["memories", "--instalacion", "x", "--raiz-datos", otra],
        vec!["memories", "--raiz-datos", otra, "--instalacion", "x"],
    ] {
        assert_eq!(
            json(a(&proyecto, &args, None))["memorias"],
            serde_json::json!([]),
            "{args:?}"
        );
    }
    assert_eq!(
        json(a(&proyecto, &["memories"], None))["memorias"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Sin su valor, la opción común se rechaza en vez de tragarse el argumento siguiente.
    assert!(!a(&proyecto, &["memories", "--raiz-datos"], None)
        .status
        .success());

    // Ayuda corta sin argumentos; la forma larga y --help siguen igual con este nombre.
    let ayuda = a(&proyecto, &[], None);
    assert!(ayuda.status.success());
    assert_eq!(String::from_utf8_lossy(&ayuda.stdout).trim(), AYUDA);
    assert!(
        String::from_utf8_lossy(&a(&proyecto, &["--help"], None).stdout)
            .contains("--control memoria-crear")
    );
    assert_eq!(
        json(a(&proyecto, &["--control", "plataformas"], None)),
        json(a(&proyecto, &["platforms"], None))
    );

    // Con el nombre `msp` los verbos no existen: una BASE posicional llamada «best» o
    // «init» sigue significando lo de siempre.
    let msp = env!("CARGO_BIN_EXE_msp");
    for args in [vec!["best", "compilar"], vec!["init"], vec!["goals"]] {
        let salida = correr(msp, &datos, &proyecto, &args, None);
        assert!(
            !salida.status.success() && salida.stdout.is_empty(),
            "{args:?}"
        );
        assert!(!proyecto.join(args[0]).exists(), "{args:?} creó una base");
    }
    let vacio = correr(msp, &datos, &proyecto, &[], None);
    assert!(!String::from_utf8_lossy(&vacio.stdout).contains("arandu <command>"));
}
