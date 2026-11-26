use anyhow::{bail, ensure, Context, Result};
use msp::adapter::{Adapter, PRESUPUESTO_POR_OMISION};
use msp::control::{data_root, Registry};
use msp::measure::{crear_snapshot, preparar_corridas, tabla_comparativa, validar_snapshot};
use msp::retrieval::{PrepareRequest, SearchRequest};
use msp::tasks::{informe_markdown, TaskRecord};
use msp::Store;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn emit(value: &impl serde::Serialize) -> Result<()> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value)?;
    writeln!(out)?;
    out.flush()?;
    Ok(())
}

fn number(args: &[String], index: &mut usize) -> Result<u64> {
    *index += 1;
    args.get(*index)
        .context("falta valor de opcion")?
        .parse()
        .context("valor entero invalido")
}

/// Opciones largas `--clave valor` más banderas sin valor. Un argumento que
/// empieza con `--` nunca puede confundirse con la BASE posicional histórica.
/// Una opción que no está en `conocidas` es un error: una mal escrita caería en
/// silencio a su valor por omisión (otro modo, otra audiencia, otra plataforma).
fn options(
    args: &[String],
    from: usize,
    conocidas: &[&str],
) -> Result<BTreeMap<String, Option<String>>> {
    const BANDERAS: &[&str] = &["--quitar", "--sin-emitir", "--sin-confirmar"];
    let mut map = BTreeMap::new();
    let mut index = from;
    while index < args.len() {
        let key = &args[index];
        ensure!(key.starts_with("--"), "se esperaba una opcion: {key}");
        ensure!(
            conocidas.contains(&key.as_str()),
            "opcion desconocida: {key}"
        );
        if BANDERAS.contains(&key.as_str()) {
            map.insert(key.clone(), None);
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .with_context(|| format!("falta valor de {key}"))?;
        ensure!(
            map.insert(key.clone(), Some(value.clone())).is_none(),
            "opcion repetida: {key}"
        );
        index += 2;
    }
    Ok(map)
}

fn required<'a>(map: &'a BTreeMap<String, Option<String>>, key: &str) -> Result<&'a str> {
    map.get(key)
        .and_then(|v| v.as_deref())
        .with_context(|| format!("falta {key}"))
}

fn registry_of(map: &BTreeMap<String, Option<String>>) -> Result<Registry> {
    let root = match map.get("--raiz-datos").and_then(|v| v.clone()) {
        Some(path) => PathBuf::from(path),
        None => data_root()?,
    };
    Registry::open(root)
}

fn install_of(map: &BTreeMap<String, Option<String>>, registry: &Registry) -> String {
    map.get("--instalacion")
        .and_then(|v| v.clone())
        .unwrap_or_else(|| registry.instalacion().to_string())
}

