//! Registro de control de la tanda 4: identidades, rutas y estado operativo.
//! Nunca guarda prompts, respuestas, salidas de herramientas ni texto recuperado.
//! Única excepción, desde la migración 004: el texto que una sesión publica
//! deliberadamente en un canal (`canales.rs`), que necesita la identidad de
//! sesión y de proyecto de este registro en la misma transacción; y desde la
//! 005, las reglas y procedimientos de rol que el dueño registra (`proyectos.rs`).
use crate::{now, Store};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

const REGISTRO: &[(i64, &str)] = &[
    (1, include_str!("../registro/001_control.sql")),
    (2, include_str!("../registro/002_tarea_y_modo.sql")),
    (
        3,
        include_str!("../registro/003_predeterminado_con_modo.sql"),
    ),
    (4, include_str!("../registro/004_canales.sql")),
    (5, include_str!("../registro/005_proyectos.sql")),
];
pub const ADAPTADOR_VERSION: &str = "claude-code-adaptador-v1";
/// Versión de Claude Code contra la que se verificaron los contratos usados.
pub const HOST_PROBADO: &str = "2.1.270";
pub const HOST_MINIMO: &str = "2.1.195";

/// Límites que el estado declara siempre: el usuario no debe inferirlos.
pub const LIMITES: &[&str] = &[
    "El plugin no controla el historial ni la memoria propia de Claude Code.",
    "Apagar no borra recuerdos ya presentes en el contexto de este chat.",
    "Una sesion queda ligada a una sola memoria; para otra memoria, abrir un chat nuevo.",
    "Activar a mitad de chat no importa la conversacion previa.",
    "Reactivar abre un intervalo nuevo y excluye el tramo apagado, sin backfill.",
    "La escritura al canal del host no se promete exactamente una vez.",
    "Ofrecer contexto no prueba que el proveedor lo recibiera ni que se aplicara.",
    "Solo registro captura y mide; no busca ni entrega antecedentes.",
    "Seleccionar una tarea no enciende la memoria ni cambia el modo.",
    "Una tarea agrupa lo capturado durante ella; no afirma causalidad.",
    "Sin ID de fuente del host, dos callbacks identicos no se distinguen de un replay.",
];

fn opaque(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?)
}

/// Un ID opaco selecciona un archivo dentro de la raíz; un nombre visible nunca.
fn check_id(id: &str) -> Result<()> {
    ensure!(
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
        "ID de memoria invalido: se exigen 32 digitos hexadecimales minusculos"
    );
    Ok(())
}

fn separator() -> char {
    if cfg!(windows) {
        '\\'
    } else {
        '/'
    }
}

/// Clave canónica de una raíz. En Windows unifica mayúsculas, separadores y el
/// prefijo extendido; en otros sistemas conserva la distinción de mayúsculas.
pub fn normalizar_raiz(path: &Path) -> Result<(String, String)> {
    normalize(path)
}

fn normalize(path: &Path) -> Result<(String, String)> {
    let absolute = std::fs::canonicalize(path)
        .with_context(|| format!("raiz inexistente o inaccesible: {}", path.display()))?;
    let mut text = absolute.to_string_lossy().into_owned();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        text = format!(r"\\{rest}");
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        text = rest.to_string();
    }
    let key = if cfg!(windows) {
        text.to_lowercase().replace('/', "\\")
    } else {
        text.clone()
    };
    Ok((key, text))
}

fn inside(root: &str, candidate: &str) -> bool {
    if candidate == root {
        return true;
    }
    let prefix = if root.ends_with(separator()) {
        root.to_string()
    } else {
        format!("{root}{}", separator())
    };
    candidate.starts_with(&prefix)
}

/// Dos carpetas se superponen si son la misma o una contiene a la otra.
pub fn solapadas(a: &str, b: &str) -> bool {
    inside(a, b) || inside(b, a)
}

/// El proyecto dueño de esa carpeta exacta. Cualquier carpeta del proyecto lo
/// nombra: la principal no es especial.
pub(crate) fn proyecto_de_carpeta(c: &Connection, key: &str) -> Result<Option<String>> {
    Ok(c.query_row(
        "SELECT proyecto_id FROM proyecto_carpeta WHERE raiz_normalizada=?1",
        [key],
        |r| r.get(0),
    )
    .optional()?)
}

/// Un cwd pertenece al proyecto si cae en cualquiera de sus carpetas.
fn en_proyecto(c: &Connection, proyecto: &str, key: &str) -> Result<bool> {
    let mut stmt =
        c.prepare("SELECT raiz_normalizada FROM proyecto_carpeta WHERE proyecto_id=?1")?;
    let rows = stmt.query_map([proyecto], |r| r.get::<_, String>(0))?;
    for root in rows {
        if inside(&root?, key) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Raíz de datos: override explícito para pruebas, o un predeterminado privado
/// del usuario fuera del repositorio, del plugin y de su cache de actualización.
pub fn data_root() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("MSP_RAIZ") {
        ensure!(!value.is_empty(), "MSP_RAIZ vacia");
        return Ok(PathBuf::from(value));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("share"))
            })
    };
    Ok(base
        .context("no se pudo resolver la raiz de datos; fijar MSP_RAIZ")?
        // Nombre anterior a MSP: ahi viven los datos existentes.
        .join("memoria-persistente"))
}

// Filas de control leidas por las consultas de sesion; no son modelos.
/// (id, memoria, proyecto, estado, revision)
type SesionRow = (i64, Option<String>, Option<String>, String, i64);
/// (id, memoria, proyecto, estado, origen_seleccion, revision)
type EstadoRow = (i64, Option<String>, Option<String>, String, String, i64);
/// (id, memoria, estado, proyecto, revision)
type VinculoRow = (i64, Option<String>, String, Option<String>, i64);

/// Estado de la sesión resuelto por el registro, ya sin contenido del chat.
#[derive(Debug, Clone)]
pub struct Active {
    pub sesion_id: i64,
    pub intervalo_id: i64,
    pub revision: i64,
    pub memoria_id: String,
    pub archivo: PathBuf,
    pub proyecto: String,
    pub sesion_clave: String,
    /// `solo_registro` captura y mide, pero no busca ni entrega antecedentes.
    pub modo: String,
    /// Selección de tarea congelada al admitir el callback, no al terminarlo.
    pub tarea: Option<TareaSeleccionada>,
}

#[derive(Debug, Clone)]
pub struct TareaSeleccionada {
    pub clave: String,
    pub definicion_version: i64,
    pub revision: i64,
}

