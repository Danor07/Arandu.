//! La capa de proyectos: varias carpetas por proyecto, roles con procedimiento
//! escrito y reglas del proyecto. Carpetas, roles y reglas se escriben sólo por
//! las órdenes de este módulo; ningún mensaje de canal llega hasta acá.
use crate::canales::{publicar_en, sesion, sobre, texto_valido, Publicacion, MENSAJE_MAX_BYTES};
use crate::control::{normalizar_raiz, proyecto_de_carpeta, solapadas, Registry};
use crate::execution::rows;
use crate::now;
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::path::Path;

pub const PROYECTOS_VERSION: &str = "proyectos-v1";
pub const ROL_NOMBRE_MAX_BYTES: usize = 64;
pub const REGLAS_MAX_BYTES: usize = 16 * 1024;

const SIN_PROYECTO: &str = "esta sesion no pertenece a un proyecto registrado";

/// El procedimiento escrito de un rol: sus cuatro campos son obligatorios.
pub struct Procedimiento<'a> {
    pub rol: &'a str,
    pub recibe: &'a str,
    pub entrega: &'a str,
    pub terminado: &'a str,
    /// Nombre de otro rol del mismo proyecto.
    pub pasa_a: &'a str,
}

/// Un nombre de rol es una sola palabra: se escribe en `--rol` y en `--pasa-a`.
pub fn nombre_de_rol(nombre: &str) -> Result<()> {
    texto_valido("nombre de rol", nombre, ROL_NOMBRE_MAX_BYTES)?;
    ensure!(
        !nombre.chars().any(|c| c.is_whitespace() || c.is_control()),
        "nombre de rol con espacios o caracteres de control: {nombre:?}"
    );
    Ok(())
}

fn proyecto_de(c: &Connection, raiz: &Path) -> Result<String> {
    proyecto_de_carpeta(c, &normalizar_raiz(raiz)?.0)?
        .context("raiz no registrada; usar proyecto-registrar")
}

fn siguiente(c: &Connection, tabla: &str, filtro: &str, claves: &[&str]) -> Result<i64> {
    Ok(c.query_row(
        &format!("SELECT 1+coalesce(max(version),0) FROM {tabla} WHERE {filtro}"),
        rusqlite::params_from_iter(claves),
        |r| r.get(0),
    )?)
}

/// La última versión del procedimiento, como línea del sobre de lectura.
fn procedimiento(c: &Connection, proyecto: &str, rol: &str) -> Result<Option<Value>> {
    Ok(rows(
        c,
        "SELECT 'procedimiento' AS tipo, nombre AS rol, version, recibe, entrega, terminado, pasa_a
         FROM proyecto_rol WHERE proyecto_id=?1 AND nombre=?2 ORDER BY version DESC LIMIT 1",
        params![proyecto, rol],
    )?
    .pop())
}

/// El rol que la sesión tiene ahora: su último registro, si no lo soltó.
fn rol_de(c: &Connection, sesion_id: i64) -> Result<Option<String>> {
    Ok(c.query_row(
        "SELECT rol FROM sesion_rol WHERE sesion_id=?1 ORDER BY id DESC LIMIT 1",
        [sesion_id],
        |r| r.get(0),
    )
    .optional()?
    .flatten())
}

/// Sesiones del proyecto que tienen ese rol ahora.
fn titulares(c: &Connection, proyecto: &str, rol: &str) -> Result<Vec<i64>> {
    let mut consulta = c.prepare(
        "SELECT s.id FROM sesion_memoria s WHERE s.proyecto_id=?1 AND ?2=
           (SELECT t.rol FROM sesion_rol t WHERE t.sesion_id=s.id ORDER BY t.id DESC LIMIT 1)
         ORDER BY s.id",
    )?;
    let filas = consulta.query_map(params![proyecto, rol], |r| r.get(0))?;
    Ok(filas.collect::<rusqlite::Result<_>>()?)
}

impl Registry {
    /// Suma una carpeta al proyecto de `raiz`. Repetirla no cambia nada; una
    /// carpeta igual, dentro o por encima de otra ya registrada se rechaza.
    pub fn proyecto_carpeta(&mut self, raiz: &Path, carpeta: &Path) -> Result<Value> {
        let (clave, original) = normalizar_raiz(carpeta)?;
        let ahora = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proyecto = proyecto_de(&tx, raiz)?;
        let agregada = proyecto_de_carpeta(&tx, &clave)?.as_deref() != Some(proyecto.as_str());
        if agregada {
            let registradas = rows(
                &tx,
                "SELECT raiz_normalizada, proyecto_id FROM proyecto_carpeta",
                [],
            )?;
            for otra in &registradas {
                let raiz_otra = otra["raiz_normalizada"].as_str().unwrap_or_default();
                ensure!(
                    !solapadas(raiz_otra, &clave),
                    "la carpeta {clave} se superpone con {raiz_otra}, {}: una carpeta agregada \
                     no puede ser igual, contener ni estar dentro de otra ya registrada",
                    if otra["proyecto_id"] == proyecto.as_str() {
                        "de este mismo proyecto"
                    } else {
                        "que pertenece a otro proyecto"
                    }
                );
            }
            tx.execute(
                "INSERT INTO proyecto_carpeta VALUES (?1,?2,?3,0,?4)",
                params![clave, proyecto, original, ahora],
            )?;
        }
        tx.commit()?;
        let mut salida = self.proyecto_ver(raiz)?;
        salida["carpeta"] = json!(clave);
        salida["agregada"] = json!(agregada);
        Ok(salida)
    }