fn control(args: &[String]) -> Result<i32> {
    let command = args
        .get(1)
        .context("falta subcomando de --control")?
        .clone();
    const OPCIONES: &[&str] = &[
        "--archivo",
        "--arreglo",
        "--audiencia",
        "--automatico",
        "--canal",
        "--carpeta",
        "--casos",
        "--chat",
        "--consulta",
        "--contrato",
        "--cwd",
        "--definicion-version",
        "--desde",
        "--destino",
        "--dir",
        "--entrega",
        "--episodio",
        "--instalacion",
        "--json",
        "--limite",
        "--markdown",
        "--memoria",
        "--modo",
        "--nombre",
        "--nota",
        "--objetivo",
        "--origen",
        "--para",
        "--pasa-a",
        "--problema",
        "--proyecto",
        "--quitar",
        "--raiz",
        "--raiz-datos",
        "--recibe",
        "--repo",
        "--responde-a",
        "--resultado",
        "--rol",
        "--salida",
        "--secuencia",
        "--sesion",
        "--tarea",
        "--terminado",
        "--texto",
        "--tipo-objetivo",
        "--vinculo",
    ];
    let map = options(args, 2, OPCIONES)?;
    let mut registry = registry_of(&map)?;
    let install = install_of(&map, &registry);
    match command.as_str() {
        "memoria-crear" => emit(&registry.create_memory(required(&map, "--nombre")?)?)?,
        "memoria-listar" => emit(&registry.list_memories()?)?,
        "plataformas" => emit(&msp::plataformas::tabla())?,
        "proyecto-registrar" => {
            emit(&registry.register_project(Path::new(required(&map, "--raiz")?))?)?
        }
        "proyecto-predeterminado" => {
            let quitar = map.contains_key("--quitar");
            let memoria = map.get("--memoria").and_then(|v| v.as_deref());
            let modo = map.get("--modo").and_then(|v| v.as_deref());
            ensure!(
                quitar != memoria.is_some(),
                "usar exactamente uno de --memoria ID o --quitar"
            );
            ensure!(!quitar || modo.is_none(), "--modo requiere --memoria ID");
            emit(&registry.set_default_with_mode(
                Path::new(required(&map, "--raiz")?),
                if quitar { None } else { memoria },
                modo.unwrap_or("memoria_activa"),
            )?)?
        }
        "sesion-iniciar" => emit(&registry.init_session(
            &install,
            required(&map, "--sesion")?,
            Path::new(required(&map, "--cwd")?),
        )?)?,
        "sesion-usar" => emit(
            &registry.use_memory_with_mode(
                &install,
                required(&map, "--sesion")?,
                Path::new(required(&map, "--cwd")?),
                required(&map, "--memoria")?,
                map.get("--modo")
                    .and_then(|v| v.as_deref())
                    .unwrap_or("memoria_activa"),
            )?,
        )?,
        "sesion-apagar" => emit(&registry.turn_off(&install, required(&map, "--sesion")?)?)?,
        "sesion-estado" => emit(&registry.status(
            &install,
            required(&map, "--sesion")?,
            map.get("--cwd").and_then(|v| v.as_deref()).map(Path::new),
        )?)?,
        "sesion-historial" => emit(&registry.history(&install, required(&map, "--sesion")?)?)?,
        "entregas" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .deliveries(required(&map, "--proyecto")?)?,
        )?,
        "sesion-modo" => emit(&registry.set_mode(
            &install,
            required(&map, "--sesion")?,
            required(&map, "--modo")?,
        )?)?,
        "sesion-tarea" => {
            let quitar = map.contains_key("--quitar");
            let tarea = map.get("--tarea").and_then(|v| v.as_deref());
            ensure!(
                quitar != tarea.is_some(),
                "usar exactamente uno de --tarea CLAVE o --quitar"
            );
            let version = match map.get("--definicion-version").and_then(|v| v.as_deref()) {
                Some(v) => Some(v.parse::<i64>().context("--definicion-version invalida")?),
                None => None,
            };
            emit(&registry.select_task(&install, required(&map, "--sesion")?, tarea, version)?)?
        }
        "tarea-registrar" => {
            let contrato: TaskRecord =
                serde_json::from_reader(File::open(required(&map, "--contrato")?)?)?;
            emit(
                &registry
                    .open_memory(required(&map, "--memoria")?)?
                    .record_task(&contrato)?,
            )?
        }
        "tarea-listar" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .list_tasks(required(&map, "--proyecto")?)?,
        )?,
        "episodio-construir" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .construir_episodio(
                    required(&map, "--episodio")?,
                    required(&map, "--proyecto")?,
                    required(&map, "--tarea")?,
                    required(&map, "--resultado")?,
                )?,
        )?,
        "episodio-leer" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .leer_episodio(required(&map, "--proyecto")?, required(&map, "--episodio")?)?,
        )?,
        "episodios-procesar" => {
            let memoria = required(&map, "--memoria")?;
            let proyecto = required(&map, "--proyecto")?;
            let chat = required(&map, "--chat")?;
            let cierres = registry.cierres_para_episodios(memoria, proyecto, chat)?;
            emit(
                &registry
                    .open_memory(memoria)?
                    .procesar_episodios_automaticos(proyecto, chat, &cierres)?,
            )?;
        }
        "episodios-listar" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .listar_episodios_automaticos(
                    required(&map, "--proyecto")?,
                    required(&map, "--chat")?,
                )?,
        )?,
        "vigente-consultar" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .consultar_vigente(required(&map, "--proyecto")?, required(&map, "--objetivo")?)?,
        )?,
        "szz-rastrear" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .szz_rastrear(
                    required(&map, "--proyecto")?,
                    std::path::Path::new(required(&map, "--repo")?),
                    required(&map, "--arreglo")?,
                )?,
        )?,
        "szz-listar" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .szz_listar(required(&map, "--proyecto")?)?,
        )?,
        "szz-confirmar" | "szz-descartar" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .szz_decidir(
                    required(&map, "--proyecto")?,
                    required(&map, "--vinculo")?
                        .parse()
                        .context("--vinculo invalido")?,
                    ["confirmado", "descartado"][usize::from(command == "szz-descartar")],
                )?,
        )?,
        "objetivos-listar" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .listar_tipos_objetivo(required(&map, "--proyecto")?)?,
        )?,
        "secuencia-calcular" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .calcular_secuencia_minima(
                    required(&map, "--proyecto")?,
                    required(&map, "--origen")?,
                    required(&map, "--episodio")?,
                )?,
        )?,
        "secuencia-leer" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .leer_secuencia_minima(
                    required(&map, "--proyecto")?,
                    required(&map, "--origen")?,
                    required(&map, "--episodio")?,
                )?,
        )?,
        "arandu-secuencia-construir" => {
            let memoria = required(&map, "--memoria")?;
            let proyecto = required(&map, "--proyecto")?;
            let mut store = registry.open_memory(memoria)?;
            let salida = match (
                map.get("--episodio").and_then(|v| v.as_deref()),
                map.get("--automatico").and_then(|v| v.as_deref()),
            ) {
                (Some(episodio), None) => store.construir_secuencia_manual(proyecto, episodio)?,
                (None, Some(id)) => {
                    let id = id.parse::<i64>().context("--automatico invalido")?;
                    store.construir_secuencia_automatica(proyecto, id)?
                }
                _ => bail!("usar exactamente uno de --episodio CLAVE o --automatico ID"),
            };
            emit(&salida)?
        }
        "arandu-secuencia-leer" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .leer_secuencia(
                    required(&map, "--proyecto")?,
                    required(&map, "--secuencia")?,
                )?,
        )?,
        "arandu-secuencia-listar" => emit(
            &registry
                .open_memory(required(&map, "--memoria")?)?
                .listar_secuencias(required(&map, "--proyecto")?)?,
        )?,
        "arandu-solucion-recuperar" => {
            let memoria = required(&map, "--memoria")?;
            let proyecto = required(&map, "--proyecto")?;
            let problema = required(&map, "--problema")?
                .parse::<i64>()
                .context("--problema invalido")?;
            let tipo: Value = serde_json::from_str(required(&map, "--tipo-objetivo")?)
                .context("--tipo-objetivo no es JSON")?;
            let limite = match map.get("--limite").and_then(|v| v.as_deref()) {
                Some(valor) => valor.parse().context("--limite invalido")?,
                None => msp::soluciones::LIMITE_POR_OMISION,
            };
            let consulta = map.get("--consulta").and_then(|v| v.as_deref());
            emit(
                &registry
                    .open_memory(memoria)?
                    .recuperar_solucion(proyecto, problema, &tipo, consulta, limite)?,
            )?
        }
        "tarea-informe" => {
            let desde = match map.get("--desde").and_then(|v| v.as_deref()) {
                Some(v) => v.parse::<i64>().context("--desde invalido")?,
                None => 0,
            };
            let informe = registry
                .open_memory(required(&map, "--memoria")?)?
                .task_report_desde(
                    required(&map, "--proyecto")?,
                    required(&map, "--tarea")?,
                    desde,
                )?;
            if let Some(ruta) = map.get("--markdown").and_then(|v| v.as_deref()) {
                std::fs::write(ruta, informe_markdown(&informe))?;
            }
            match map.get("--json").and_then(|v| v.as_deref()) {
                Some(ruta) => {
                    std::fs::write(ruta, serde_json::to_string_pretty(&informe)?)?;
                    emit(&json!({ "informe": ruta }))?
                }
                None => emit(&informe)?,
            }
        }
        "snapshot-crear" => emit(&crear_snapshot(
            &registry,
            required(&map, "--memoria")?,
            Path::new(required(&map, "--destino")?),
            map.get("--nota").and_then(|v| v.as_deref()).unwrap_or(""),
        )?)?,
        "snapshot-validar" => emit(&validar_snapshot(Path::new(required(&map, "--archivo")?))?)?,
        "corrida-preparar" => emit(&preparar_corridas(
            Path::new(required(&map, "--casos")?),
            Path::new(required(&map, "--salida")?),
        )?)?,
        "corrida-informe" => {
            let dir = Path::new(required(&map, "--dir")?);
            let corrida: Value = serde_json::from_slice(&std::fs::read(dir.join("corrida.json"))?)
                .context("falta corrida.json en ese directorio")?;
            let aislado = Registry::open(
                corrida["raiz_datos"]
                    .as_str()
                    .context("corrida sin raiz de datos")?,
            )?;
            // El informe de una corrida solo cuenta lo que ella produjo: nada de
            // los totales historicos que venian dentro de la copia.
            let informe = aislado
                .open_memory(
                    corrida["memoria_id"]
                        .as_str()
                        .context("corrida sin memoria")?,
                )?
                .task_report_desde(
                    corrida["proyecto_normalizado"]
                        .as_str()
                        .context("corrida sin proyecto")?,
                    required(&map, "--tarea")?,
                    corrida["solo_eventos_desde_instancia_id"]
                        .as_i64()
                        .unwrap_or(0),
                )?;
            std::fs::write(
                dir.join("informe.json"),
                serde_json::to_string_pretty(&informe)?,
            )?;
            std::fs::write(dir.join("informe.md"), informe_markdown(&informe))?;
            emit(&json!({"informe": dir.join("informe.json"),
                "corrida": corrida["corrida_id"],
                "desde_instancia_id": informe["desde_instancia_id"]}))?
        }
        "corrida-tabla" => {
            let salida = Path::new(required(&map, "--salida")?);
            let tabla = tabla_comparativa(salida)?;
            std::fs::write(
                salida.join("tabla.json"),
                serde_json::to_string_pretty(&tabla)?,
            )?;
            emit(&tabla)?
        }
        "memoria-ruta" => {
            registry.open_memory(required(&map, "--memoria")?)?;
            emit(&json!({"memoria_id":required(&map,"--memoria")?,"abierta":true}))?
        }
        "canal-latido" => emit(&registry.canal_latido(&install, required(&map, "--sesion")?)?)?,
        "canal-publicar" => {
            let responde_a = match map.get("--responde-a").and_then(|v| v.as_deref()) {
                Some(v) => Some(v.parse::<i64>().context("--responde-a invalido")?),
                None => None,
            };
            emit(
                &registry.canal_publicar(
                    &install,
                    required(&map, "--sesion")?,
                    &msp::canales::Publicacion {
                        canal: required(&map, "--canal")?,
                        texto: required(&map, "--texto")?,
                        audiencia: map
                            .get("--audiencia")
                            .and_then(|v| v.as_deref())
                            .unwrap_or("todos"),
                        para: map.get("--para").and_then(|v| v.as_deref()),
                        responde_a,
                    },
                )?,
            )?
        }
        // El cursor avanza recien despues de escribir la salida completa.
        "canal-leer" => registry.canal_leer(
            &install,
            required(&map, "--sesion")?,
            required(&map, "--canal")?,
            emit,
        )?,
        "canal-historial" => {
            let desde = match map.get("--desde").and_then(|v| v.as_deref()) {
                Some(v) => v.parse::<i64>().context("--desde invalido")?,
                None => 0,
            };
            emit(&registry.canal_historial(
                &install,
                required(&map, "--sesion")?,
                required(&map, "--canal")?,
                desde,
            )?)?
        }
        "proyecto-carpeta" => emit(&registry.proyecto_carpeta(
            Path::new(required(&map, "--raiz")?),
            Path::new(required(&map, "--carpeta")?),
        )?)?,
        "proyecto-principal" => emit(&registry.proyecto_principal(
            Path::new(required(&map, "--raiz")?),
            Path::new(required(&map, "--carpeta")?),
        )?)?,
        "proyecto-ver" => emit(&registry.proyecto_ver(Path::new(required(&map, "--raiz")?))?)?,
        "proyecto-reglas" => emit(&registry.proyecto_reglas(
            Path::new(required(&map, "--raiz")?),
            required(&map, "--texto")?,
        )?)?,
        "proyecto-leer" => emit(&registry.proyecto_leer(&install, required(&map, "--sesion")?)?)?,
        "rol-definir" => emit(&registry.rol_definir(
            Path::new(required(&map, "--raiz")?),
            &msp::proyectos::Procedimiento {
                rol: required(&map, "--rol")?,
                recibe: required(&map, "--recibe")?,
                entrega: required(&map, "--entrega")?,
                terminado: required(&map, "--terminado")?,
                pasa_a: required(&map, "--pasa-a")?,
            },
        )?)?,
        "rol-tomar" => emit(&registry.rol_tomar(
            &install,
            required(&map, "--sesion")?,
            Some(required(&map, "--rol")?),
        )?)?,
        "rol-soltar" => emit(&registry.rol_tomar(&install, required(&map, "--sesion")?, None)?)?,
        "rol-pasar" => emit(&registry.rol_pasar(
            &install,
            required(&map, "--sesion")?,
            required(&map, "--texto")?,
        )?)?,
        other => bail!("subcomando de control desconocido: {other}"),
    }
    Ok(0)
}

