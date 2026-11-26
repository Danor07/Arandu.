//! SZZ: de un arreglo al commit que introdujo el error y al episodio que lo produjo.
//! El vínculo es probable; sólo uno confirmado resta evidencia. Límites en README.
use crate::execution::{nonempty, rows};
use crate::{now, Store};
use anyhow::{bail, ensure, Context, Result};
use rusqlite::{params, TransactionBehavior};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

pub const SZZ_VERSION: &str = "szz-blame-commit-en-captura-v1";

fn hex(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `git` sin shell, sin paginador y sin dejar que la configuración del repo ejecute nada.
fn git(repo: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let salida = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["--no-pager", "-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .context("no se pudo ejecutar git")?;
    ensure!(
        salida.status.success(),
        "git {} fallo: {}",
        args.first().copied().unwrap_or_default(),
        String::from_utf8_lossy(&salida.stderr).trim()
    );
    Ok(salida.stdout)
}

fn linea(bytes: Vec<u8>) -> Result<String> {
    Ok(String::from_utf8(bytes)?.trim().to_string())
}

/// Rangos `(inicio, cantidad)` del lado viejo de un diff `-U0`: las líneas que el arreglo
/// borró o reemplazó. Sólo mira encabezados `@@`; el contenido del diff no se interpreta.
pub fn rangos_viejos(diff: &[u8]) -> Vec<(usize, usize)> {
    let mut rangos = Vec::new();
    for linea in diff.split(|b| *b == b'\n') {
        let Some(resto) = linea.strip_prefix(b"@@ -") else {
            continue;
        };
        let viejo: Vec<u8> = resto.iter().copied().take_while(|b| *b != b' ').collect();
        let Ok(viejo) = std::str::from_utf8(&viejo) else {
            continue;
        };
        let (inicio, cantidad) = viejo.split_once(',').unwrap_or((viejo, "1"));
        if let (Ok(inicio), Ok(cantidad)) = (inicio.parse::<usize>(), cantidad.parse::<usize>()) {
            if inicio > 0 && cantidad > 0 {
                rangos.push((inicio, cantidad));
            }
        }
    }
    rangos
}

/// Líneas por commit en la salida de `git blame --line-porcelain`. Un registro es una
/// línea `<hash> <línea original> <línea final> [<cantidad>]`. El contenido no puede pasar
/// por uno porque siempre empieza con tabulador, ni un encabezado porque empieza con su nombre.
pub fn parsear_blame(salida: &[u8]) -> BTreeMap<String, usize> {
    let mut lineas = BTreeMap::new();
    for linea in salida.split(|b| *b == b'\n') {
        let Ok(texto) = std::str::from_utf8(linea) else {
            continue;
        };
        let mut partes = texto.split(' ');
        let (hash, resto): (&str, Vec<&str>) =
            (partes.next().unwrap_or_default(), partes.collect());
        if [40, 64].contains(&hash.len())
            && hex(hash)
            && resto.len() >= 2
            && resto.iter().all(|n| n.parse::<usize>().is_ok())
        {
            *lineas.entry(hash.to_ascii_lowercase()).or_default() += 1;
        }
    }
    lineas
}

/// Hashes (o prefijos de 7 o más) que la salida de un `git commit` muestra entre corchetes,
/// como `[rama abc1234] mensaje`. Sólo si la captura es un `Bash` cuyo comando dice `git commit`.
pub fn hashes_de_captura(payload: &Value) -> Vec<String> {
    let comando = payload["tool_input"]["command"]
        .as_str()
        .unwrap_or_default();
    if payload["tool_name"] != "Bash" || !comando.contains("git commit") {
        return Vec::new();
    }
    let respuesta = match &payload["tool_response"] {
        Value::String(s) => s.clone(),
        otro => otro["stdout"].as_str().unwrap_or_default().to_string(),
    };
    let mut hashes = Vec::new();
    for tramo in respuesta.split('[').skip(1) {
        let dentro = tramo.split(']').next().unwrap_or_default();
        if let Some(ultimo) = dentro.split_whitespace().last() {
            if (7..=64).contains(&ultimo.len()) && hex(ultimo) {
                hashes.push(ultimo.to_ascii_lowercase());
            }
        }
    }
    hashes
}

/// Commit señalado por blame → [(archivo, líneas)].
type Senalados = BTreeMap<String, Vec<(String, usize)>>;

/// (hash completo del arreglo, commits señalados, archivos con nombre no UTF-8 omitidos).
fn senalados(repo: &Path, arreglo: &str) -> Result<(String, Senalados, Vec<String>)> {
    // Un valor que empieza con «-» sería una opción de git, no una revisión.
    ensure!(
        !arreglo.is_empty() && !arreglo.starts_with('-') && !arreglo.contains(char::is_whitespace),
        "revision invalida"
    );
    let commit = linea(git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{arreglo}^{{commit}}"),
        ],
    )?)?;
    ensure!(
        [40, 64].contains(&commit.len()) && hex(&commit),
        "git no devolvio un commit"
    );
    let padres = linea(git(repo, &["rev-list", "--parents", "-n", "1", &commit])?)?;
    let padres: Vec<&str> = padres.split(' ').skip(1).collect();
    ensure!(
        padres.len() == 1,
        "el arreglo tiene {} padres: se rastrea solo un commit con un unico padre",
        padres.len()
    );
    let padre = padres[0];
    let comunes = [
        "--no-ext-diff",
        "--no-textconv",
        "--no-renames",
        "--no-color",
    ];
    let nombres = git(
        repo,
        &[
            &["diff", "--name-only", "-z", "--diff-filter=M"],
            &comunes[..],
            &[padre, &commit],
        ]
        .concat(),
    )?;
    let mut por_commit = Senalados::new();
    let mut omitidos = Vec::new();
    for nombre in nombres.split(|b| *b == 0).filter(|n| !n.is_empty()) {
        let Ok(archivo) = std::str::from_utf8(nombre) else {
            omitidos.push(String::from_utf8_lossy(nombre).into_owned());
            continue;
        };
        let diff = git(
            repo,
            &[
                &["diff", "-U0"],
                &comunes[..],
                &[padre, &commit, "--", archivo],
            ]
            .concat(),
        )?;
        let rangos: Vec<String> = rangos_viejos(&diff)
            .iter()
            .map(|(inicio, cantidad)| format!("-L{inicio},+{cantidad}"))
            .collect();
        if rangos.is_empty() {
            continue;
        }
        let mut args = vec!["blame", "--line-porcelain"];
        args.extend(rangos.iter().map(String::as_str));
        args.extend([padre, "--", archivo]);
        for (hash, lineas) in parsear_blame(&git(repo, &args)?) {
            por_commit
                .entry(hash)
                .or_default()
                .push((archivo.to_string(), lineas));
        }
    }
    Ok((commit, por_commit, omitidos))
}

