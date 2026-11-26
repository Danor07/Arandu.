//! Competencia entre caminos de un tipo de objetivo y alerta temprana.
//! Lógica pura, sin E/S. Reglas, simulación y límites en README, «Competencia».
use serde::Serialize;
use std::collections::BTreeMap;

pub const COMPETENCIA_VERSION: &str = "competencia-ventana-v1";
/// Verificaciones recientes de cada camino que cuentan para competir.
pub const VENTANA: usize = 20;
/// Ventaja (éxitos menos fallos en la ventana) que la retadora tiene que superar.
/// Lo eligió la simulación sintética de `tests/competencia.rs`; provisional hasta tener uso real.
pub const K_VENTAJA: i64 = 2;
/// Fallos seguidos con los que un camino se da por roto: no releva a nadie y, si es la
/// vigente, deja de ofrecerse al host. El relevo llega por la ventaja, como siempre.
pub const F_FALLOS: usize = 5;

/// Éxitos menos fallos.
pub fn neto(serie: &[bool]) -> i64 {
    serie.iter().map(|e| if *e { 1 } else { -1 }).sum()
}

/// Los últimos `f` resultados son todos fallos.
pub fn rota(serie: &[bool], f: usize) -> bool {
    serie.len() >= f && serie[serie.len() - f..].iter().all(|e| !e)
}

/// Estado de la competencia de un tipo. Se obtiene plegando las verificaciones
/// en orden; no guarda nada que no salga de ellas.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Competencia {
    /// Últimos `VENTANA` resultados de cada camino. Restar evidencia es observar un fallo más.
    pub ventana: BTreeMap<String, Vec<bool>>,
    pub vigente: Option<String>,
    /// Camino archivado → sucesor que lo desplazó.
    pub archivados: BTreeMap<String, String>,
}

impl Competencia {
    /// El primer camino con un éxito queda vigente. Otro lo releva si su neto supera
    /// al de la vigente por más de `k`, la vigente tiene algún fallo en su ventana y
    /// él tiene algún éxito y no está roto (sus últimos `f` no son todos fallos).
    /// Una vigente que no falla no se releva: dos caminos sanos no oscilan.
    pub fn observar(&mut self, camino: &str, exito: bool, k: i64, f: usize) {
        let serie = self.ventana.entry(camino.into()).or_default();
        serie.push(exito);
        if serie.len() > VENTANA {
            serie.remove(0);
        }
        let Some(vigente) = self.vigente.clone() else {
            if exito {
                self.vigente = Some(camino.into());
            }
            return;
        };
        let actual = &self.ventana[&vigente];
        if !actual.contains(&false) {
            return;
        }
        let base = neto(actual);
        // Entre varias retadoras gana la de más neto; a igual neto, el hash menor.
        let nuevo = self
            .ventana
            .iter()
            .filter(|(c, s)| {
                **c != vigente && s.contains(&true) && !rota(s, f) && neto(s) > base + k
            })
            .max_by(|a, b| neto(a.1).cmp(&neto(b.1)).then(b.0.cmp(a.0)))
            .map(|(c, _)| c.clone());
        if let Some(nuevo) = nuevo {
            self.archivados.remove(&nuevo);
            self.archivados.insert(vigente, nuevo.clone());
            self.vigente = Some(nuevo);
        }
    }
}

pub const ALERTA_VERSION: &str = "alerta-varianza-autocorrelacion-v2";
/// Ventana y umbrales provisionales, elegidos con las tasas medidas en
/// `tests/competencia.rs` sobre series sintéticas: no hay uso real con qué calibrarlos.
pub const ALERTA_VENTANA: usize = 10;
/// Fallos de más que la ventana reciente tiene que mostrar sobre la anterior.
pub const ALERTA_MAS_FALLOS: usize = 3;
/// Con ventana 10: pasar de 0 a 3 fallos sube la varianza 0.21; de 2 a 5, 0.09. De 3 a 6
/// sube 0.03 y no alerta: ese camino ya venía fallando y lo resuelve la competencia.
pub const ALERTA_SUBE_VARIANZA: f64 = 0.075;
pub const ALERTA_SUBE_AUTOCORRELACION: f64 = 0.1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Alerta {
    pub regla_version: &'static str,
    /// `sin_datos`, `ninguna`, `varianza`, `varianza_y_autocorrelacion` o `falla_sostenida`.
    pub nivel: &'static str,
    pub ventana: usize,
    pub varianza_anterior: f64,
    pub varianza_reciente: f64,
    pub autocorrelacion_anterior: f64,
    pub autocorrelacion_reciente: f64,
}

/// Varianza poblacional y autocorrelación de retardo 1. Serie constante: (0, 0).
pub fn momentos(serie: &[bool]) -> (f64, f64) {
    let x: Vec<f64> = serie.iter().map(|b| f64::from(u8::from(*b))).collect();
    let media = x.iter().sum::<f64>() / x.len().max(1) as f64;
    let suma: f64 = x.iter().map(|v| (v - media).powi(2)).sum();
    if suma == 0.0 {
        return (0.0, 0.0);
    }
    let cruzada: f64 = x.windows(2).map(|w| (w[0] - media) * (w[1] - media)).sum();
    (suma / x.len() as f64, cruzada / suma)
}

/// `falla_sostenida` si los últimos `F_FALLOS` resultados son fallos. Si no, compara
/// las últimas `ALERTA_VENTANA` verificaciones con las anteriores y avisa sólo cuando
/// el camino empeora: `ALERTA_MAS_FALLOS` fallos de más y más varianza. La
/// autocorrelación sólo refuerza. Mientras la caída siga dentro de la ventana
/// reciente, sigue avisando aunque los últimos resultados ya sean éxitos.
pub fn alerta(serie: &[bool]) -> Alerta {
    let mut a = Alerta {
        regla_version: ALERTA_VERSION,
        nivel: "sin_datos",
        ventana: ALERTA_VENTANA,
        varianza_anterior: 0.0,
        varianza_reciente: 0.0,
        autocorrelacion_anterior: 0.0,
        autocorrelacion_reciente: 0.0,
    };
    if serie.len() >= 2 * ALERTA_VENTANA {
        let (anterior, reciente) =
            serie[serie.len() - 2 * ALERTA_VENTANA..].split_at(ALERTA_VENTANA);
        (a.varianza_anterior, a.autocorrelacion_anterior) = momentos(anterior);
        (a.varianza_reciente, a.autocorrelacion_reciente) = momentos(reciente);
        let fallos = |s: &[bool]| s.iter().filter(|e| !**e).count();
        a.nivel = if fallos(reciente) < fallos(anterior) + ALERTA_MAS_FALLOS
            || a.varianza_reciente - a.varianza_anterior < ALERTA_SUBE_VARIANZA
        {
            "ninguna"
        } else if a.autocorrelacion_reciente - a.autocorrelacion_anterior
            < ALERTA_SUBE_AUTOCORRELACION
        {
            "varianza"
        } else {
            "varianza_y_autocorrelacion"
        };
    }
    if rota(serie, F_FALLOS) {
        a.nivel = "falla_sostenida";
    }
    a
}