/// Lee el payload del hook por stdin. stdout queda reservado al protocolo del
/// host; el informe va a un archivo o a stderr como diagnóstico.
fn hook(args: &[String]) -> Result<i32> {
    // Falla cerrada: una opción mal escrita (`--plataform codex`) no puede caer
    // en silencio a Claude Code, que es la única que abre el canal de control.
    const OPCIONES: &[&str] = &[
        "--plataforma",
        "--presupuesto-bytes",
        "--informe",
        "--sin-emitir",
        "--sin-confirmar",
        "--raiz-datos",
        "--instalacion",
    ];
    let map = options(args, 1, OPCIONES)?;
    let registry = registry_of(&map)?;
    let install = install_of(&map, &registry);
    let budget = match map.get("--presupuesto-bytes").and_then(|v| v.as_deref()) {
        Some(value) => value.parse().context("--presupuesto-bytes invalido")?,
        None => match std::env::var("MSP_PRESUPUESTO_BYTES") {
            Ok(value) => value.parse().context("MSP_PRESUPUESTO_BYTES invalido")?,
            Err(_) => PRESUPUESTO_POR_OMISION,
        },
    };
    let mut raw = String::new();
    io::stdin().read_to_string(&mut raw)?;
    let payload: Value = serde_json::from_str(raw.trim()).context("payload de hook no es JSON")?;
    let plataforma = match map.get("--plataforma").and_then(|v| v.as_deref()) {
        Some(nombre) => msp::plataformas::plataforma(nombre)?,
        None => &msp::plataformas::CLAUDE_CODE,
    };
    let mut adapter = Adapter::new(registry, install, budget).en(plataforma);
    let mut salida = adapter.hook(&payload)?;
    // `--sin-emitir` registra la frontera de escritura sin tocar stdout;
    // `--sin-confirmar` deja la entrega preparada y en estado incierto, como una
    // caída entre el commit de preparación y la confirmación.
    let emitido = if map.contains_key("--sin-confirmar") {
        false
    } else if map.contains_key("--sin-emitir") {
        adapter.confirmar_escritura(&salida, "replay_offline")?;
        false
    } else {
        adapter.emitir(&salida)?
    };
    salida.informe["contexto_escrito_al_host"] = json!(emitido);
    match map.get("--informe").and_then(|v| v.as_deref()) {
        Some(path) => std::fs::write(path, serde_json::to_string(&salida.informe)?)?,
        None => eprintln!("{}", serde_json::to_string(&salida.informe)?),
    }
    Ok(0)
}