/// Los tres modos visibles. Apagado no es un modo del intervalo: es ausencia.
pub const MODOS: &[&str] = &["solo_registro", "memoria_activa"];

pub fn modo_valido(modo: &str) -> Result<()> {
    ensure!(
        MODOS.contains(&modo),
        "modo invalido: usar solo_registro o memoria_activa"
    );
    Ok(())
}

#[derive(Debug, Clone)]
pub struct Assignment {
    pub clave_evento: String,
    pub grupo_evidencia: String,
    pub secuencia: i64,
    /// Tiempo de fuente congelado en la asignacion.
    pub timestamp: i64,
    pub nueva: bool,
    /// Otra observacion del mismo ID de fuente con distinto payload. Se conserva
    /// como evento propio; el informe deduplica por operacion, no al capturar.
    pub observacion_secundaria: bool,
}

pub struct Registry {
    pub(crate) conn: Connection,
    root: PathBuf,
    instalacion: String,
}

impl Registry {
    /// Identidad estable de esta instalación del adaptador, por raíz de datos.
    pub fn instalacion(&self) -> &str {
        &self.instalacion
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("memorias"))?;
        let mut conn = Connection::open(root.join("registro.sqlite"))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS registro_migraciones(version INTEGER PRIMARY KEY, sha256 TEXT NOT NULL, aplicado_en INTEGER NOT NULL);")?;
        let applied: Vec<(i64, String)> = {
            let mut s =
                tx.prepare("SELECT version,sha256 FROM registro_migraciones ORDER BY version")?;
            let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        ensure!(
            applied.len() <= REGISTRO.len(),
            "registro de una version mas nueva"
        );
        for (index, (version, hash)) in applied.iter().enumerate() {
            ensure!(
                *version == REGISTRO[index].0
                    && *hash == crate::migration_digest(REGISTRO[index].1),
                "migracion de registro alterada o historial discontinuo: {version}"
            );
        }
        for (version, sql) in REGISTRO.iter().skip(applied.len()) {
            tx.execute_batch(sql)
                .with_context(|| format!("migracion de registro {version}"))?;
            tx.execute(
                "INSERT INTO registro_migraciones VALUES (?1,?2,?3)",
                params![version, crate::migration_digest(sql), now()?],
            )?;
        }
        let instalacion: String =
            tx.query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
        tx.commit()?;
        // Identidad de instalación durable y opaca, junto al registro que la usa.
        let marca = root.join("instalacion");
        let instalacion = match std::fs::read_to_string(&marca) {
            Ok(previous) => previous.trim().to_string(),
            Err(_) => {
                std::fs::write(&marca, &instalacion)?;
                instalacion
            }
        };
        check_id(&instalacion).context("archivo de instalacion corrupto")?;
        Ok(Self {
            conn,
            root,
            instalacion,
        })
    }

    /// Registra una memoria ya copiada dentro de una raíz aislada. Solo para
    /// corridas: conserva el linaje sellado de la copia en otro registro y no
    /// modifica ni adopta nada en el registro de producción.
    pub fn adopt_memory(&mut self, id: &str, nombre: &str) -> Result<Value> {
        check_id(id)?;
        let relative = format!("memorias/{id}.sqlite");
        let archivo = self.file_of(&relative)?;
        ensure!(
            archivo.is_file(),
            "falta la copia {}: copiarla antes de adoptarla",
            archivo.display()
        );
        let sellada = Store::open(&archivo)?.identity()?;
        ensure!(
            sellada.as_deref() == Some(id),
            "la copia no conserva la identidad sellada esperada"
        );
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existente: Option<String> = tx
            .query_row("SELECT nombre FROM memoria WHERE id=?1", [id], |r| r.get(0))
            .optional()?;
        if existente.is_none() {
            tx.execute(
                "INSERT INTO memoria VALUES (?1,?2,?3,1,?4)",
                params![id, nombre, relative, time],
            )?;
            log(
                &tx,
                None,
                "memoria_creada",
                json!({"memoria":id,"adoptada":true}),
                time,
            )?;
        }
        tx.commit()?;
        Ok(json!({"memoria_id":id,"archivo":archivo,"adoptada":true}))
    }

    pub fn memory_name(&self, id: &str) -> Result<Option<String>> {
        check_id(id)?;
        Ok(self
            .conn
            .query_row("SELECT nombre FROM memoria WHERE id=?1", [id], |r| r.get(0))
            .optional()?)
    }

    fn file_of(&self, relative: &str) -> Result<PathBuf> {
        let (folder, name) = relative
            .split_once('/')
            .context("ruta de memoria invalida en el registro")?;
        ensure!(folder == "memorias", "carpeta de memoria invalida");
        let id = name
            .strip_suffix(".sqlite")
            .context("nombre de archivo de memoria invalido")?;
        check_id(id)?;
        Ok(self.root.join("memorias").join(name))
    }

