//! Instantáneas de tramos de un chat, independientes de las tareas manuales.
use crate::cortes::{detectar_tramos, EventoCorte};
use crate::episodios::{hash_secuencia, normalizar_herramienta, NORMALIZADOR_VERSION};
use crate::execution::{nonempty, rows};
use crate::{now, Store};
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, TransactionBehavior};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const DETECTOR_VERSION: &str = "cortes-lista-v1";

/// Dato de intervalo_captura, no un hook crudo reconstruido.
#[derive(Debug, Clone)]
pub struct CierreChat {
    pub intervalo_id: i64,
    pub cerrado_en: i64,
}

fn listar(c: &Connection, proyecto: &str, chat: &str) -> Result<Value> {
    let mut episodios = rows(c,
        "SELECT e.* FROM episodio_automatico e JOIN episodio_automatico_sello s ON s.episodio_id=e.id
         JOIN captura_evento c ON c.instancia_id=e.inicio_instancia_id
         WHERE proyecto=?1 AND chat=?2 ORDER BY c.secuencia,e.id",params![proyecto,chat])?;
    for ep in &mut episodios {
        let id = ep["id"].as_i64().context("episodio automatico sin id")?;
        ep["pasos"] = json!(rows(c,
            "SELECT p.*,i.clave_evento FROM episodio_automatico_paso p JOIN instancia i ON i.id=p.instancia_id
             WHERE episodio_id=?1 ORDER BY orden",[id])?);
        ep["resultado"] = rows(
            c,
            "SELECT * FROM episodio_automatico_resultado WHERE episodio_id=?1",
            [id],
        )?
        .pop()
        .unwrap_or(Value::Null);
        ep["estado_resultado"] = json!(if ep["resultado"].is_null() {
            "sin_verificacion"
        } else {
            "comprobacion_observada"
        });
    }
    Ok(
        json!({"origen_objetivo":"automatico_lista","proyecto":proyecto,"chat":chat,"episodios":episodios,
        "alcance":"resultado tecnico de la ultima comprobacion reconocida; no acredita calidad global del objetivo",
        "costo":{"duracion_microsegundos":null,"reintentos":null,"tokens":null,
            "motivo":"los cortes no miden trabajo efectivo, correlacion de reintentos ni tokens"}}),
    )
}

impl Store {
    pub fn listar_episodios_automaticos(&mut self, proyecto: &str, chat: &str) -> Result<Value> {
        nonempty(proyecto)?;
        nonempty(chat)?;
        let tx = self.conn.transaction()?;
        let salida = listar(&tx, proyecto, chat)?;
        tx.commit()?;
        Ok(salida)
    }

    /// Lee el chat en orden de fuente. Sólo persiste tramos cerrados. Un cierre
    /// registrado en control se intercala después de su último evento observado.
    pub fn procesar_episodios_automaticos(
        &mut self,
        proyecto: &str,
        chat: &str,
        cierres: &[CierreChat],
    ) -> Result<Value> {
        nonempty(proyecto)?;
        nonempty(chat)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let crudos = rows(&tx,
            "SELECT c.instancia_id,c.json_original FROM captura_evento c JOIN instancia i ON i.id=c.instancia_id
             WHERE c.grupo_evidencia=?1 AND json_extract(i.metadatos_json,'$.proyecto')=?2 ORDER BY c.secuencia",params![chat,proyecto])?;
        let mut datos = Vec::new();
        let mut ultimos = BTreeMap::new();
        for (indice, r) in crudos.iter().enumerate() {
            let v: Value =
                serde_json::from_str(r["json_original"].as_str().context("crudo ausente")?)?;
            if let Some(intervalo) = v["metadatos"]["intervalo_captura"].as_i64() {
                ultimos.insert(intervalo, indice);
            }
            datos.push((r["instancia_id"].as_i64().context("instancia sin id")?, v));
        }
        let mut ordenados: Vec<_> = cierres.iter().collect();
        ordenados.sort_by_key(|c| c.intervalo_id);
        let mut limites = BTreeMap::new();
        for cierre in ordenados {
            // El intervalo final puede no tener eventos (reactivación seguida
            // de SessionEnd). Cierra lo observado hasta él, nunca lo posterior.
            if let Some(indice) = ultimos.range(..=cierre.intervalo_id).map(|(_, i)| *i).max() {
                limites.entry(indice).or_insert(cierre);
            }
        }
        let mut eventos = Vec::new();
        let mut referencias = Vec::new();
        let mut controles = Vec::new();
        for (indice, (id, v)) in datos.into_iter().enumerate() {
            eventos.push(EventoCorte {
                chat: chat.into(),
                payload: v,
                fin_sesion: false,
            });
            referencias.push(Some(id));
            controles.push(None);
            if let Some(cierre) = limites.get(&indice) {
                eventos.push(EventoCorte {
                    chat: chat.into(),
                    payload: Value::Null,
                    fin_sesion: true,
                });
                referencias.push(None);
                controles.push(Some(*cierre));
            }
        }
        let tramos = detectar_tramos(chat, &eventos);
        let mut abiertos = Vec::new();
        for tramo in tramos {
            let inicio = referencias[tramo.inicio].context("inicio sin captura")?;
            let Some(fin) = tramo.fin else {
                abiertos.push(json!({"objetivo":tramo.objetivo,"inicio_instancia_id":inicio,"herramientas_observadas":tramo.herramientas.len()}));
                continue;
            };
            let pasos: Vec<_> = tramo
                .herramientas
                .iter()
                .map(|i| normalizar_herramienta(&eventos[*i].payload))
                .collect();
            let hash = hash_secuencia(&pasos)?;
            let fin_instancia = referencias[fin];
            let cierre = controles[fin];
            let ultima = tramo
                .comprobacion
                .as_ref()
                .and_then(|c| referencias[c.evento]);
            if let Some(anterior) = rows(
                &tx,
                "SELECT * FROM episodio_automatico WHERE inicio_instancia_id=?1",
                [inicio],
            )?
            .pop()
            {
                ensure!(
                    anterior["proyecto"] == proyecto
                        && anterior["chat"] == chat
                        && anterior["objetivo"] == tramo.objetivo
                        && anterior["fin_instancia_id"] == json!(fin_instancia)
                        && anterior["cierre_intervalo_id"] == json!(cierre.map(|c| c.intervalo_id))
                        && anterior["cierre_control_en"] == json!(cierre.map(|c| c.cerrado_en))
                        && anterior["motivo_fin"] == json!(tramo.motivo_fin)
                        && anterior["secuencia_sha256"] == hash
                        && anterior["ultima_comprobacion_id"] == json!(ultima),
                    "datos tardios cambian un episodio automatico sellado; requiere revision"
                );
                continue;
            }
            tx.execute("INSERT INTO episodio_automatico(origen_objetivo,proyecto,chat,objetivo,fuente,inicio_instancia_id,fin_instancia_id,cierre_intervalo_id,cierre_control_en,motivo_fin,ultima_comprobacion_id,detector_version,normalizador_version,secuencia_sha256,cantidad_pasos,creado_en)
                VALUES ('automatico_lista',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",params![proyecto,chat,tramo.objetivo,tramo.fuente,inicio,fin_instancia,
                    cierre.map(|c|c.intervalo_id),cierre.map(|c|c.cerrado_en),tramo.motivo_fin,ultima,DETECTOR_VERSION,NORMALIZADOR_VERSION,hash,pasos.len() as i64,now()?])?;
            let id = tx.last_insert_rowid();
            for (indice, (evento, paso)) in tramo.herramientas.iter().zip(&pasos).enumerate() {
                tx.execute(
                    "INSERT INTO episodio_automatico_paso VALUES (?1,?2,?3,?4,?5)",
                    params![
                        id,
                        (indice + 1) as i64,
                        referencias[*evento],
                        paso.herramienta,
                        paso.tipo_destino
                    ],
                )?;
            }
            if let Some(c) = tramo.comprobacion {
                if let Some(resultado) = c.resultado {
                    tx.execute("INSERT INTO episodio_automatico_resultado VALUES (?1,?2,?3,'ultima-comprobacion-v1')",params![id,referencias[c.evento],i64::from(resultado)])?;
                }
            }
            tx.execute("INSERT INTO episodio_automatico_sello VALUES (?1)", [id])?;
        }
        let mut salida = listar(&tx, proyecto, chat)?;
        salida["abiertos"] = json!(abiertos);
        tx.commit()?;
        Ok(salida)
    }
}