/// Órdenes cortas: sólo con el binario `arandu`, para que una BASE posicional de `msp`
/// que se llame igual que un verbo siga significando lo de siempre.
fn corta(args: &[String]) -> Result<i32> {
    // Las opciones comunes pasan tal cual a la orden larga.
    let (mut propios, mut comunes) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < args.len() {
        if ["--raiz-datos", "--instalacion"].contains(&args[i].as_str()) {
            comunes.extend(args[i..(i + 2).min(args.len())].iter().cloned());
            i += 2;
        } else {
            propios.push(args[i].clone());
            i += 1;
        }
    }
    let registry = registry_of(&options(&comunes, 0, &["--raiz-datos", "--instalacion"])?)?;
    let cwd = std::env::current_dir()?;
    let (proyecto, memoria) = registry.proyecto_en(&cwd)?.unzip();
    drop(registry);
    let ctx = msp::cortas::Contexto {
        cwd: cwd.to_string_lossy().into_owned(),
        proyecto,
        memoria: memoria.flatten(),
        sesion: std::env::var("ARANDU_SESSION")
            .ok()
            .filter(|s| !s.is_empty()),
    };
    let mut larga = msp::cortas::traducir(&propios, &ctx)?;
    larga.extend(comunes);
    control(&larga)
}