    /// Abre el archivo de contenido tras comprobar que corresponde a ese ID.
    pub fn open_memory(&self, id: &str) -> Result<Store> {
        check_id(id)?;
        let (relative, ready): (String, i64) = self
            .conn
            .query_row(
                "SELECT archivo_relativo,lista FROM memoria WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .with_context(|| format!("memoria desconocida: {id}"))?;
        ensure!(ready == 1, "memoria sin inicializar; repetir memoria-crear");
        let store = Store::open(self.file_of(&relative)?)?;
        ensure!(
            store.identity()?.as_deref() == Some(id),
            "el archivo no corresponde a la memoria pedida"
        );
        Ok(store)
    }

    /// Crear es idempotente y **no** activa nada. Si una ejecución anterior cayó
    /// entre reservar el nombre e inicializar el archivo, repetirla lo completa.
    pub fn create_memory(&mut self, name: &str) -> Result<Value> {
        ensure!(!name.trim().is_empty(), "nombre de memoria vacio");
        ensure!(name.len() <= 200, "nombre de memoria demasiado largo");
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, String, i64)> = tx
            .query_row(
                "SELECT id,archivo_relativo,lista FROM memoria WHERE nombre=?1",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (id, relative, ready) = match existing {
            Some(row) => row,
            None => {
                let id = opaque(&tx)?;
                let relative = format!("memorias/{id}.sqlite");
                tx.execute(
                    "INSERT INTO memoria VALUES (?1,?2,?3,0,?4)",
                    params![id, name, relative, time],
                )?;
                log(&tx, None, "memoria_creada", json!({ "memoria": id }), time)?;
                (id, relative, 0)
            }
        };
        tx.commit()?;
        if ready == 0 {
            // El archivo se migra y sella antes de marcar la memoria usable: un
            // fallo aquí no deja un registro utilizable apuntando a algo vacío.
            let mut store = Store::open(self.file_of(&relative)?)?;
            store.stamp_identity(&id, name)?;
            drop(store);
            self.conn
                .execute("UPDATE memoria SET lista=1 WHERE id=?1", [&id])?;
        }
        Ok(
            json!({"memoria_id":id,"nombre":name,"archivo":self.file_of(&relative)?,
                  "activada":false,"nota":"crear no activa: usar memoria-usar en un chat"}),
        )
    }

    pub fn list_memories(&self) -> Result<Value> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id,m.nombre,m.archivo_relativo,m.lista,m.creada_en,
                    (SELECT count(*) FROM proyecto_local p WHERE p.memoria_predeterminada=m.id),
                    (SELECT count(*) FROM sesion_memoria s WHERE s.memoria_id=m.id),
                    (SELECT count(*) FROM sesion_memoria s WHERE s.memoria_id=m.id AND s.estado='encendida')
             FROM memoria m ORDER BY m.creada_en,m.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(
                json!({"memoria_id":r.get::<_,String>(0)?,"nombre":r.get::<_,String>(1)?,
                "archivo_relativo":r.get::<_,String>(2)?,"lista":r.get::<_,i64>(3)?==1,
                "creada_en":r.get::<_,i64>(4)?,"proyectos_con_predeterminado":r.get::<_,i64>(5)?,
                "sesiones_vinculadas":r.get::<_,i64>(6)?,"sesiones_encendidas":r.get::<_,i64>(7)?}),
            )
        })?;
        Ok(json!({"raiz_datos":self.root,"memorias":rows.collect::<rusqlite::Result<Vec<_>>>()?}))
    }

    /// Proyecto que contiene a `cwd`: (raíz de identidad, memoria predeterminada). La raíz
    /// es el nombre del proyecto dentro de las memorias. `None` si la carpeta no está en
    /// ningún proyecto registrado.
    pub fn proyecto_en(&self, cwd: &Path) -> Result<Option<(String, Option<String>)>> {
        let Ok(id) = project_of(&self.conn, &normalize(cwd)?.0) else {
            return Ok(None);
        };
        Ok(Some(self.conn.query_row(
            "SELECT raiz_normalizada,memoria_predeterminada FROM proyecto_local WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?))
    }

    pub fn register_project(&mut self, root: &Path) -> Result<Value> {
        let (key, original) = normalize(root)?;
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: Option<String> = tx
            .query_row(
                "SELECT id FROM proyecto_local WHERE raiz_normalizada=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        let id = match id {
            Some(id) => id,
            None => {
                let id = opaque(&tx)?;
                tx.execute(
                    "INSERT INTO proyecto_local(id,raiz_normalizada,raiz_original,memoria_predeterminada,registrado_en) VALUES (?1,?2,?3,NULL,?4)",
                    params![id, key, original, time],
                )?;
                log(
                    &tx,
                    None,
                    "proyecto_registrado",
                    json!({"proyecto":id,"raiz":key}),
                    time,
                )?;
                id
            }
        };
        tx.commit()?;
        Ok(
            json!({"proyecto_id":id,"raiz_normalizada":key,"raiz_original":original,
            "nota":"un worktree distinto es otra raiz y por lo tanto otro proyecto"}),
        )
    }

    /// El predeterminado se aplica al inicializar sesiones nuevas. No mueve chats
    /// existentes ni activa nada por sí mismo.
    pub fn set_default(&mut self, root: &Path, memory: Option<&str>) -> Result<Value> {
        self.set_default_with_mode(root, memory, "memoria_activa")
    }

    pub fn set_default_with_mode(
        &mut self,
        root: &Path,
        memory: Option<&str>,
        modo: &str,
    ) -> Result<Value> {
        modo_valido(modo)?;
        if let Some(id) = memory {
            check_id(id)?;
        }
        let (key, _) = normalize(root)?;
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let project = proyecto_de_carpeta(&tx, &key)?
            .context("raiz no registrada; usar proyecto-registrar")?;
        if let Some(id) = memory {
            let ready: i64 = tx
                .query_row("SELECT lista FROM memoria WHERE id=?1", [id], |r| r.get(0))
                .optional()?
                .context("memoria desconocida")?;
            ensure!(ready == 1, "memoria sin inicializar");
        }
        tx.execute(
            "UPDATE proyecto_local SET memoria_predeterminada=?1,modo_predeterminado=?2 WHERE id=?3",
            params![memory, modo, project],
        )?;
        let unaffected: i64 = tx.query_row(
            "SELECT count(*) FROM sesion_memoria WHERE proyecto_id=?1",
            [&project],
            |r| r.get(0),
        )?;
        log(
            &tx,
            None,
            if memory.is_some() {
                "predeterminado_fijado"
            } else {
                "predeterminado_quitado"
            },
            json!({"proyecto":project,"memoria":memory,"modo":modo}),
            time,
        )?;
        tx.commit()?;
        Ok(
            json!({"proyecto_id":project,"raiz_normalizada":key,"memoria_predeterminada":memory,"modo_predeterminado":modo,
            "sesiones_existentes_sin_cambios":unaffected,
            "nota":"solo afecta a chats nuevos de esta raiz"}),
        )
    }

    /// Materializa la fila de una sesión nueva aplicando la precedencia:
    /// decisión explícita previa > predeterminado del proyecto > apagado.
    /// Una sesión ya conocida se devuelve intacta: reanudar conserva su decisión.
    pub fn init_session(&mut self, install: &str, session: &str, cwd: &Path) -> Result<Value> {
        ensure!(
            !install.trim().is_empty() && !session.trim().is_empty(),
            "instalacion/sesion vacias"
        );
        let observed = normalize(cwd).ok();
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let known: Option<i64> = tx
            .query_row(
                "SELECT id FROM sesion_memoria WHERE instalacion=?1 AND sesion_host=?2",
                params![install, session],
                |r| r.get(0),
            )
            .optional()?;
        let created = known.is_none();
        if created {
            let project = observed
                .as_ref()
                .and_then(|(key, _)| project_of(&tx, key).ok());
            let (default,modo): (Option<String>,String) = match &project {
                Some(id) => tx.query_row(
                    "SELECT memoria_predeterminada,modo_predeterminado FROM proyecto_local WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?,r.get(1)?)),
                )?,
                None => (None,"memoria_activa".into()),
            };
            let (state, source) = match &default {
                Some(_) => ("encendida", "predeterminado_proyecto"),
                None => ("apagada", "sin_seleccion"),
            };
            tx.execute("INSERT INTO sesion_memoria(instalacion,sesion_host,proyecto_id,memoria_id,estado,origen_seleccion,revision,creada_en,actualizado_en,recuperacion) VALUES (?1,?2,?3,?4,?5,?6,0,?7,?7,?8)",
                params![install,session,project,default,state,source,time,i64::from(modo=="memoria_activa")])?;
            let id = tx.last_insert_rowid();
            log(
                &tx,
                Some(id),
                "sesion_inicializada",
                json!({"memoria":default,"origen":source,"modo":if default.is_some(){modo.as_str()}else{"apagada"}}),
                time,
            )?;
            if let Some(memory) = &default {
                open_interval(
                    &tx,
                    id,
                    memory,
                    0,
                    "sesion_nueva_con_predeterminado",
                    &modo,
                    time,
                )?;
                log(
                    &tx,
                    Some(id),
                    "memoria_encendida",
                    json!({"motivo":"predeterminado_proyecto","modo":modo}),
                    time,
                )?;
            }
        }
        tx.commit()?;
        let mut state = self.status(install, session, Some(cwd))?;
        state["sesion_creada_ahora"] = json!(created);
        Ok(state)
    }

    /// Vincula esta sesión a una memoria y la enciende. Solo por invocación
    /// explícita del usuario: ni el modelo ni un texto recuperado llegan aquí.
    pub fn use_memory(
        &mut self,
        install: &str,
        session: &str,
        cwd: &Path,
        memory: &str,
    ) -> Result<Value> {
        self.use_memory_with_mode(install, session, cwd, memory, "memoria_activa")
    }

    /// Enciende la sesión en un modo explícito. `solo_registro` conserva captura
    /// e instrumentación pero no busca, no prepara paquetes ni escribe contexto.
    pub fn use_memory_with_mode(
        &mut self,
        install: &str,
        session: &str,
        cwd: &Path,
        memory: &str,
        modo: &str,
    ) -> Result<Value> {
        check_id(memory)?;
        modo_valido(modo)?;
        let (key, _) = normalize(cwd)?;
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ready: i64 = tx
            .query_row("SELECT lista FROM memoria WHERE id=?1", [memory], |r| {
                r.get(0)
            })
            .optional()?
            .context("memoria desconocida")?;
        ensure!(ready == 1, "memoria sin inicializar; repetir memoria-crear");
        let project = project_of(&tx, &key)
            .context("cwd fuera de toda raiz registrada; usar proyecto-registrar")?;
        let row: Option<VinculoRow> = tx
            .query_row("SELECT id,memoria_id,estado,proyecto_id,revision FROM sesion_memoria WHERE instalacion=?1 AND sesion_host=?2",
                params![install,session],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let id = match row {
            None => {
                tx.execute("INSERT INTO sesion_memoria(instalacion,sesion_host,proyecto_id,memoria_id,estado,origen_seleccion,revision,recuperacion,creada_en,actualizado_en) VALUES (?1,?2,?3,?4,'encendida','sesion_explicita',0,?5,?6,?6)",
                    params![install,session,project,memory,i64::from(modo=="memoria_activa"),time])?;
                let id = tx.last_insert_rowid();
                open_interval(&tx, id, memory, 0, "activacion_explicita", modo, time)?;
                id
            }
            Some((id, current, state, existing_project, revision)) => {
                if let Some(existing) = &existing_project {
                    ensure!(
                        existing == &project,
                        "este chat pertenece a otro proyecto registrado; abrir un chat nuevo"
                    );
                }
                if let Some(bound) = &current {
                    ensure!(
                        bound == memory,
                        "este chat ya esta vinculado a la memoria {bound}; apagar no lo libera. \
                         Para usar {memory} abrir un chat nuevo: el contexto actual ya contiene \
                         recuerdos de {bound} y no puede presentarse como aislado"
                    );
                }
                if state == "encendida" {
                    tx.commit()?;
                    let mut out = self.status(install, session, Some(cwd))?;
                    out["cambio"] = json!(
                        "ninguno: ya estaba encendida; usar modo para cambiar entre solo_registro y memoria_activa"
                    );
                    return Ok(out);
                }
                // Reactivar abre un intervalo nuevo desde la frontera actual.
                let next = revision + 1;
                tx.execute("UPDATE sesion_memoria SET memoria_id=?1,estado='encendida',origen_seleccion='sesion_explicita',proyecto_id=?2,revision=?3,recuperacion=?4,actualizado_en=?5 WHERE id=?6",
                    params![memory,project,next,i64::from(modo=="memoria_activa"),time,id])?;
                open_interval(
                    &tx,
                    id,
                    memory,
                    next,
                    "reactivacion_sin_backfill",
                    modo,
                    time,
                )?;
                id
            }
        };
        log(
            &tx,
            Some(id),
            "memoria_encendida",
            json!({"memoria":memory,"origen":"sesion_explicita","modo":modo}),
            time,
        )?;
        tx.commit()?;
        let mut out = self.status(install, session, Some(cwd))?;
        out["cambio"] = json!(format!(
            "memoria encendida en modo {modo} para este chat, desde la frontera actual"
        ));
        Ok(out)
    }

    /// Cambiar de modo cierra el intervalo y abre otro: queda verificable que el
    /// modo fue constante durante una corrida comparativa.
    pub fn set_mode(&mut self, install: &str, session: &str, modo: &str) -> Result<Value> {
        modo_valido(modo)?;
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (id, estado, revision, memoria): (i64, String, i64, Option<String>) = tx
            .query_row(
                "SELECT id,estado,revision,memoria_id FROM sesion_memoria WHERE instalacion=?1 AND sesion_host=?2",
                params![install, session],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .context("sesion desconocida")?;
        let memoria = memoria.context("la sesion no esta vinculada a ninguna memoria")?;
        ensure!(
            estado == "encendida",
            "la sesion esta apagada; encenderla explicitamente en el modo deseado"
        );
        let actual: String = tx.query_row(
            "SELECT CASE WHEN recuperacion=1 THEN 'memoria_activa' ELSE 'solo_registro' END FROM sesion_memoria WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        if actual == modo {
            tx.commit()?;
            let mut out = self.status(install, session, None)?;
            out["cambio"] = json!("ninguno: ya estaba en ese modo");
            return Ok(out);
        }
        let next = revision + 1;
        tx.execute(
            "UPDATE sesion_memoria SET recuperacion=?1,revision=?2,actualizado_en=?3 WHERE id=?4",
            params![i64::from(modo == "memoria_activa"), next, time, id],
        )?;
        tx.execute("UPDATE intervalo_captura SET estado='cerrado',cerrado_en=?1,frontera_cierre=?2 WHERE sesion_id=?3 AND estado='abierto'",
            params![time, json!({"motivo":"cambio_de_modo","modo_nuevo":modo}).to_string(), id])?;
        open_interval(&tx, id, &memoria, next, "cambio_de_modo", modo, time)?;
        log(
            &tx,
            Some(id),
            "modo_cambiado",
            json!({"anterior":actual,"nuevo":modo,"revision":next}),
            time,
        )?;
        tx.commit()?;
        let mut out = self.status(install, session, None)?;
        out["cambio"] = json!(format!(
            "modo {modo}; una corrida comparativa que cambia de modo queda no comparable"
        ));
        Ok(out)
    }

    /// Seleccionar tarea **no** enciende la memoria ni cambia el modo: solo fija
    /// a qué tarea se asociarán los eventos que se admitan de ahora en adelante.
    pub fn select_task(
        &mut self,
        install: &str,
        session: &str,
        clave: Option<&str>,
        version: Option<i64>,
    ) -> Result<Value> {
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (id, revision): (i64, i64) = tx
            .query_row(
                "SELECT id,tarea_revision FROM sesion_memoria WHERE instalacion=?1 AND sesion_host=?2",
                params![install, session],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .context("sesion desconocida; vincular una memoria primero")?;
        let next = revision + 1;
        tx.execute(
            "UPDATE sesion_memoria SET tarea_clave=?1,tarea_definicion_version=?2,tarea_revision=?3,actualizado_en=?4 WHERE id=?5",
            params![clave, version, next, time, id],
        )?;
        log(
            &tx,
            Some(id),
            if clave.is_some() {
                "tarea_seleccionada"
            } else {
                "tarea_quitada"
            },
            json!({"tarea":clave,"definicion_version":version,"revision":next}),
            time,
        )?;
        tx.commit()?;
        let mut out = self.status(install, session, None)?;
        out["cambio"] = json!(match clave {
            Some(_) => "tarea seleccionada; no enciende la memoria ni cambia el modo",
            None => "seleccion de tarea quitada",
        });
        Ok(out)
    }

    /// Revalida que la selección congelada siga vigente antes de enlazar.
    pub fn task_still_selected(&self, active: &Active) -> Result<bool> {
        let Some(t) = &active.tarea else {
            return Ok(false);
        };
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sesion_memoria WHERE id=?1 AND tarea_revision=?2 AND tarea_clave=?3)",
            params![active.sesion_id, t.revision, t.clave],
            |r| r.get(0),
        )?)
    }

    /// Apagar cierra el intervalo y sube la revisión: los callbacks pendientes de
    /// esa sesión quedan invalidados y no habrá capturas ni emisiones nuevas.
    pub fn turn_off(&mut self, install: &str, session: &str) -> Result<Value> {
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (id, state, revision): (i64, String, i64) = tx
            .query_row("SELECT id,estado,revision FROM sesion_memoria WHERE instalacion=?1 AND sesion_host=?2",
                params![install,session],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?
            .context("sesion desconocida; nada que apagar")?;
        if state == "apagada" {
            tx.commit()?;
            let mut out = self.status(install, session, None)?;
            out["cambio"] = json!("ninguno: ya estaba apagada");
            return Ok(out);
        }
        let next = revision + 1;
        tx.execute(
            "UPDATE sesion_memoria SET estado='apagada',revision=?1,actualizado_en=?2 WHERE id=?3",
            params![next, time, id],
        )?;
        tx.execute("UPDATE intervalo_captura SET estado='cerrado',cerrado_en=?1,frontera_cierre=?2 WHERE sesion_id=?3 AND estado='abierto'",
            params![time,json!({"motivo":"apagado_explicito","revision_cierre":next}).to_string(),id])?;
        log(
            &tx,
            Some(id),
            "memoria_apagada",
            json!({ "revision": next }),
            time,
        )?;
        tx.commit()?;
        let mut out = self.status(install, session, None)?;
        out["cambio"] =
            json!("memoria apagada; lo ya confirmado permanece y el contexto cargado no se borra");
        Ok(out)
    }

    pub fn status(&self, install: &str, session: &str, cwd: Option<&Path>) -> Result<Value> {
        let row: Option<EstadoRow> = self.conn
            .query_row("SELECT id,memoria_id,proyecto_id,estado,origen_seleccion,revision FROM sesion_memoria WHERE instalacion=?1 AND sesion_host=?2",
                params![install,session],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
        let observed = cwd.and_then(|p| normalize(p).ok());
        let plataforma = crate::plataformas::de_sesion(session);
        let mut out = json!({
            "instalacion":install,"sesion":session,"raiz_datos":self.root,
            "adaptador":plataforma.adaptador,"host_probado":plataforma.host_probado,
            "host_minimo":(plataforma.adaptador==ADAPTADOR_VERSION).then_some(HOST_MINIMO),
            "cwd_normalizado":observed.as_ref().map(|(k,_)|k.clone()),
            "limites":LIMITES,"captura":(plataforma.cobertura)()
        });
        let Some((id, memory, project, state, source, revision)) = row else {
            out["estado"] = json!("apagada");
            out["origen_seleccion"] = json!("sesion_desconocida");
            out["memoria"] = Value::Null;
            out["proyecto"] = Value::Null;
            out["modo"] = json!("apagada");
            out["tarea"] = Value::Null;
            out["operacion"] = json!("apagada: este chat todavia no tiene fila de control");
            return Ok(out);
        };
        let memory_name: Option<String> = match &memory {
            Some(m) => self
                .conn
                .query_row("SELECT nombre FROM memoria WHERE id=?1", [m], |r| r.get(0))
                .optional()?,
            None => None,
        };
        let project_root: Option<String> = match &project {
            Some(p) => self
                .conn
                .query_row(
                    "SELECT raiz_normalizada FROM proyecto_local WHERE id=?1",
                    [p],
                    |r| r.get(0),
                )
                .optional()?,
            None => None,
        };
        let interval: Option<(i64, i64, i64)> = self.conn.query_row(
            "SELECT id,revision_activacion,abierto_en FROM intervalo_captura WHERE sesion_id=?1 AND estado='abierto'",
            [id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let intervals: i64 = self.conn.query_row(
            "SELECT count(*) FROM intervalo_captura WHERE sesion_id=?1",
            [id],
            |r| r.get(0),
        )?;
        let assigned: i64 = self.conn.query_row(
            "SELECT count(*) FROM asignacion_evento a JOIN intervalo_captura i ON i.id=a.intervalo_id WHERE i.sesion_id=?1",
            [id],|r|r.get(0))?;
        let outside = match (&project, &observed) {
            (Some(p), Some((key, _))) => !en_proyecto(&self.conn, p, key)?,
            _ => false,
        };
        out["memoria"] = json!({"memoria_id":memory,"nombre":memory_name});
        out["proyecto"] = json!({"proyecto_id":project,"raiz_normalizada":project_root});
        out["estado"] = json!(state);
        out["origen_seleccion"] = json!(source);
        out["revision"] = json!(revision);
        out["intervalo_abierto"] = match interval {
            Some((iid, rev, opened)) => {
                json!({"id":iid,"revision_activacion":rev,"abierto_en":opened})
            }
            None => Value::Null,
        };
        out["intervalos_totales"] = json!(intervals);
        out["eventos_asignados"] = json!(assigned);
        let (recuperacion, tarea_clave, tarea_version, tarea_revision): (i64, Option<String>, Option<i64>, i64) =
            self.conn.query_row(
                "SELECT recuperacion,tarea_clave,tarea_definicion_version,tarea_revision FROM sesion_memoria WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        let modo = if state != "encendida" {
            "apagada"
        } else if recuperacion == 1 {
            "memoria_activa"
        } else {
            "solo_registro"
        };
        out["modo"] = json!(modo);
        out["modos_posibles"] = json!({
            "apagada": "sin captura ni recuperacion nuevas",
            "solo_registro": "captura y mide, pero no busca ni entrega antecedentes",
            "memoria_activa": "captura y ofrece antecedentes"
        });
        out["tarea"] = match &tarea_clave {
            Some(clave) => json!({"clave":clave,"definicion_version":tarea_version,
                "revision_seleccion":tarea_revision,
                "nota":"seleccionar tarea no enciende la memoria ni cambia el modo"}),
            None => Value::Null,
        };
        out["fuera_de_raiz_registrada"] = json!(outside);
        out["operacion"] = json!(if outside {
            "suspendida: el cwd salio de la raiz registrada; volver a ella o registrar el ambito"
        } else if modo == "memoria_activa" {
            "capturando y ofreciendo contexto dentro del intervalo abierto"
        } else if modo == "solo_registro" {
            "solo registro: captura y mide, sin buscar ni entregar antecedentes"
        } else {
            "apagada: sin captura ni entrega de contexto"
        });
        Ok(out)
    }

    /// Puerta única de los hooks. Consulta **solo** control y frontera: si está
    /// apagada devuelve None sin abrir la memoria ni conservar el payload.
    pub fn gate(
        &mut self,
        install: &str,
        session: &str,
        cwd: Option<&Path>,
    ) -> Result<Option<Active>> {
        let observed = cwd.and_then(|p| normalize(p).ok());
        let time = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<SesionRow> = tx
            .query_row("SELECT id,memoria_id,proyecto_id,estado,revision FROM sesion_memoria WHERE instalacion=?1 AND sesion_host=?2",
                params![install,session],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((id, Some(memory), project, state, revision)) = row else {
            tx.commit()?;
            return Ok(None);
        };
        // Modo y tarea se congelan acá, al admitir el callback. No se vuelve a
        // consultar «la tarea actual» cuando termina el trabajo diferido.
        let (recuperacion, tarea_clave, tarea_version, tarea_revision): (i64, Option<String>, Option<i64>, i64) =
            tx.query_row(
                "SELECT recuperacion,tarea_clave,tarea_definicion_version,tarea_revision FROM sesion_memoria WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
        if state != "encendida" {
            // Se registra que llegó un callback estando apagada; el detalle no
            // contiene nada del payload ni del transcript.
            log(
                &tx,
                Some(id),
                "callback_rechazado",
                json!({"motivo":"sesion_apagada","revision":revision}),
                time,
            )?;
            tx.commit()?;
            return Ok(None);
        }
        let project = project.context("sesion encendida sin proyecto registrado")?;
        let (root, relative): (String, String) = tx.query_row(
            "SELECT p.raiz_normalizada,m.archivo_relativo FROM proyecto_local p, memoria m WHERE p.id=?1 AND m.id=?2",
            params![project,memory],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if let Some((key, _)) = &observed {
            if !en_proyecto(&tx, &project, key)? {
                // Un `cd` fuera de las carpetas del proyecto suspende; nunca elige otro.
                log(
                    &tx,
                    Some(id),
                    "callback_rechazado",
                    json!({"motivo":"cwd_fuera_de_raiz_registrada"}),
                    time,
                )?;
                tx.commit()?;
                return Ok(None);
            }
        }
        let interval: Option<(i64, i64)> = tx.query_row(
            "SELECT id,revision_activacion FROM intervalo_captura WHERE sesion_id=?1 AND estado='abierto'",
            [id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((interval_id, activation)) = interval else {
            log(
                &tx,
                Some(id),
                "callback_rechazado",
                json!({"motivo":"sin_intervalo_abierto"}),
                time,
            )?;
            tx.commit()?;
            return Ok(None);
        };
        if activation != revision {
            log(
                &tx,
                Some(id),
                "callback_rechazado",
                json!({"motivo":"revision_desactualizada","intervalo":activation,"sesion":revision}),
                time,
            )?;
            tx.commit()?;
            return Ok(None);
        }
        let modo: String = tx.query_row(
            "SELECT modo FROM intervalo_captura WHERE id=?1",
            [interval_id],
            |r| r.get(0),
        )?;
        tx.commit()?;
        let file = self.file_of(&relative)?;
        Ok(Some(Active {
            sesion_id: id,
            intervalo_id: interval_id,
            revision,
            memoria_id: memory,
            archivo: file,
            proyecto: root,
            sesion_clave: format!("{install}/{session}"),
            modo: if recuperacion == 1 {
                modo
            } else {
                "solo_registro".to_string()
            },
            tarea: tarea_clave.map(|clave| TareaSeleccionada {
                clave,
                definicion_version: tarea_version.unwrap_or(0),
                revision: tarea_revision,
            }),
        }))
    }

    /// Asigna identidad y secuencia por clave idempotente **antes** del commit
    /// del crudo. Un replay del mismo payload reutiliza la asignación; dos
    /// eventos legítimos de texto idéntico tienen payloads distintos y por lo
    /// tanto claves y secuencias distintas.
    pub fn allocate(
        &mut self,
        active: &Active,
        idem: &str,
        key: &str,
        group: &str,
    ) -> Result<Assignment> {
        ensure!(idem.len() == 64, "clave idempotente invalida");
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Revalidar dentro de la transacción de escritura: una carrera con el
        // apagado no puede colarse entre la puerta y la asignación.
        let open: Option<i64> = tx
            .query_row(
                "SELECT i.id FROM intervalo_captura i JOIN sesion_memoria s ON s.id=i.sesion_id
             WHERE i.id=?1 AND i.estado='abierto' AND s.estado='encendida'
               AND s.revision=?2 AND i.revision_activacion=?2",
                params![active.intervalo_id, active.revision],
                |r| r.get(0),
            )
            .optional()?;
        let existing: Option<(String, String, i64, i64, i64)> = tx
            .query_row(
                "SELECT clave_evento,grupo_evidencia,secuencia,timestamp,intervalo_id FROM asignacion_evento WHERE clave_idem=?1",
                [idem],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        if let Some((existing_key, existing_group, sequence, timestamp, interval)) = existing {
            ensure!(
                interval == active.intervalo_id,
                "esa asignacion pertenece a otro intervalo de captura"
            );
            tx.commit()?;
            return Ok(Assignment {
                clave_evento: existing_key.clone(),
                grupo_evidencia: existing_group,
                secuencia: sequence,
                timestamp,
                nueva: false,
                observacion_secundaria: existing_key != key,
            });
        }
        ensure!(
            open.is_some(),
            "intervalo cerrado o revision cambiada: callback descartado sin capturar"
        );
        // El mismo ID de fuente con otro payload es una observacion distinta de
        // la misma operacion: se conserva con un sufijo derivado del payload en
        // vez de perderla. Un replay exacto ya se resolvio arriba por clave_idem.
        let taken: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM asignacion_evento WHERE clave_evento=?1)",
            [key],
            |r| r.get(0),
        )?;
        let key = if taken {
            format!("{key}#{}", &idem[..8])
        } else {
            key.to_string()
        };
        ensure!(
            !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM asignacion_evento WHERE clave_evento=?1)",
                [&key],
                |r| r.get::<_, bool>(0)
            )?,
            "identidad de fuente {key} ya asignada a otro payload"
        );
        let next: i64 = tx.query_row(
            "SELECT coalesce(max(secuencia),-1)+1 FROM asignacion_evento WHERE grupo_evidencia=?1",
            [group],
            |r| r.get(0),
        )?;
        let time = now()?;
        tx.execute(
            "INSERT INTO asignacion_evento VALUES (?1,?2,?3,?4,?5,?6,?6)",
            params![idem, active.intervalo_id, key, group, next, time],
        )?;
        tx.commit()?;
        Ok(Assignment {
            clave_evento: key,
            grupo_evidencia: group.to_string(),
            secuencia: next,
            timestamp: time,
            nueva: true,
            observacion_secundaria: taken,
        })
    }

    /// Avanza el cursor **después** del commit del crudo. Una caída intermedia
    /// deja el cursor atrás: el replay vuelve a confirmar sin duplicar.
    pub fn advance_cursor(&mut self, active: &Active, source: &str, position: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO cursor_ingesta VALUES (?1,?2,?3,1,?4)
             ON CONFLICT(intervalo_id,fuente) DO UPDATE SET
               posicion=excluded.posicion,confirmados=confirmados+1,actualizado_en=excluded.actualizado_en",
            params![active.intervalo_id, source, position, now()?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Reconfirma que la sesión sigue encendida en la misma revisión. Se llama
    /// justo antes de escribir al canal del host; no es una lectura de booleano
    /// hecha al empezar una tarea larga.
    pub fn still_active(&self, active: &Active) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM intervalo_captura i JOIN sesion_memoria s ON s.id=i.sesion_id
             WHERE i.id=?1 AND i.estado='abierto' AND s.estado='encendida' AND s.revision=?2)",
            params![active.intervalo_id, active.revision],
            |r| r.get(0),
        )?)
    }

    pub fn close_open_interval(
        &mut self,
        install: &str,
        session: &str,
        reason: &str,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE intervalo_captura SET estado='cerrado',cerrado_en=?1,frontera_cierre=?2
             WHERE estado='abierto' AND sesion_id=(SELECT id FROM sesion_memoria WHERE instalacion=?3 AND sesion_host=?4)",
            params![now()?, json!({ "motivo": reason }).to_string(), install, session],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn history(&self, install: &str, session: &str) -> Result<Value> {
        let id: i64 = self
            .conn
            .query_row(
                "SELECT id FROM sesion_memoria WHERE instalacion=?1 AND sesion_host=?2",
                params![install, session],
                |r| r.get(0),
            )
            .optional()?
            .context("sesion desconocida")?;
        let control: Vec<Value> = {
            let mut stmt = self.conn.prepare(
                "SELECT accion,detalle_json,registrado_en FROM control_historial WHERE sesion_id=?1 ORDER BY id",
            )?;
            let rows = stmt.query_map([id], |r| {
                Ok(json!({"accion":r.get::<_,String>(0)?,
                    "detalle":serde_json::from_str::<Value>(&r.get::<_,String>(1)?).unwrap_or(Value::Null),
                    "registrado_en":r.get::<_,i64>(2)?}))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let intervals: Vec<Value> = {
            let mut stmt = self.conn.prepare(
                "SELECT i.id,i.revision_activacion,i.estado,i.frontera_apertura,i.frontera_cierre,i.abierto_en,i.cerrado_en,
                        (SELECT count(*) FROM asignacion_evento a WHERE a.intervalo_id=i.id),
                        (SELECT coalesce(sum(c.confirmados),0) FROM cursor_ingesta c WHERE c.intervalo_id=i.id)
                 FROM intervalo_captura i WHERE i.sesion_id=?1 ORDER BY i.id",
            )?;
            let rows = stmt.query_map([id], |r| {
                Ok(json!({"intervalo_id":r.get::<_,i64>(0)?,"revision_activacion":r.get::<_,i64>(1)?,
                    "estado":r.get::<_,String>(2)?,
                    "frontera_apertura":serde_json::from_str::<Value>(&r.get::<_,String>(3)?).unwrap_or(Value::Null),
                    "frontera_cierre":r.get::<_,Option<String>>(4)?.and_then(|s|serde_json::from_str::<Value>(&s).ok()),
                    "abierto_en":r.get::<_,i64>(5)?,"cerrado_en":r.get::<_,Option<i64>>(6)?,
                    "eventos_asignados":r.get::<_,i64>(7)?,"cursor_confirmados":r.get::<_,i64>(8)?}))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        Ok(
            json!({"sesion":format!("{install}/{session}"),"intervalos":intervals,"control":control}),
        )
    }

    /// Sólo SessionEnd, dentro de esta memoria y proyecto. Apagar o cambiar de
    /// modo no es terminar el chat y no se convierte en un cierre automático.
    pub fn cierres_para_episodios(
        &self,
        memoria: &str,
        proyecto: &str,
        chat: &str,
    ) -> Result<Vec<crate::episodios_automaticos::CierreChat>> {
        check_id(memoria)?;
        // El primer segmento del grupo es la plataforma, y tiene que ser la de
        // la sesión: los cierres de un host no se atribuyen al chat de otro.
        let plataforma = chat.split_once('/').map_or("", |(nombre, _)| nombre);
        let mut consulta = self.conn.prepare(
            "SELECT i.id,i.cerrado_en,s.sesion_host FROM intervalo_captura i JOIN sesion_memoria s ON s.id=i.sesion_id
             JOIN proyecto_local p ON p.id=s.proyecto_id
             WHERE i.memoria_id=?1 AND p.raiz_normalizada=?2
               AND substr(?3,instr(?3,'/')+1)=s.instalacion||'/'||s.sesion_host
               AND i.estado='cerrado' AND json_extract(i.frontera_cierre,'$.motivo') GLOB 'sesion_terminada:*'
             ORDER BY i.id")?;
        let filas = consulta.query_map(params![memoria, proyecto, chat], |r| {
            Ok((
                crate::episodios_automaticos::CierreChat {
                    intervalo_id: r.get(0)?,
                    cerrado_en: r.get(1)?,
                },
                r.get::<_, String>(2)?,
            ))
        })?;
        let mut cierres = Vec::new();
        for fila in filas {
            let (cierre, sesion) = fila?;
            if crate::plataformas::de_sesion(&sesion).nombre == plataforma {
                cierres.push(cierre);
            }
        }
        Ok(cierres)
    }
}

fn project_of(tx: &Connection, key: &str) -> Result<String> {
    // La carpeta más larga que contenga al cwd, de cualquier proyecto: un
    // subdirectorio no elige otro proyecto y el basename nunca participa.
    let mut stmt = tx.prepare(
        "SELECT proyecto_id,raiz_normalizada FROM proyecto_carpeta ORDER BY length(raiz_normalizada) DESC",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, root) = row?;
        if inside(&root, key) {
            return Ok(id);
        }
    }
    bail!("raiz no registrada para {key}")
}

fn open_interval(
    tx: &Transaction<'_>,
    session: i64,
    memory: &str,
    revision: i64,
    reason: &str,
    modo: &str,
    time: i64,
) -> Result<()> {
    // La frontera de apertura es la actual: no se importa historial previo. El
    // modo queda fijado en el intervalo, así una corrida puede comprobarse
    // constante sin depender de que nadie lo haya cambiado a mitad de camino.
    tx.execute(
        "INSERT INTO intervalo_captura(sesion_id,memoria_id,revision_activacion,frontera_apertura,estado,abierto_en,modo) VALUES (?1,?2,?3,?4,'abierto',?5,?6)",
        params![session, memory, revision,
            json!({"motivo":reason,"desde":"frontera_actual","importa_historial_previo":false,"abierto_en":time,"modo":modo}).to_string(),
            time, modo],
    )?;
    Ok(())
}

fn log(
    tx: &Transaction<'_>,
    session: Option<i64>,
    action: &str,
    detail: Value,
    time: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO control_historial(sesion_id,accion,detalle_json,registrado_en) VALUES (?1,?2,?3,?4)",
        params![session, action, detail.to_string(), time],
    )?;
    Ok(())
}

/// Tabla de cobertura declarada: fuente, completitud y límite por evento.
pub fn cobertura() -> Value {
    json!([
        {"hook":"SessionStart","captura":"ninguna","control":"materializa la fila de sesion y aplica el predeterminado",
         "limite":"resume/fork llegan con su propio session_id; una sesion nueva no copia vinculos por compartir transcript"},
        {"hook":"UserPromptSubmit","captura":"texto del prompt del usuario","campo":"prompt","origen":"usuario","rol":"problema",
         "completitud":"completa segun el host","limite":"es tambien el unico punto donde se ofrece contexto"},
        {"hook":"PostToolUse","captura":"entrada y salida de la herramienta","campo":"tool_input + tool_response","origen":"herramienta","rol":"otro",
         "completitud":"lo que el host serializa","limite":"ejecutar una herramienta no valida nada"},
        {"hook":"PostToolUseFailure","captura":"error de la herramienta","campo":"tool_input + error","origen":"herramienta","rol":"otro",
         "completitud":"mensaje de error del host","limite":"un fallo de herramienta no prueba fallo global"},
        {"hook":"Stop","captura":"texto final del asistente del turno","campo":"last_assistant_message","origen":"modelo","rol":"solucion",
         "completitud":"solo el mensaje final","limite":"fin de turno no prueba exito"},
        {"hook":"SubagentStop","captura":"texto final del subagente","campo":"last_assistant_message + agent_id","origen":"modelo","rol":"otro",
         "completitud":"solo si el host aporta agent_id","limite":"el padre se toma del prompt_id explicito; nunca por cercania temporal"},
        {"hook":"PreCompact","captura":"marcador de frontera, sin contenido","origen":"herramienta","rol":"otro",
         "completitud":"solo el disparador","limite":"compactar no crea otra seleccion ni reimporta lo anterior"},
        {"hook":"SessionEnd","captura":"ninguna","control":"cierra el intervalo abierto",
         "limite":"un cierre por caida del host queda pendiente hasta la proxima orden explicita"},
        {"fuente":"transcript_path","uso":"no se lee en esta tanda",
         "limite":"mensajes intermedios del asistente, razonamiento y bloques que los hooks no exponen quedan fuera de la captura"}
    ])
}