const LEER: &str = "SELECT v.*,d.decision,d.registrado_en AS decidido_en,
    coalesce(d.decision,'propuesto') AS estado FROM szz_vinculo v LEFT JOIN szz_decision d ON d.vinculo_id=v.id";

impl Store {
    /// Propone vínculos; no resta nada. Repetirlo no duplica ni reabre lo ya decidido.
    pub fn szz_rastrear(&mut self, proyecto: &str, repo: &Path, arreglo: &str) -> Result<Value> {
        nonempty(proyecto)?;
        let (commit, por_commit, omitidos) = senalados(repo, arreglo)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut sin_episodio = Vec::new();
        for (senalado, archivos) in &por_commit {
            let capturas = rows(
                &tx,
                "SELECT i.id,json_extract(i.metadatos_json,'$.payload_hook') AS payload FROM instancia i
                 JOIN instancia_proyecto p ON p.instancia_id=i.id
                 WHERE p.proyecto=?1 AND instr(i.metadatos_json,?2)>0 ORDER BY i.id",
                params![proyecto, &senalado[..7]],
            )?;
            for captura in capturas {
                let payload: Value =
                    serde_json::from_str(captura["payload"].as_str().unwrap_or("null"))?;
                if !hashes_de_captura(&payload)
                    .iter()
                    .any(|h| senalado.starts_with(h))
                {
                    continue;
                }
                let archivos: Vec<Value> = archivos
                    .iter()
                    .map(|(archivo, lineas)| json!({"archivo":archivo,"lineas":lineas}))
                    .collect();
                tx.execute(
                    "INSERT OR IGNORE INTO szz_vinculo(proyecto,arreglo_commit,senalado_commit,archivos_json,instancia_id,origen,episodio_id,registrado_en)
                     SELECT ?1,?2,?3,?4,?5,c.origen,c.episodio_id,?6 FROM objetivo_episodio_captura c WHERE c.instancia_id=?5",
                    params![proyecto, commit, senalado, serde_json::to_string(&archivos)?, captura["id"].as_i64(), now()?],
                )?;
            }
            if tx.query_row(
                "SELECT count(*) FROM szz_vinculo WHERE proyecto=?1 AND arreglo_commit=?2 AND senalado_commit=?3",
                params![proyecto, commit, senalado],
                |r| r.get::<_, i64>(0),
            )? == 0
            {
                sin_episodio.push(senalado);
            }
        }
        let vinculos = rows(
            &tx,
            &format!("{LEER} WHERE v.proyecto=?1 AND v.arreglo_commit=?2 ORDER BY v.id"),
            params![proyecto, commit],
        )?;
        tx.commit()?;
        Ok(
            json!({"regla":SZZ_VERSION,"arreglo":commit,"vinculos":vinculos,
            "senalados_sin_episodio":sin_episodio,"archivos_omitidos":omitidos,
            "alcance":"vinculo probable: blame senala lineas, no culpas; solo un vinculo confirmado resta evidencia"}),
        )
    }

    pub fn szz_listar(&mut self, proyecto: &str) -> Result<Value> {
        nonempty(proyecto)?;
        Ok(json!({"regla":SZZ_VERSION,"vinculos":rows(
            &self.conn,
            &format!("{LEER} WHERE v.proyecto=?1 ORDER BY v.id"),
            [proyecto],
        )?}))
    }

    /// `confirmado` o `descartado`, una sola vez. Desde su fecha, un vínculo confirmado
    /// cuenta como un fallo del camino de ese episodio.
    pub fn szz_decidir(&mut self, proyecto: &str, vinculo: i64, decision: &str) -> Result<Value> {
        nonempty(proyecto)?;
        if !["confirmado", "descartado"].contains(&decision) {
            bail!("decision invalida");
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            tx.query_row(
                "SELECT count(*) FROM szz_vinculo WHERE id=?1 AND proyecto=?2",
                params![vinculo, proyecto],
                |r| r.get::<_, i64>(0)
            )? == 1,
            "vinculo inexistente en el proyecto"
        );
        tx.execute(
            "INSERT INTO szz_decision VALUES (?1,?2,?3)",
            params![vinculo, decision, now()?],
        )?;
        let fila = rows(&tx, &format!("{LEER} WHERE v.id=?1"), [vinculo])?.remove(0);
        tx.commit()?;
        Ok(fila)
    }
}
