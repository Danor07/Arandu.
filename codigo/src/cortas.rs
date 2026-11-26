//! Órdenes cortas, al estilo de git: `arandu best "objetivo"`. Cada una se traduce a la
//! orden larga de `--control` que ya existe; acá no hay lógica nueva, sólo nombres y lo
//! que se deduce de la carpeta. Los verbos y sus mensajes están en inglés.
use anyhow::{bail, ensure, Context as _, Result};

/// Lo que el programa ya sabe sin que se lo digan.
#[derive(Debug, Default, Clone)]
pub struct Contexto {
    /// Carpeta actual.
    pub cwd: String,
    /// Raíz de identidad del proyecto que contiene a `cwd`, si está registrado.
    pub proyecto: Option<String>,
    /// Memoria predeterminada de ese proyecto.
    pub memoria: Option<String>,
    /// Sesión (chat) actual: `ARANDU_SESSION`.
    pub sesion: Option<String>,
}

pub const AYUDA: &str = "\
arandu <command> [arguments]

Project and memory are taken from the current folder; the session from -s or ARANDU_SESSION.

  init                 register this folder as a project
  folder PATH [--main] add another folder to this project (--main: make it the main one)
  project              folders, roles and rules version of this project
  new NAME             create a memory
  memories             list memories
  use MEMORY [--record]  make MEMORY this project's default (--record: capture only)
  start                register this session (hooks do it for you)
  status               state of this session
  best GOAL            the current best solution for a goal
  goals                all goal types of this project
  blame [REV]          trace a fix commit (default HEAD) back to the episode that caused it
  links                list the links proposed by blame
  confirm N | discard N  decide a proposed link
  say TEXT [--general] [--active | --to SESSION] [--re N]  post to the project channel
  inbox [--general]    read pending messages
  log [--general]      channel history
  ping                 mark this session as active
  define ROLE --gets T --gives T --done T --next ROLE  write a role's procedure
  role NAME | role --drop  take or drop a role
  pass TEXT            hand the work over to the next role
  rules                project rules and this session's role procedure
  rules --set TEXT     set the project rules
  platforms            supported hosts

Options for any command: -m MEMORY, -s SESSION.
The long form still works: arandu --control ORDER --option value (see --help).";

pub const VERBOS: &[&str] = &[
    "init",
    "folder",
    "project",
    "new",
    "memories",
    "use",
    "start",
    "status",
    "best",
    "goals",
    "blame",
    "links",
    "confirm",
    "discard",
    "say",
    "inbox",
    "log",
    "ping",
    "define",
    "role",
    "pass",
    "rules",
    "platforms",
];

/// Argumentos de una orden corta, ya separados: posicionales, banderas y opciones con valor.
struct Entrada {
    posicionales: Vec<String>,
    banderas: Vec<String>,
    opciones: Vec<(String, String)>,
}

fn separar(args: &[String], banderas: &[&str], con_valor: &[&str]) -> Result<Entrada> {
    let mut e = Entrada {
        posicionales: Vec::new(),
        banderas: Vec::new(),
        opciones: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if con_valor.contains(&a) {
            let valor = args
                .get(i + 1)
                .with_context(|| format!("{a} needs a value"))?;
            ensure!(!e.opciones.iter().any(|(k, _)| k == a), "{a} given twice");
            e.opciones.push((a.to_string(), valor.clone()));
            i += 2;
        } else if banderas.contains(&a) {
            ensure!(!e.banderas.iter().any(|b| b == a), "{a} given twice");
            e.banderas.push(a.to_string());
            i += 1;
        } else {
            // Una opción mal escrita no puede pasar por texto: caería a un valor por omisión.
            ensure!(!a.starts_with('-') || a == "-", "unknown option: {a}");
            e.posicionales.push(args[i].clone());
            i += 1;
        }
    }
    Ok(e)
}

impl Entrada {
    fn opcion(&self, clave: &str) -> Option<&str> {
        self.opciones
            .iter()
            .find(|(k, _)| k == clave)
            .map(|(_, v)| v.as_str())
    }
    fn bandera(&self, clave: &str) -> bool {
        self.banderas.iter().any(|b| b == clave)
    }
    /// Exactamente `n` posicionales, o un error que dice cómo se usa.
    fn exactos(&self, n: usize, uso: &str) -> Result<()> {
        ensure!(self.posicionales.len() == n, "usage: arandu {uso}");
        Ok(())
    }
}