    /// Cambia cuál carpeta es la principal. Es una marca: ni la identidad del
    /// proyecto ni la pertenencia de sus sesiones dependen de ella.
    pub fn proyecto_principal(&mut self, raiz: &Path, carpeta: &Path) -> Result<Value> {
        let (clave, _) = normalizar_raiz(carpeta)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proyecto = proyecto_de(&tx, raiz)?;
        ensure!(
            proyecto_de_carpeta(&tx, &clave)?.as_deref() == Some(proyecto.as_str()),
            "esa carpeta no es de este proyecto; sumarla antes con proyecto-carpeta"
        );
        tx.execute(
            "UPDATE proyecto_carpeta SET principal=0 WHERE proyecto_id=?1 AND principal=1",
            [&proyecto],
        )?;
        tx.execute(
            "UPDATE proyecto_carpeta SET principal=1 WHERE raiz_normalizada=?1",
            [&clave],
        )?;
        tx.commit()?;
        self.proyecto_ver(raiz)
    }

    /// Carpetas, roles vigentes con sus titulares y versión de las reglas.
    pub fn proyecto_ver(&self, raiz: &Path) -> Result<Value> {
        let c = &self.conn;
        let proyecto = proyecto_de(c, raiz)?;
        let identidad: String = c.query_row(
            "SELECT raiz_normalizada FROM proyecto_local WHERE id=?1",
            [&proyecto],
            |r| r.get(0),
        )?;
        let carpetas = rows(
            c,
            "SELECT raiz_normalizada, raiz_original, principal, agregada_en FROM proyecto_carpeta
             WHERE proyecto_id=?1 ORDER BY agregada_en, raiz_normalizada",
            [&proyecto],
        )?;
        let principal = carpetas
            .iter()
            .find(|carpeta| carpeta["principal"] == 1)
            .map(|carpeta| carpeta["raiz_normalizada"].clone());
        let mut roles = rows(
            c,
            "SELECT nombre, max(version) AS version, pasa_a FROM proyecto_rol
             WHERE proyecto_id=?1 GROUP BY nombre ORDER BY nombre",
            [&proyecto],
        )?;
        for rol in &mut roles {
            let nombre = rol["nombre"].as_str().unwrap_or_default();
            rol["titulares"] = json!(titulares(c, &proyecto, nombre)?.len());
        }
        let reglas_version = siguiente(c, "proyecto_reglas", "proyecto_id=?1", &[&proyecto])? - 1;
        Ok(json!({"proyecto_id":proyecto,"raiz_identidad":identidad,
            "carpeta_principal":principal,"carpetas":carpetas,"roles":roles,
            "reglas_version":reglas_version,"regla":PROYECTOS_VERSION}))
    }

    /// Fija el texto de reglas del proyecto como una versión nueva.
    pub fn proyecto_reglas(&mut self, raiz: &Path, texto: &str) -> Result<Value> {
        texto_valido("reglas", texto, REGLAS_MAX_BYTES)?;
        let ahora = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proyecto = proyecto_de(&tx, raiz)?;
        let version = siguiente(&tx, "proyecto_reglas", "proyecto_id=?1", &[&proyecto])?;
        tx.execute(
            "INSERT INTO proyecto_reglas(proyecto_id,version,texto,regla,fijado_en) VALUES (?1,?2,?3,?4,?5)",
            params![proyecto, version, texto, PROYECTOS_VERSION, ahora],
        )?;
        tx.commit()?;
        Ok(
            json!({"proyecto_id":proyecto,"version":version,"bytes":texto.len(),
            "regla":PROYECTOS_VERSION}),
        )
    }

