//! La urna observada: lo que se ve de cada camino como atractor. Es lectura; no
//! decide la vigente ni lo que se entrega. Definiciones y límites en README, «Urna».
use crate::competencia::{F_FALLOS, VENTANA};
use crate::execution::rows;
use crate::retrieval::Cutoff;
use crate::vigentes::{Camino, TipoObjetivo};
use anyhow::Result;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub const URNA_VERSION: &str = "urna-observada-v1";
/// Verificaciones del tipo que forman cada tramo al comparar cuotas. Provisional.
pub const TRAMO: usize = VENTANA;
/// La vigente está en desuso si su cuota reciente es a lo sumo esta fracción de la
/// anterior (uno de cada `DESUSO_DIVISOR`) sin que haya fallado. Provisional.
pub const DESUSO_DIVISOR: usize = 2;

/// Cuotas (reciente, anterior) de un camino: qué fracción de las últimas `tramo`
/// verificaciones del tipo fueron suyas, y de las `tramo` anteriores a esas. `posiciones`
/// son los índices de sus verificaciones entre las `total` del tipo. Sin tramo anterior
/// completo no hay con qué comparar: la anterior es `None`.
pub fn cuotas(posiciones: &[usize], total: usize, tramo: usize) -> (f64, Option<f64>) {
    let desde = total.saturating_sub(tramo);
    let en = |a: usize, b: usize| posiciones.iter().filter(|p| (a..b).contains(*p)).count();
    let reciente = en(desde, total) as f64 / (total - desde).max(1) as f64;
    let anterior =
        (total >= 2 * tramo && tramo > 0).then(|| en(desde - tramo, desde) as f64 / tramo as f64);
    (reciente, anterior)
}

/// Pasos de `propio` que `otro` no tiene: lo que habría que aprender o instalar para
/// pasar de `otro` a `propio`. Es la medida conjetural del costo de desacople.
pub fn no_compartidos(propio: &BTreeSet<String>, otro: &BTreeSet<String>) -> usize {
    propio.difference(otro).count()
}

/// Deja de recibir trayectorias sin haber fallado: lo que una alerta de fallos no ve.
pub fn en_desuso(reciente: f64, anterior: Option<f64>, fallos_recientes: usize) -> bool {
    anterior.is_some_and(|a| a > 0.0 && reciente * DESUSO_DIVISOR as f64 <= a)
        && fallos_recientes == 0
}