fn run() -> Result<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arandu = std::env::args()
        .next()
        .and_then(|a| {
            Some(
                Path::new(&a)
                    .file_stem()?
                    .to_str()?
                    .eq_ignore_ascii_case("arandu"),
            )
        })
        .unwrap_or(false);
    if arandu {
        match args.first().map(String::as_str) {
            None | Some("help") => {
                println!("{}", msp::cortas::AYUDA);
                return Ok(0);
            }
            Some(verbo) if msp::cortas::VERBOS.contains(&verbo) => return corta(&args),
            _ => {}
        }
    }
    match args.first().map(String::as_str) {
        Some("--control") => return control(&args),
        Some("--hook") => return hook(&args),
        _ => {}
    }
    if args.is_empty() || args == ["--help"] {
        println!("memoria BASE import ARCHIVO.jsonl\nmemoria BASE episode GRUPO\nmemoria BASE work [--limit N] [--lease-ms N] [--stop-after-reserve] [--pause-after-reserve-ms N]\nmemoria BASE retry TRABAJO_ID\nTiempos almacenados: microsegundos UTC. --stop-after-reserve sale con codigo 3.");
        println!("memoria BASE search PROYECTO CONSULTA [--origin ORIGEN] [--role ROL] [--limit N]\nmemoria BASE detail PROYECTO INSTANCIA_ID [--before N] [--after N]\nmemoria BASE prepare PROYECTO PROBLEMA_ID CLAVE_PAQUETE CONSULTA [--budget-bytes N] [--origin ORIGEN] [--role ROL] [--limit N]\nmemoria BASE packet PROYECTO CLAVE_PAQUETE\nmemoria BASE reindex");
        println!("memoria BASE record CONTRATO.json\nmemoria BASE attempt PROYECTO INTENTO\nmemoria BASE unexposed PROYECTO\nmemoria BASE exposures PROYECTO [--call LLAMADA] [--packet PAQUETE]");
        println!(
            "\nTanda 4 (control y adaptador). La BASE posicional nunca empieza con '--':\n\
             memoria --control memoria-crear --nombre NOMBRE\n\
             memoria --control memoria-listar\n\
             memoria --control proyecto-registrar --raiz RUTA\n\
             memoria --control proyecto-predeterminado --raiz RUTA (--memoria ID [--modo solo_registro|memoria_activa] | --quitar)\n\
             memoria --control sesion-iniciar --sesion ID --cwd RUTA\n\
             memoria --control sesion-usar --sesion ID --cwd RUTA --memoria ID\n\
             memoria --control sesion-apagar --sesion ID\n\
             memoria --control sesion-estado --sesion ID [--cwd RUTA]\n\
             memoria --control sesion-historial --sesion ID\n\
             memoria --control entregas --memoria ID --proyecto PROYECTO\n\
             memoria --control sesion-modo --sesion ID --modo solo_registro|memoria_activa\n\
             memoria --control sesion-tarea --sesion ID (--tarea CLAVE --definicion-version N | --quitar)\n\
             memoria --control tarea-registrar --memoria ID --contrato CONTRATO.json\n\
             memoria --control tarea-listar --memoria ID --proyecto PROYECTO\n\
             memoria --control episodio-construir --memoria ID --proyecto PROYECTO --tarea CLAVE --resultado CLAVE --episodio CLAVE\n\
             memoria --control episodio-leer --memoria ID --proyecto PROYECTO --episodio CLAVE\n\
             memoria --control episodios-procesar --memoria ID --proyecto PROYECTO --chat GRUPO\n\
             memoria --control episodios-listar --memoria ID --proyecto PROYECTO --chat GRUPO\n\
             memoria --control secuencia-calcular --memoria ID --proyecto PROYECTO --origen manual_tarea|automatico_lista --episodio CLAVE_O_ID\n\
             memoria --control vigente-consultar --memoria ID --proyecto PROYECTO --objetivo TEXTO\n\
             memoria --control objetivos-listar --memoria ID --proyecto PROYECTO\n\
             memoria --control szz-rastrear --memoria ID --proyecto PROYECTO --repo RUTA --arreglo COMMIT\n\
             memoria --control szz-listar --memoria ID --proyecto PROYECTO\n\
             memoria --control szz-confirmar|szz-descartar --memoria ID --proyecto PROYECTO --vinculo ID\n\
             memoria --control secuencia-leer --memoria ID --proyecto PROYECTO --origen manual_tarea|automatico_lista --episodio CLAVE_O_ID\n\
             memoria --control arandu-secuencia-construir --memoria ID --proyecto PROYECTO (--episodio CLAVE | --automatico ID)\n\
             memoria --control arandu-secuencia-leer --memoria ID --proyecto PROYECTO --secuencia CLAVE\n\
             memoria --control arandu-secuencia-listar --memoria ID --proyecto PROYECTO\n\
             memoria --control arandu-solucion-recuperar --memoria ID --proyecto PROYECTO --problema INSTANCIA_ID --tipo-objetivo JSON [--consulta TEXTO] [--limite N]\n\
             memoria --control tarea-informe --memoria ID --proyecto PROYECTO --tarea CLAVE [--desde N] [--json A] [--markdown A]\n\
             memoria --control snapshot-crear --memoria ID --destino RUTA [--nota TEXTO]\n\
             memoria --control snapshot-validar --archivo RUTA\n\
             memoria --control corrida-preparar --casos CASOS.json --salida DIR\n\
             memoria --control corrida-informe --dir DIR --tarea CLAVE\n\
             memoria --control corrida-tabla --salida DIR\n\
             memoria --control plataformas\n\
             memoria --control canal-latido --sesion ID\n\
             memoria --control canal-publicar --sesion ID --canal general|proyecto --texto TEXTO [--audiencia todos|activos|uno] [--para SESION] [--responde-a MENSAJE_ID]\n\
             memoria --control canal-leer --sesion ID --canal general|proyecto\n\
             memoria --control canal-historial --sesion ID --canal general|proyecto [--desde MENSAJE_ID]\n\
             memoria --control proyecto-carpeta --raiz RUTA --carpeta RUTA\n\
             memoria --control proyecto-principal --raiz RUTA --carpeta RUTA\n\
             memoria --control proyecto-ver --raiz RUTA\n\
             memoria --control proyecto-reglas --raiz RUTA --texto TEXTO\n\
             memoria --control proyecto-leer --sesion ID\n\
             memoria --control rol-definir --raiz RUTA --rol NOMBRE --recibe TEXTO --entrega TEXTO --terminado TEXTO --pasa-a ROL\n\
             memoria --control rol-tomar --sesion ID --rol NOMBRE\n\
             memoria --control rol-soltar --sesion ID\n\
             memoria --control rol-pasar --sesion ID --texto TEXTO\n\
             memoria --hook [--plataforma claude-code|codex|opencode|local] [--presupuesto-bytes N] [--informe ARCHIVO] [--sin-emitir]   (payload por stdin)\n\
             Opciones comunes: --raiz-datos RUTA (o MSP_RAIZ), --instalacion ID.\n\
             La sesion es siempre explicita: nunca se adivina 'el ultimo chat'."
        );
        return Ok(0);
    }
    let command = args.get(1).context("falta comando; usar --help")?;
    match command.as_str() {
        "exposures" if args.len() >= 3 => {
            let mut call = None;
            let mut packet = None;
            let mut index = 3;
            while index < args.len() {
                let value = args.get(index + 1).context("falta valor de filtro")?;
                match args[index].as_str() {
                    "--call" => call = Some(value.as_str()),
                    "--packet" => packet = Some(value.as_str()),
                    other => bail!("opcion desconocida: {other}"),
                }
                index += 2;
            }
            emit(&Store::open(&args[0])?.exposures(&args[2], call, packet)?)?;
        }
        "record" if args.len() == 3 => {
            let record: msp::execution::Record = serde_json::from_reader(File::open(&args[2])?)?;
            emit(&Store::open(&args[0])?.record_execution(&record)?)?;
        }
        "attempt" if args.len() == 4 => {
            emit(&Store::open(&args[0])?.execution(&args[2], &args[3])?)?;
        }
        "unexposed" if args.len() == 3 => {
            emit(&Store::open(&args[0])?.unexposed_packets(&args[2])?)?;
        }
        "search" if args.len() >= 4 => {
            let (request, _) = search_args(&args, 4, false)?;
            emit(&Store::open(&args[0])?.search(&request)?)?;
        }
        "detail" if args.len() >= 4 => {
            let mut before = 0;
            let mut after = 0;
            let mut index = 4;
            while index < args.len() {
                match args[index].as_str() {
                    "--before" => before = number(&args, &mut index)?.try_into()?,
                    "--after" => after = number(&args, &mut index)?.try_into()?,
                    other => bail!("opcion desconocida: {other}"),
                }
                index += 1;
            }
            emit(&Store::open(&args[0])?.detail(&args[2], args[3].parse()?, before, after)?)?;
        }
        "prepare" if args.len() >= 6 => {
            let (busqueda, presupuesto_bytes) = search_args(&args, 6, true)?;
            let request = PrepareRequest {
                problema_id: args[3].parse()?,
                busqueda,
                presupuesto_bytes,
            };
            emit(&Store::open(&args[0])?.prepare_memory(&args[4], &request)?)?;
        }
        "packet" if args.len() == 4 => {
            emit(&Store::open(&args[0])?.memory_packet(&args[2], &args[3])?)?;
        }
        "reindex" if args.len() == 2 => {
            Store::open(&args[0])?.rebuild_search()?;
            emit(&json!({"indice":"reconstruido"}))?;
        }
        "import" if args.len() == 3 => {
            let mut input = BufReader::new(File::open(&args[2])?);
            let mut store = Store::open(&args[0])?;
            let mut line = String::new();
            let mut index = 0;
            loop {
                line.clear();
                if input.read_line(&mut line)? == 0 {
                    break;
                }
                index += 1;
                // El separador JSONL no forma parte del evento; el resto queda intacto.
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                let receipt = store
                    .capture(&line)
                    .with_context(|| format!("{}: linea {index}; no confirmada", args[2]))?;
                emit(&receipt)?;
            }
        }
        "episode" if args.len() == 3 => {
            emit(&Store::open(&args[0])?.episode(&args[2])?)?;
        }
        "retry" if args.len() == 3 => {
            let id: i64 = args[2].parse()?;
            Store::open(&args[0])?.retry(id)?;
            emit(&json!({"reanudado":id}))?;
        }
        "work" => {
            let mut limit = u64::MAX;
            let mut lease_ms = 30_000;
            let mut pause_ms = 0;
            let mut stop = false;
            let mut index = 2;
            while index < args.len() {
                match args[index].as_str() {
                    "--limit" => limit = number(&args, &mut index)?,
                    "--lease-ms" => lease_ms = number(&args, &mut index)?,
                    "--pause-after-reserve-ms" => pause_ms = number(&args, &mut index)?,
                    "--stop-after-reserve" => stop = true,
                    other => bail!("opcion desconocida: {other}"),
                }
                index += 1;
            }
            anyhow::ensure!(lease_ms > 0, "--lease-ms debe ser positivo");
            let mut store = Store::open(&args[0])?;
            let mut completed = 0;
            while completed < limit {
                let Some(lease) = store.reserve(Duration::from_millis(lease_ms))? else {
                    break;
                };
                emit(&json!({"reservado":lease}))?;
                if stop {
                    return Ok(3);
                }
                // Dormir cero milisegundos no hace nada: no hace falta un caso aparte.
                std::thread::sleep(Duration::from_millis(pause_ms));
                let calculated = store.calculate(&lease);
                let result: Value = match calculated {
                    Ok(value) => value,
                    Err(error) => {
                        store.fail(&lease, &format!("{error:#}"))?;
                        return Err(error.context(
                            "error tecnico registrado; reintento programado o trabajo fallido",
                        ));
                    }
                };
                // Si falla el commit o vence la reserva, se deja recuperable. Nunca se
                // convierte una confirmación incierta en un segundo resultado.
                store.finish(&lease, &result)?;
                emit(&json!({"completado":lease.trabajo_id,"resultado_tecnico":result}))?;
                completed += 1;
            }
            emit(&json!({"completados_en_ejecucion":completed}))?;
        }
        _ => bail!("comando o argumentos invalidos; usar --help"),
    }
    Ok(0)
}

fn search_args(args: &[String], start: usize, prepare: bool) -> Result<(SearchRequest, usize)> {
    let mut request = SearchRequest {
        proyecto: args[2].clone(),
        consulta: args[start - 1].clone(),
        origen: None,
        rol: None,
        limite: if prepare { 100 } else { 20 },
    };
    let mut budget = 4096;
    let mut index = start;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" => request.limite = number(args, &mut index)?.try_into()?,
            "--budget-bytes" if prepare => budget = number(args, &mut index)?.try_into()?,
            "--origin" | "--role" => {
                let origin = args[index] == "--origin";
                index += 1;
                let value = args.get(index).context("falta valor de filtro")?.clone();
                if origin {
                    request.origen = Some(value)
                } else {
                    request.rol = Some(value)
                };
            }
            other => bail!("opcion desconocida: {other}"),
        }
        index += 1;
    }
    Ok((request, budget))
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    }
}