/// Traduce una orden corta (`args[0]` es el verbo) a los argumentos de `--control`.
pub fn traducir(args: &[String], ctx: &Contexto) -> Result<Vec<String>> {
    let verbo = args
        .first()
        .map(String::as_str)
        .context("missing command")?;
    let (banderas, con_valor): (&[&str], &[&str]) = match verbo {
        "use" => (&["--record"], &[]),
        "say" => (&["--general", "--active"], &["--to", "--re"]),
        "inbox" | "log" => (&["--general"], &[]),
        "role" => (&["--drop"], &[]),
        "folder" => (&["--main"], &[]),
        "rules" => (&[], &["--set"]),
        "define" => (&[], &["--gets", "--gives", "--done", "--next"]),
        _ => (&[], &[]),
    };
    let e = separar(&args[1..], banderas, &[con_valor, &["-m", "-s"]].concat())?;
    let proyecto = || {
        ctx.proyecto
            .clone()
            .context("this folder is not inside a registered project: run `arandu init`")
    };
    let memoria = || {
        // Primero el proyecto: sin él, el consejo útil es `arandu init`, no elegir memoria.
        proyecto()?;
        e.opcion("-m")
            .map(str::to_string)
            .or(ctx.memoria.clone())
            .context("no memory for this project: run `arandu use MEMORY`, or pass -m MEMORY")
    };
    let sesion = || {
        e.opcion("-s")
            .map(str::to_string)
            .or(ctx.sesion.clone())
            .context("no session: pass -s SESSION or set ARANDU_SESSION")
    };
    // La raíz con la que el registro conoce al proyecto; sin proyecto, la carpeta actual.
    let raiz = || ctx.proyecto.clone().unwrap_or_else(|| ctx.cwd.clone());
    let canal = || ["proyecto", "general"][usize::from(e.bandera("--general"))].to_string();
    let texto = |n: usize| e.posicionales[n].clone();
    let s = |v: &str| v.to_string();
    // (orden larga, pares opción-valor)
    let (orden, pares): (&str, Vec<(&str, String)>) = match verbo {
        "init" => {
            e.exactos(0, "init")?;
            ("proyecto-registrar", vec![("--raiz", ctx.cwd.clone())])
        }
        "folder" => {
            e.exactos(1, "folder PATH [--main]")?;
            (
                ["proyecto-carpeta", "proyecto-principal"][usize::from(e.bandera("--main"))],
                vec![("--raiz", proyecto()?), ("--carpeta", texto(0))],
            )
        }
        "project" => {
            e.exactos(0, "project")?;
            ("proyecto-ver", vec![("--raiz", proyecto()?)])
        }
        "start" => {
            e.exactos(0, "start")?;
            (
                "sesion-iniciar",
                vec![("--sesion", sesion()?), ("--cwd", ctx.cwd.clone())],
            )
        }
        "define" => {
            e.exactos(
                1,
                "define ROLE --gets TEXT --gives TEXT --done TEXT --next ROLE",
            )?;
            let campo = |corta: &str| {
                e.opcion(corta)
                    .map(str::to_string)
                    .with_context(|| format!("define needs {corta}"))
            };
            (
                "rol-definir",
                vec![
                    ("--raiz", proyecto()?),
                    ("--rol", texto(0)),
                    ("--recibe", campo("--gets")?),
                    ("--entrega", campo("--gives")?),
                    ("--terminado", campo("--done")?),
                    ("--pasa-a", campo("--next")?),
                ],
            )
        }
        "new" => {
            e.exactos(1, "new NAME")?;
            ("memoria-crear", vec![("--nombre", texto(0))])
        }
        "memories" => {
            e.exactos(0, "memories")?;
            ("memoria-listar", vec![])
        }
        "platforms" => {
            e.exactos(0, "platforms")?;
            ("plataformas", vec![])
        }
        "use" => {
            e.exactos(1, "use MEMORY [--record]")?;
            let modo = ["memoria_activa", "solo_registro"][usize::from(e.bandera("--record"))];
            (
                "proyecto-predeterminado",
                vec![
                    ("--raiz", raiz()),
                    ("--memoria", texto(0)),
                    ("--modo", s(modo)),
                ],
            )
        }
        "status" => {
            e.exactos(0, "status")?;
            (
                "sesion-estado",
                vec![("--sesion", sesion()?), ("--cwd", ctx.cwd.clone())],
            )
        }
        "best" => {
            e.exactos(1, "best GOAL")?;
            (
                "vigente-consultar",
                vec![
                    ("--memoria", memoria()?),
                    ("--proyecto", proyecto()?),
                    ("--objetivo", texto(0)),
                ],
            )
        }
        "goals" | "links" => {
            e.exactos(0, verbo)?;
            (
                ["objetivos-listar", "szz-listar"][usize::from(verbo == "links")],
                vec![("--memoria", memoria()?), ("--proyecto", proyecto()?)],
            )
        }
        "blame" => {
            ensure!(e.posicionales.len() <= 1, "usage: arandu blame [REV]");
            let rev = e.posicionales.first().cloned().unwrap_or_else(|| s("HEAD"));
            (
                "szz-rastrear",
                vec![
                    ("--memoria", memoria()?),
                    ("--proyecto", proyecto()?),
                    ("--repo", ctx.cwd.clone()),
                    ("--arreglo", rev),
                ],
            )
        }
        "confirm" | "discard" => {
            e.exactos(1, &format!("{verbo} N"))?;
            (
                ["szz-confirmar", "szz-descartar"][usize::from(verbo == "discard")],
                vec![
                    ("--memoria", memoria()?),
                    ("--proyecto", proyecto()?),
                    ("--vinculo", texto(0)),
                ],
            )
        }
        "say" => {
            e.exactos(1, "say TEXT [--general] [--active | --to SESSION] [--re N]")?;
            ensure!(
                !(e.bandera("--active") && e.opcion("--to").is_some()),
                "--active and --to cannot be combined"
            );
            let mut pares = vec![
                ("--sesion", sesion()?),
                ("--canal", canal()),
                ("--texto", texto(0)),
            ];
            if let Some(para) = e.opcion("--to") {
                pares.extend([("--audiencia", s("uno")), ("--para", s(para))]);
            } else if e.bandera("--active") {
                pares.push(("--audiencia", s("activos")));
            }
            if let Some(n) = e.opcion("--re") {
                pares.push(("--responde-a", s(n)));
            }
            ("canal-publicar", pares)
        }
        "inbox" | "log" => {
            e.exactos(0, &format!("{verbo} [--general]"))?;
            (
                ["canal-leer", "canal-historial"][usize::from(verbo == "log")],
                vec![("--sesion", sesion()?), ("--canal", canal())],
            )
        }
        "ping" => {
            e.exactos(0, "ping")?;
            ("canal-latido", vec![("--sesion", sesion()?)])
        }
        "rules" => {
            e.exactos(0, "rules [--set TEXT]")?;
            match e.opcion("--set") {
                Some(texto) => (
                    "proyecto-reglas",
                    vec![("--raiz", proyecto()?), ("--texto", s(texto))],
                ),
                None => ("proyecto-leer", vec![("--sesion", sesion()?)]),
            }
        }
        "role" => {
            if e.bandera("--drop") {
                e.exactos(0, "role --drop")?;
                ("rol-soltar", vec![("--sesion", sesion()?)])
            } else {
                e.exactos(1, "role NAME | role --drop")?;
                (
                    "rol-tomar",
                    vec![("--sesion", sesion()?), ("--rol", texto(0))],
                )
            }
        }
        "pass" => {
            e.exactos(1, "pass TEXT")?;
            (
                "rol-pasar",
                vec![("--sesion", sesion()?), ("--texto", texto(0))],
            )
        }
        otro => bail!("unknown command: {otro}"),
    };
    let mut salida = vec![s("--control"), s(orden)];
    for (clave, valor) in pares {
        salida.extend([s(clave), valor]);
    }
    Ok(salida)
}