/// Pasos normalizados (herramienta y tipo de destino) de la mínima de una derivación.
pub(crate) fn pasos(c: &Connection, derivacion_id: i64) -> Result<Vec<Value>> {
    rows(c,"SELECT p.orden_original,p.instancia_id,ep.herramienta,ep.tipo_destino FROM secuencia_minima_paso p
        JOIN secuencia_minima d ON d.id=p.derivacion_id
        JOIN (SELECT 'manual_tarea' AS origen,episodio_id,orden,herramienta,tipo_destino FROM episodio_paso
            UNION ALL SELECT 'automatico_lista',episodio_id,orden,herramienta,tipo_destino FROM episodio_automatico_paso) ep
        ON ep.origen=d.origen AND ep.episodio_id=coalesce(d.episodio_manual_id,d.episodio_automatico_id) AND ep.orden=p.orden_original
        WHERE p.derivacion_id=?1 AND p.en_minima=1 ORDER BY p.orden_original",[derivacion_id])
}

/// Entregas de una vigente de este tipo y qué hizo después el chat que la recibió.
const SEGUIMIENTO: &str = "SELECT v.evento_id,d.hash_minima AS ofrecido,
    EXISTS(SELECT 1 FROM objetivo_episodio_fuente f JOIN episodio_tipo_objetivo t ON t.origen=f.origen AND t.episodio_id=f.episodio_id
        WHERE t.tipo_clave=v.tipo_clave AND f.hash_minima=d.hash_minima AND f.evidencia_id>pi.id AND (?2 IS NULL OR f.evidencia_id<?2)
        AND EXISTS(SELECT 1 FROM objetivo_episodio_captura c JOIN instancia i ON i.id=c.instancia_id
            WHERE c.origen=f.origen AND c.episodio_id=f.episodio_id AND i.grupo_evidencia=pi.grupo_evidencia)) AS siguio,
    EXISTS(SELECT 1 FROM objetivo_episodio_fuente f JOIN episodio_tipo_objetivo t ON t.origen=f.origen AND t.episodio_id=f.episodio_id
        WHERE t.tipo_clave=v.tipo_clave AND f.evidencia_id>pi.id AND (?2 IS NULL OR f.evidencia_id<?2)
        AND EXISTS(SELECT 1 FROM objetivo_episodio_captura c JOIN instancia i ON i.id=c.instancia_id
            WHERE c.origen=f.origen AND c.episodio_id=f.episodio_id AND i.grupo_evidencia=pi.grupo_evidencia)) AS hizo_algo
    FROM recuperacion_vigente v JOIN camino_derivacion d ON d.tipo_clave=v.tipo_clave AND d.derivacion_id=v.derivacion_id
    JOIN evento_recuperacion r ON r.id=v.evento_id JOIN instancia pi ON pi.id=r.problema_instancia_id
    WHERE v.tipo_clave=?1 AND (?2 IS NULL OR pi.id<?2) ORDER BY v.evento_id";

/// Un éxito es inducido si al chat que lo logró ya se le había ofrecido ese mismo
/// camino. Sólo los demás muestran convergencia independiente.
const INDUCIDOS: &str = "SELECT f.origen,f.episodio_id FROM objetivo_episodio_fuente f
    JOIN episodio_tipo_objetivo t ON t.origen=f.origen AND t.episodio_id=f.episodio_id
    WHERE t.tipo_clave=?1 AND f.hash_minima IS NOT NULL AND EXISTS(
        SELECT 1 FROM recuperacion_vigente v JOIN camino_derivacion d ON d.tipo_clave=v.tipo_clave AND d.derivacion_id=v.derivacion_id
        JOIN evento_recuperacion r ON r.id=v.evento_id JOIN instancia pi ON pi.id=r.problema_instancia_id
        WHERE v.tipo_clave=?1 AND d.hash_minima=f.hash_minima AND pi.id<f.evidencia_id
        AND EXISTS(SELECT 1 FROM objetivo_episodio_captura c JOIN instancia i ON i.id=c.instancia_id
            WHERE c.origen=f.origen AND c.episodio_id=f.episodio_id AND i.grupo_evidencia=pi.grupo_evidencia))";

/// El bloque `urna` de la consulta: por camino, cuota, profundidad y costo de desacople
/// respecto de la vigente; y para el tipo, cuánto se sigue lo que la memoria ofrece.
pub(crate) fn observada(
    c: &Connection,
    tipo: &TipoObjetivo,
    observadas: &[Value],
    caminos: &[Camino],
    corte: Option<&Cutoff>,
) -> Result<Value> {
    let inducidos: BTreeSet<(String, i64)> = rows(c, INDUCIDOS, [&tipo.clave])?
        .iter()
        .filter_map(|f| {
            Some((
                f["origen"].as_str()?.to_string(),
                f["episodio_id"].as_i64()?,
            ))
        })
        .collect();
    let conjunto = |derivacion: i64| -> Result<BTreeSet<String>> {
        Ok(pasos(c, derivacion)?
            .iter()
            .map(|p| {
                let texto = |campo: &str| p[campo].as_str().unwrap_or_default().to_string();
                format!("{}/{}", texto("herramienta"), texto("tipo_destino"))
            })
            .collect())
    };
    let vigente = caminos.iter().find(|x| x.estado == "vigente");
    let pasos_vigente = vigente.map(|v| conjunto(v.derivacion_id)).transpose()?;
    let total: usize = caminos.iter().map(|x| x.serie.len()).sum();
    let mut salida = Vec::new();
    let mut desuso = false;
    for camino in caminos {
        let inducidos_del_camino = observadas
            .iter()
            .filter(|f| {
                f["resultado"] == 1
                    && f["hash_minima"] == camino.hash_minima.as_str()
                    && f["origen"]
                        .as_str()
                        .zip(f["episodio_id"].as_i64())
                        .is_some_and(|(origen, id)| inducidos.contains(&(origen.to_string(), id)))
            })
            .count();
        let (reciente, anterior) = cuotas(&camino.posiciones, total, TRAMO);
        let propios = conjunto(camino.derivacion_id)?;
        if camino.estado == "vigente" {
            let cola = &camino.serie[camino.serie.len().saturating_sub(F_FALLOS)..];
            desuso = en_desuso(reciente, anterior, cola.iter().filter(|e| !**e).count());
        }
        salida.push(json!({"hash_minima":camino.hash_minima,"estado":camino.estado,
            "cuota":camino.serie.len() as f64 / total.max(1) as f64,
            "cuota_reciente":reciente,"cuota_anterior":anterior,
            // Profundidad: sólo éxitos de chats a los que no se les ofreció este camino.
            "profundidad":camino.exitos - inducidos_del_camino,
            "exitos_inducidos":inducidos_del_camino,
            "pasos":propios,
            "pasos_no_compartidos_con_vigente":pasos_vigente.as_ref().map(|v| no_compartidos(&propios, v))}));
    }
    let entregas = rows(
        c,
        SEGUIMIENTO,
        params![tipo.clave, corte.map(|x| x.problem_id)],
    )?;
    let seguidas = entregas.iter().filter(|e| e["siguio"] == 1).count();
    let con_episodio = entregas.iter().filter(|e| e["hizo_algo"] == 1).count();
    Ok(
        json!({"regla":URNA_VERSION,"tramo":TRAMO,"caminos":salida,"vigente_en_desuso":desuso,
        "seguimiento":{"entregas":entregas.len(),"seguidas":seguidas,
            "otro_camino":con_episodio - seguidas,"sin_episodio":entregas.len() - con_episodio,
            // Estimación de p(x) en el punto actual; sin entregas con episodio no hay dato.
            "p_seguir":(con_episodio > 0).then(|| seguidas as f64 / con_episodio as f64)},
        "alcance":"lectura: no decide la vigente; ofrecido = paquete preparado con esa vigente, no lectura acreditada por el host"}),
    )
}