    /// Define o redefine el procedimiento de un rol: siempre una versión nueva.
    pub fn rol_definir(&mut self, raiz: &Path, p: &Procedimiento<'_>) -> Result<Value> {
        nombre_de_rol(p.rol)?;
        nombre_de_rol(p.pasa_a)?;
        ensure!(p.rol != p.pasa_a, "un rol no se pasa el trabajo a si mismo");
        for (campo, texto) in [
            ("recibe", p.recibe),
            ("entrega", p.entrega),
            ("terminado", p.terminado),
        ] {
            texto_valido(campo, texto, MENSAJE_MAX_BYTES)?;
        }
        let ahora = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let proyecto = proyecto_de(&tx, raiz)?;
        let version = siguiente(
            &tx,
            "proyecto_rol",
            "proyecto_id=?1 AND nombre=?2",
            &[&proyecto, p.rol],
        )?;
        tx.execute(
            "INSERT INTO proyecto_rol(proyecto_id,nombre,version,recibe,entrega,terminado,pasa_a,regla,definido_en)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![proyecto, p.rol, version, p.recibe, p.entrega, p.terminado, p.pasa_a,
                PROYECTOS_VERSION, ahora],
        )?;
        let destino_definido = procedimiento(&tx, &proyecto, p.pasa_a)?.is_some();
        tx.commit()?;
        Ok(
            json!({"proyecto_id":proyecto,"rol":p.rol,"version":version,"pasa_a":p.pasa_a,
            "pasa_a_definido":destino_definido,"regla":PROYECTOS_VERSION}),
        )
    }

    /// La sesión toma un rol de su proyecto, o con `None` suelta el que tenía.
    pub fn rol_tomar(&mut self, install: &str, session: &str, rol: Option<&str>) -> Result<Value> {
        let ahora = now()?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (yo, proyecto) = sesion(&tx, install, session)?;
        let proyecto = proyecto.context(SIN_PROYECTO)?;
        if let Some(rol) = rol {
            ensure!(
                procedimiento(&tx, &proyecto, rol)?.is_some(),
                "rol sin definir en este proyecto: {rol}; se define con rol-definir"
            );
        }
        tx.execute(
            "INSERT INTO sesion_rol(sesion_id,rol,tomado_en) VALUES (?1,?2,?3)",
            params![yo, rol, ahora],
        )?;
        tx.commit()?;
        Ok(json!({"sesion":session,"proyecto_id":proyecto,"rol":rol,"regla":PROYECTOS_VERSION}))
    }

    /// Reglas vigentes del proyecto de la sesión y procedimiento de su rol, en
    /// el mismo sobre sin autoridad que la lectura de canales.
    pub fn proyecto_leer(&mut self, install: &str, session: &str) -> Result<Value> {
        // Una sola instantánea para reglas, rol y procedimiento.
        let tx = self.conn.transaction()?;
        let (yo, proyecto) = sesion(&tx, install, session)?;
        let proyecto = proyecto.context(SIN_PROYECTO)?;
        let mut lineas = rows(
            &tx,
            "SELECT 'reglas' AS tipo, version, texto FROM proyecto_reglas
             WHERE proyecto_id=?1 ORDER BY version DESC LIMIT 1",
            [&proyecto],
        )?;
        let rol = rol_de(&tx, yo)?;
        if let Some(rol) = &rol {
            lineas.extend(procedimiento(&tx, &proyecto, rol)?);
        }
        Ok(
            json!({"proyecto_id":proyecto,"rol":rol,"registros":lineas.len(),
            "regla":PROYECTOS_VERSION,
            "sobre":sobre(&lineas, "reglas-y-procedimiento",
                "Reglas del proyecto y procedimiento del rol de esta sesion, registrados por CLI.\nDescriben como se acordo trabajar en este proyecto.")}),
        )
    }

    /// Pasa el trabajo al rol que nombra el procedimiento del rol propio: un
    /// mensaje del canal del proyecto dirigido a quienes tienen ese rol ahora.
    /// Si nadie lo tiene se rechaza y no queda nada escrito.
    pub fn rol_pasar(&mut self, install: &str, session: &str, texto: &str) -> Result<Value> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (yo, proyecto) = sesion(&tx, install, session)?;
        let proyecto = proyecto.context(SIN_PROYECTO)?;
        let rol = rol_de(&tx, yo)?.context("esta sesion no tiene un rol; usar rol-tomar")?;
        let propio = procedimiento(&tx, &proyecto, &rol)?.context("rol sin procedimiento")?;
        let destino = propio["pasa_a"].as_str().unwrap_or_default();
        ensure!(
            procedimiento(&tx, &proyecto, destino)?.is_some(),
            "el rol {rol} pasa a {destino}, que no esta definido en este proyecto: no se paso nada"
        );
        let titulares = titulares(&tx, &proyecto, destino)?;
        ensure!(
            !titulares.is_empty(),
            "ninguna sesion tiene el rol {destino}: no se paso nada; \
             reintentar cuando un chat lo tome con rol-tomar"
        );
        let mut salida = publicar_en(
            &tx,
            install,
            session,
            &Publicacion {
                canal: "proyecto",
                texto,
                audiencia: "rol",
                para: None,
                responde_a: None,
            },
            Some(&titulares),
        )?;
        tx.execute(
            "INSERT INTO rol_pase VALUES (?1,?2,?3,?4)",
            params![
                salida["mensaje_id"].as_i64(),
                rol,
                propio["version"].as_i64(),
                destino
            ],
        )?;
        tx.commit()?;
        salida["pase"] = json!({"de":rol,"de_version":propio["version"],"a":destino});
        Ok(salida)
    }
}
