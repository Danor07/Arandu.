//! Competencia, archivado y alerta temprana. La simulación usa sólo escenarios
//! SINTÉTICOS: no hay uso real todavía (entrega 5 pendiente).
use msp::competencia::{
    alerta, momentos, neto, rota, Competencia, ALERTA_VENTANA, COMPETENCIA_VERSION, F_FALLOS,
    K_VENTAJA, VENTANA,
};
use proptest::prelude::*;
use serde_json::{json, Value};

// ---------- Simulación: horquilla imperfecta contra la regla simple ----------

const PASOS: usize = 200;
const APARECE: usize = 20;
/// Lazo cerrado: la vigente se usa casi siempre; la otra, esta fracción de veces.
/// Es un supuesto fijo: el seguimiento no crece con la cuota acumulada, así que en estos
/// mundos no hay rendimientos crecientes ni, por lo tanto, la bifurcación que la horquilla
/// describe. Se compara su umbral como regla de corte, nada más (ver README).
const EXPLORACION: f64 = 0.3;
const SEMILLAS: u64 = 200;
/// Un cambio de vigente (archivar, migrar vínculos, otra recomendación al host) se cobra
/// en éxitos perdidos. Es una convención, no una medición: por eso se prueban varios precios.
const COSTO_CAMBIO: f64 = 1.0;
const PRECIOS: [f64; 5] = [0.0, 0.5, 1.0, 2.0, 5.0];
// Escenarios 0 a 4: nueva claramente mejor, suerte inicial, ruido intermitente, empate
// y vieja se degrada. 5 a 7: variantes moderadas de los tres primeros.

/// Probabilidad real de éxito. 0 es la vieja; 1, la nueva. `usos` son los del camino.
fn p(escenario: usize, camino: usize, t: usize, usos: usize) -> f64 {
    match (escenario, camino) {
        (0, 0) => 0.6,
        (0, _) => 0.95,
        (1, 0) => 0.85,
        (1, _) => [0.3, 1.0][usize::from(usos < 4)],
        (2, 0) => [0.95, 0.5][usize::from((t / 10).is_multiple_of(3))],
        (2, _) => 0.7,
        (3, _) => 0.8,
        (4, 0) => [0.3, 0.9][usize::from(t < 100)],
        (4, _) => 0.8,
        // Variantes moderadas de los tres primeros: diferencias más chicas.
        (5, 0) => 0.75,
        (5, _) => 0.9,
        (6, 0) => 0.85,
        (6, _) => [0.6, 1.0][usize::from(usos < 4)],
        (7, 0) => [0.9, 0.7][usize::from((t / 10).is_multiple_of(3))],
        _ => 0.8,
    }
}

struct Azar(u64);
impl Azar {
    // splitmix64: determinista y sin dependencias.
    fn uniforme(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
}

const R: f64 = 1.0;
const DT: f64 = 0.1;
const EULER: usize = 10;
const H_MAX: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Regla {
    /// La de producción: `Competencia::observar`, el mismo código que usa `vigentes.rs`.
    Produccion { k: i64, f: usize },
    /// Racha simétrica: cualquiera de las dos gana con `n` éxitos seguidos.
    Racha { n: usize },
    /// La ventaja en ventana sin las dos guardas de producción: la otra gana si su neto
    /// de las últimas 20 verificaciones supera al de la vigente por más de `k`.
    Ventana { k: i64 },
    /// ẋ = h + r·x − x³ integrada con Euler, una unidad de tiempo por verificación.
    /// h = ganancia·(evidencia nueva − evidencia vieja). `resta_fallos: false` es la
    /// lectura literal «h son los éxitos verificados».
    Horquilla {
        ganancia: f64,
        fuga: f64,
        resta_fallos: bool,
    },
    /// La misma h sin integrar: cambia cuando cruza el pliegue h_c(r) = 2·(r/3)^(3/2).
    Pliegue { ganancia: f64, fuga: f64 },
}

enum Estado {
    Produccion(Competencia),
    Racha {
        rachas: [usize; 2],
        vigente: usize,
    },
    Ventana {
        serie: [Vec<bool>; 2],
        vigente: usize,
    },
    Horquilla {
        x: f64,
        e: [f64; 2],
    },
    Pliegue {
        e: [f64; 2],
        vigente: usize,
    },
}

const NOMBRES: [&str; 2] = ["vieja", "nueva"];

fn evidencia(e: &mut [f64; 2], camino: usize, exito: bool, fuga: f64, resta_fallos: bool) {
    e.iter_mut().for_each(|v| *v *= 1.0 - fuga);
    e[camino] += match (exito, resta_fallos) {
        (true, _) => 1.0,
        (false, true) => -1.0,
        (false, false) => 0.0,
    };
}

impl Estado {
    fn vigente(&self) -> usize {
        match self {
            Estado::Produccion(c) => usize::from(c.vigente.as_deref() == Some("nueva")),
            Estado::Racha { vigente, .. }
            | Estado::Ventana { vigente, .. }
            | Estado::Pliegue { vigente, .. } => *vigente,
            Estado::Horquilla { x, .. } => usize::from(*x > 0.0),
        }
    }
    fn observar(&mut self, regla: Regla, camino: usize, exito: bool) {
        match (self, regla) {
            (Estado::Produccion(c), Regla::Produccion { k, f }) => {
                c.observar(NOMBRES[camino], exito, k, f)
            }
            (Estado::Racha { rachas, vigente }, Regla::Racha { n }) => {
                rachas[camino] = if exito { rachas[camino] + 1 } else { 0 };
                if exito && rachas[camino] >= n {
                    *vigente = camino;
                }
            }
            (Estado::Ventana { serie, vigente }, Regla::Ventana { k }) => {
                serie[camino].push(exito);
                let neto = |s: &Vec<bool>| neto(&s[s.len().saturating_sub(VENTANA)..]);
                if neto(&serie[1 - *vigente]) > neto(&serie[*vigente]) + k {
                    *vigente = 1 - *vigente;
                }
            }
            (
                Estado::Horquilla { x, e },
                Regla::Horquilla {
                    ganancia,
                    fuga,
                    resta_fallos,
                },
            ) => {
                evidencia(e, camino, exito, fuga, resta_fallos);
                let h = (ganancia * (e[1] - e[0])).clamp(-H_MAX, H_MAX);
                for _ in 0..EULER {
                    *x += DT * (h + R * *x - *x * *x * *x);
                }
            }
            (Estado::Pliegue { e, vigente }, Regla::Pliegue { ganancia, fuga }) => {
                evidencia(e, camino, exito, fuga, true);
                let otra = 1 - *vigente;
                if e[otra] > 0.0 && ganancia * (e[otra] - e[*vigente]) > 2.0 * (R / 3.0).powf(1.5) {
                    *vigente = otra;
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Un mundo de la simulación. `escenarios` indexa `p`.
#[derive(Clone, Copy)]
struct Mundo {
    nombre: &'static str,
    exploracion: f64,
    pasos: usize,
    escenarios: &'static [usize],
}

const BASE: Mundo = Mundo {
    nombre: "base",
    exploracion: EXPLORACION,
    pasos: PASOS,
    escenarios: &[0, 1, 2, 3, 4],
};

/// Devuelve (arrepentimiento acumulado, cambios de vigente) de una corrida.
fn corrida(regla: Regla, escenario: usize, semilla: u64, mundo: Mundo) -> (f64, usize) {
    let mut azar = Azar(semilla.wrapping_mul(1_000_003) + escenario as u64);
    let mut estado = match regla {
        Regla::Produccion { .. } => Estado::Produccion(Competencia::default()),
        Regla::Racha { .. } => Estado::Racha {
            rachas: [0; 2],
            vigente: 0,
        },
        Regla::Ventana { .. } => Estado::Ventana {
            serie: [Vec::new(), Vec::new()],
            vigente: 0,
        },
        Regla::Horquilla { .. } => Estado::Horquilla {
            x: -R.sqrt(),
            e: [0.0; 2],
        },
        Regla::Pliegue { .. } => Estado::Pliegue {
            e: [0.0; 2],
            vigente: 0,
        },
    };
    let (mut usos, mut arrepentimiento, mut cambios) = ([0usize; 2], 0.0, 0);
    for t in 0..mundo.pasos {
        // Los dos sorteos se consumen siempre: todas las reglas ven el mismo azar.
        let (elige, sale) = (azar.uniforme(), azar.uniforme());
        let vigente = estado.vigente();
        let reales = [p(escenario, 0, t, usos[0]), p(escenario, 1, t, usos[1])];
        if t >= APARECE {
            arrepentimiento += reales[0].max(reales[1]) - reales[vigente];
        }
        let camino = if t >= APARECE && elige < mundo.exploracion {
            1 - vigente
        } else {
            vigente
        };
        usos[camino] += 1;
        estado.observar(regla, camino, sale < reales[camino]);
        cambios += usize::from(estado.vigente() != vigente);
    }
    (arrepentimiento, cambios)
}

/// (arrepentimiento, cambios) de cada semilla, sumados sobre los escenarios del mundo:
/// permite cobrar el cambio a cualquier precio y comparar reglas de a pares con el mismo azar.
struct Medida(Vec<(f64, f64)>);

impl Medida {
    fn media(&self, f: impl Fn(&(f64, f64)) -> f64) -> f64 {
        self.0.iter().map(f).sum::<f64>() / self.0.len() as f64
    }
    fn costo(&self, precio: f64) -> f64 {
        self.media(|x| x.0 + precio * x.1)
    }
    /// Media y error estándar de (costo propio − costo de la otra), semilla por semilla.
    fn menos(&self, otra: &Medida, precio: f64) -> (f64, f64) {
        let n = self.0.len() as f64;
        let d: Vec<f64> = self
            .0
            .iter()
            .zip(&otra.0)
            .map(|(a, b)| a.0 - b.0 + precio * (a.1 - b.1))
            .collect();
        let media = d.iter().sum::<f64>() / n;
        let varianza = d.iter().map(|x| (x - media).powi(2)).sum::<f64>() / (n - 1.0);
        (media, (varianza / n).sqrt())
    }
}

fn medir(regla: Regla, semillas: std::ops::Range<u64>, mundo: Mundo) -> Medida {
    Medida(
        semillas
            .map(|s| {
                mundo.escenarios.iter().fold((0.0, 0.0), |suma, e| {
                    let (a, c) = corrida(regla, *e, s, mundo);
                    (suma.0 + a, suma.1 + c as f64)
                })
            })
            .collect(),
    )
}

const FAMILIAS: [&str; 5] = ["produccion", "racha", "ventana", "horquilla", "pliegue"];
const PRODUCCION: Regla = Regla::Produccion {
    k: K_VENTAJA,
    f: F_FALLOS,
};

/// Grillas fijas. Horquilla y pliegue comparten ganancia y fuga: lo único que
/// cambia entre ellas es integrar o no la dinámica.
fn grillas() -> [Vec<Regla>; 5] {
    let mut produccion = Vec::new();
    for k in [0, 1, 2, 4, 6] {
        produccion.push(Regla::Produccion { k, f: F_FALLOS });
    }
    let mut horquillas = Vec::new();
    let mut pliegues = Vec::new();
    for ganancia in [0.05, 0.1, 0.2, 0.4, 0.8] {
        for fuga in [0.0, 0.05, 0.1, 0.2, 0.4] {
            pliegues.push(Regla::Pliegue { ganancia, fuga });
            for resta_fallos in [false, true] {
                horquillas.push(Regla::Horquilla {
                    ganancia,
                    fuga,
                    resta_fallos,
                });
            }
        }
    }
    [
        produccion,
        [1, 2, 3, 5, 8].map(|n| Regla::Racha { n }).to_vec(),
        [0, 1, 2, 4, 6].map(|k| Regla::Ventana { k }).to_vec(),
        horquillas,
        pliegues,
    ]
}

/// En un mundo y a un precio: cada familia elige su configuración de menor costo con las
/// semillas de selección y se compara, con semillas que no miró, contra la que eligió la
/// familia de producción en ese mismo mundo. Diferencia positiva: la regla simple decide mejor.
/// `constantes_fijas` es lo que cuesta dejar las constantes de producción sin reajustar.
fn comparar(mundo: Mundo, semillas: u64, precios: &[f64]) -> Value {
    let medidas = |rango: std::ops::Range<u64>| -> Vec<Vec<Medida>> {
        grillas()
            .iter()
            .map(|g| g.iter().map(|r| medir(*r, rango.clone(), mundo)).collect())
            .collect()
    };
    let (seleccion, validacion) = (medidas(0..semillas), medidas(3 * semillas..4 * semillas));
    let fijas = medir(PRODUCCION, 3 * semillas..4 * semillas, mundo);
    precios
        .iter()
        .map(|precio| {
            let mejor: Vec<usize> = seleccion
                .iter()
                .map(|familia| {
                    (0..familia.len())
                        .min_by(|a, b| familia[*a].costo(*precio).total_cmp(&familia[*b].costo(*precio)))
                        .unwrap()
                })
                .collect();
            let simple = &validacion[0][mejor[0]];
            let (media, error) = fijas.menos(simple, *precio);
            let mut fila = json!({"mundo":mundo.nombre,"exploracion":mundo.exploracion,"pasos":mundo.pasos,
                "precio_del_cambio":precio,
                "constantes_fijas":{"costo":fijas.costo(*precio),"menos_elegida":media,"error_estandar":error}});
            for (i, familia) in FAMILIAS.iter().enumerate() {
                let (media, error) = validacion[i][mejor[i]].menos(simple, *precio);
                fila[familia] = json!({"regla":format!("{:?}", grillas()[i][mejor[i]]),
                    "costo":validacion[i][mejor[i]].costo(*precio),
                    "menos_produccion":media,"error_estandar":error,
                    // Decide mejor que la regla simple por más de dos errores estándar.
                    "gana":media < -2.0 * error});
            }
            fila
        })
        .collect()
}

/// `cargo test --test competencia simulacion -- --nocapture` imprime la comparación.
/// La hoja de ruta: «si la dinámica no decide mejor, se queda la regla simple».
#[test]
fn simulacion_horquilla_contra_regla_simple_es_reproducible_y_respalda_produccion() {
    let s = comparar(BASE, SEMILLAS, &[COSTO_CAMBIO]);
    println!("{}", serde_json::to_string_pretty(&s).unwrap());
    assert_eq!(s, comparar(BASE, SEMILLAS, &[COSTO_CAMBIO]));
    assert_eq!(COMPETENCIA_VERSION, "competencia-ventana-v1");
    let fila = &s[0];
    // Si cambian escenarios, grillas o costo y la elección se mueve, producción tiene que moverse con ella.
    assert_eq!(fila["produccion"]["regla"], format!("{PRODUCCION:?}"));
    // Ni la ecuación integrada ni su umbral estático deciden mejor que la regla simple.
    assert_eq!(fila["horquilla"]["gana"], false);
    assert_eq!(fila["pliegue"]["gana"], false);
}

/// Los mundos del ataque: otros precios del cambio, otra exploración, otro horizonte y
/// escenarios moderados. Tarda minutos; se corre a mano:
/// `cargo test --release --test competencia mundos -- --ignored --nocapture`.
#[test]
#[ignore = "tarda minutos; ver README"]
fn simulacion_en_los_mundos_del_ataque() {
    let mundo = |nombre, exploracion, pasos| Mundo {
        nombre,
        exploracion,
        pasos,
        ..BASE
    };
    let mut filas = comparar(BASE, SEMILLAS, &PRECIOS)
        .as_array()
        .unwrap()
        .clone();
    for m in [
        mundo("exploracion_0.05", 0.05, PASOS),
        mundo("exploracion_0.1", 0.1, PASOS),
        mundo("exploracion_0.5", 0.5, PASOS),
        mundo("60_pasos", EXPLORACION, 60),
        mundo("1000_pasos", EXPLORACION, 1000),
        Mundo {
            nombre: "escenarios_moderados",
            escenarios: &[5, 6, 7, 3, 4],
            ..BASE
        },
        Mundo {
            nombre: "solo_mejor_empate_degrada",
            escenarios: &[0, 3, 4],
            ..BASE
        },
    ] {
        filas.extend(
            comparar(m, SEMILLAS, &[COSTO_CAMBIO])
                .as_array()
                .unwrap()
                .clone(),
        );
    }
    for f in &filas {
        let celda = |familia: &str, campo: &str| {
            format!(
                "{:+.2}±{:.2}{}",
                f[familia][campo].as_f64().unwrap(),
                f[familia]["error_estandar"].as_f64().unwrap(),
                if f[familia]["gana"] == true {
                    " GANA"
                } else {
                    ""
                }
            )
        };
        println!(
            "{} precio={} | simple elige {} costo {:.2} | fijas {} | racha {} | ventana {} | horquilla {} | pliegue {}",
            f["mundo"],
            f["precio_del_cambio"],
            f["produccion"]["regla"],
            f["produccion"]["costo"].as_f64().unwrap(),
            celda("constantes_fijas", "menos_elegida"),
            celda("racha", "menos_produccion"),
            celda("ventana", "menos_produccion"),
            celda("horquilla", "menos_produccion"),
            celda("pliegue", "menos_produccion")
        );
    }
}

// ---------- Regla de producción ----------

fn plegar(eventos: &[(u8, bool)]) -> Competencia {
    let mut c = Competencia::default();
    for (camino, exito) in eventos {
        c.observar(&format!("c{camino}"), *exito, K_VENTAJA, F_FALLOS);
    }
    c
}

fn repetir(camino: u8, exito: bool, veces: usize) -> Vec<(u8, bool)> {
    vec![(camino, exito); veces]
}

#[test]
fn neto_y_rota_miran_la_serie_que_reciben() {
    assert_eq!(neto(&[]), 0);
    assert_eq!(neto(&[true, true, false]), 1);
    assert!(!rota(&[false], 2) && rota(&[true, false, false], 2));
    assert!(!rota(&[false, true, false], 2) && !rota(&[false, false, true], 2));
}

#[test]
fn la_primera_con_exito_queda_y_la_retadora_gana_por_mas_de_k_si_la_vigente_fallo() {
    assert_eq!(plegar(&[(0, false), (1, false)]).vigente, None);
    assert_eq!(
        plegar(&[(0, false), (0, true)]).vigente.as_deref(),
        Some("c0")
    );
    // Vigente con neto 0 (un fallo y un éxito): la retadora necesita más de K de ventaja.
    let mut eventos = [
        vec![(0, false), (0, true)],
        repetir(1, true, K_VENTAJA as usize),
    ]
    .concat();
    let antes = plegar(&eventos);
    assert_eq!(antes.vigente.as_deref(), Some("c0"));
    assert!(antes.archivados.is_empty());
    eventos.push((1, true));
    let despues = plegar(&eventos);
    assert_eq!(despues.vigente.as_deref(), Some("c1"));
    assert_eq!(despues.archivados.get("c0").map(String::as_str), Some("c1"));
    // La archivada vuelve por la misma regla, y la otra pasa al archivo.
    eventos.push((1, false));
    eventos.extend(repetir(0, true, 2 * K_VENTAJA as usize + 2));
    let vuelta = plegar(&eventos);
    assert_eq!(vuelta.vigente.as_deref(), Some("c0"));
    assert_eq!(
        vuelta.archivados,
        [("c1".to_string(), "c0".to_string())].into()
    );
}

#[test]
fn solo_cuentan_las_ultimas_verificaciones_de_cada_camino() {
    let c = plegar(&[repetir(0, false, 5), repetir(0, true, VENTANA + 3)].concat());
    assert_eq!(c.ventana["c0"], vec![true; VENTANA]);
    // Treinta éxitos viejos no valen más que la ventana: con un fallo la retadora no
    // llega a superarla por más de K; con dos, sí.
    let con_fallos = |fallos: usize| {
        let mut eventos = repetir(0, true, 30);
        for _ in 0..fallos {
            eventos.extend([(0, false), (0, true)]);
        }
        eventos.extend(repetir(1, true, VENTANA));
        plegar(&eventos)
    };
    let c = con_fallos(1);
    assert_eq!(neto(&c.ventana["c0"]), VENTANA as i64 - 2);
    assert_eq!(c.vigente.as_deref(), Some("c0"));
    assert_eq!(con_fallos(2).vigente.as_deref(), Some("c1"));
}

#[test]
fn regresion_dos_caminos_sin_fallos_no_oscilan() {
    // Ataque A3: éxitos A A B B A A B B… cambiaban la vigente 73 veces en 200.
    for patron in [2u32, 3, 7] {
        let mut c = Competencia::default();
        for t in 0..1000u32 {
            c.observar(
                ["a", "b"][(t / patron % 2) as usize],
                true,
                K_VENTAJA,
                F_FALLOS,
            );
            assert_eq!(c.vigente.as_deref(), Some("a"));
        }
        assert!(c.archivados.is_empty());
    }
}

#[test]
fn regresion_una_vigente_que_se_rompe_se_marca_rota_y_cede_por_ventaja() {
    // Ataque A4: 60 éxitos y después sólo fallos; la alternativa tenía un éxito viejo.
    // A los F fallos seguidos está rota (no se ofrece); el relevo llega cuando el neto
    // de su ventana cae más de K por debajo del de la alternativa.
    let mut eventos = [repetir(0, true, 60), vec![(1, true)]].concat();
    let mut fallos = 0;
    while plegar(&eventos).vigente.as_deref() == Some("c0") {
        // Acotado: una regla rota tiene que hacer fallar la prueba, no colgarla.
        assert!(fallos < 2 * VENTANA, "la vigente rota nunca cede");
        eventos.push((0, false));
        fallos += 1;
        assert_eq!(
            rota(&plegar(&eventos).ventana["c0"], F_FALLOS),
            fallos >= F_FALLOS
        );
    }
    // neto de la vigente: VENTANA − 2·fallos; el de la otra: 1.
    assert_eq!(fallos, (VENTANA + K_VENTAJA as usize).div_ceil(2));
    let c = plegar(&eventos);
    assert_eq!(c.vigente.as_deref(), Some("c1"));
    assert_eq!(c.archivados.get("c0").map(String::as_str), Some("c1"));
    // Ataque M7: 1 éxito y 30 fallos contra 1 éxito y 1 fallo; releva por ventaja.
    let c = plegar(
        &[
            vec![(0, true), (1, true), (1, false)],
            repetir(0, false, 30),
        ]
        .concat(),
    );
    assert_eq!(c.vigente.as_deref(), Some("c1"));
    // Ataque M1: sin alternativa sigue vigente, y `rota` avisa para no ofrecerla.
    let sola = plegar(&[vec![(0, true)], repetir(0, false, 12)].concat());
    assert_eq!(sola.vigente.as_deref(), Some("c0"));
    assert!(rota(&sola.ventana["c0"], F_FALLOS) && sola.archivados.is_empty());
}

#[test]
fn una_retadora_rota_o_sin_exitos_no_releva_y_a_igual_neto_gana_el_nombre_menor() {
    // c1 tiene ventaja de sobra, pero sus últimos F resultados son fallos: no releva.
    let rota_retadora = [
        vec![(0, true)],
        repetir(1, true, 12),
        repetir(1, false, F_FALLOS),
        repetir(0, false, 3),
    ]
    .concat();
    assert_eq!(plegar(&rota_retadora).vigente.as_deref(), Some("c0"));
    // Con un fallo menos en la cola no está rota y releva.
    let sana = [
        vec![(0, true)],
        repetir(1, true, 12),
        repetir(1, false, F_FALLOS - 1),
        repetir(0, false, 3),
    ]
    .concat();
    assert_eq!(plegar(&sana).vigente.as_deref(), Some("c1"));
    // Una retadora sin ningún éxito no releva aunque su neto sea mayor.
    let sin_exito = plegar(&[vec![(0, true)], repetir(0, false, 8), vec![(1, false)]].concat());
    assert_eq!(sin_exito.vigente.as_deref(), Some("c0"));
    // Dos retadoras con el mismo neto cuando la vigente falla: el nombre menor.
    let empate = [vec![(0, true)], repetir(2, true, 3), repetir(1, true, 3)].concat();
    assert_eq!(plegar(&empate).vigente.as_deref(), Some("c0"));
    assert_eq!(
        plegar(&[empate.clone(), vec![(0, false)]].concat())
            .vigente
            .as_deref(),
        Some("c1")
    );
    // Con más neto gana la otra, sea cual sea su nombre.
    let mayor = plegar(&[empate, vec![(2, true), (0, false)]].concat());
    assert_eq!(mayor.vigente.as_deref(), Some("c2"));
    assert_eq!(
        mayor.archivados,
        [("c0".to_string(), "c2".to_string())].into()
    );
}

proptest! {
    #[test]
    fn la_competencia_conserva_sus_invariantes(eventos in prop::collection::vec((0u8..4, any::<bool>()), 0..120)) {
        let c = plegar(&eventos);
        prop_assert_eq!(&c, &plegar(&eventos));
        prop_assert_eq!(c.vigente.is_some(), eventos.iter().any(|e| e.1));
        if let Some(v) = &c.vigente {
            prop_assert!(!c.archivados.contains_key(v));
            let tuvo_exito = eventos.iter().any(|e| e.1 && &format!("c{}", e.0) == v);
            prop_assert!(tuvo_exito);
        }
        for (archivado, sucesor) in &c.archivados {
            prop_assert_ne!(archivado, sucesor);
            prop_assert!(c.ventana.contains_key(archivado) && c.ventana.contains_key(sucesor));
        }
        // La ventana de cada camino es la cola de sus propios resultados.
        for (camino, ventana) in &c.ventana {
            let propios: Vec<bool> = eventos.iter().filter(|e| &format!("c{}", e.0) == camino).map(|e| e.1).collect();
            prop_assert_eq!(ventana.as_slice(), &propios[propios.len().saturating_sub(VENTANA)..]);
        }
        // Un solo camino nunca se archiva a sí mismo.
        let solo: Vec<_> = eventos.iter().map(|e| (0u8, e.1)).collect();
        prop_assert!(plegar(&solo).archivados.is_empty());
    }

    #[test]
    fn sin_fallos_la_primera_vigente_no_se_releva(caminos in prop::collection::vec(0u8..5, 1..300)) {
        let eventos: Vec<_> = caminos.iter().map(|c| (*c, true)).collect();
        let c = plegar(&eventos);
        prop_assert_eq!(c.vigente, Some(format!("c{}", caminos[0])));
        prop_assert!(c.archivados.is_empty());
    }

    #[test]
    fn la_vigente_solo_cambia_por_un_fallo_suyo_o_por_un_exito_de_quien_la_releva(
        eventos in prop::collection::vec((0u8..4, any::<bool>()), 0..120),
    ) {
        let mut c = Competencia::default();
        for (camino, exito) in eventos {
            let (antes, nombre) = (c.clone(), format!("c{camino}"));
            c.observar(&nombre, exito, K_VENTAJA, F_FALLOS);
            if c.vigente == antes.vigente {
                prop_assert_eq!(&c.archivados, &antes.archivados);
                continue;
            }
            let nueva = c.vigente.clone().unwrap();
            match antes.vigente {
                // La primera queda con un éxito aunque arrastre fallos: no hay otra que ofrecer.
                None => prop_assert!(exito && nueva == nombre),
                Some(vieja) => {
                    prop_assert!((exito && nueva == nombre) || (!exito && vieja == nombre));
                    // La que cae tenía algún fallo; la que sube, ventaja, algún éxito y no está rota.
                    prop_assert!(c.ventana[&vieja].contains(&false));
                    prop_assert!(neto(&c.ventana[&nueva]) - neto(&c.ventana[&vieja]) > K_VENTAJA);
                    prop_assert!(c.ventana[&nueva].contains(&true) && !rota(&c.ventana[&nueva], F_FALLOS));
                    prop_assert_eq!(c.archivados.get(&vieja), Some(&nueva));
                }
            }
        }
    }

    #[test]
    fn la_alerta_solo_avisa_cuando_el_camino_empeora(
        prefijo in prop::collection::vec(any::<bool>(), 0..30),
        cola in prop::collection::vec(any::<bool>(), 2 * ALERTA_VENTANA),
    ) {
        let (varianza, autocorrelacion) = momentos(&cola);
        prop_assert!((0.0..=0.25).contains(&varianza));
        prop_assert!((-1.0..=1.0).contains(&autocorrelacion));
        let negada: Vec<bool> = cola.iter().map(|b| !b).collect();
        let espejo = momentos(&negada);
        prop_assert!((espejo.0 - varianza).abs() < 1e-12 && (espejo.1 - autocorrelacion).abs() < 1e-12);
        // Sólo cuentan las dos últimas ventanas.
        let completa = [prefijo.clone(), cola.clone()].concat();
        let a = alerta(&completa);
        prop_assert_eq!(&a, &alerta(&cola));
        let fallos = |s: &[bool]| s.iter().filter(|e| !**e).count();
        prop_assert_eq!(a.nivel == "falla_sostenida", rota(&cola, F_FALLOS));
        if a.nivel.starts_with("varianza") {
            prop_assert!(fallos(&cola[ALERTA_VENTANA..]) > fallos(&cola[..ALERTA_VENTANA]));
            prop_assert!(a.varianza_reciente > a.varianza_anterior);
        }
        // Con pocos datos no hay ventanas que comparar, pero una falla sostenida igual se ve.
        let corta = &prefijo[..prefijo.len().min(2 * ALERTA_VENTANA - 1)];
        prop_assert!(["sin_datos", "falla_sostenida"].contains(&alerta(corta).nivel));
        // Un camino que sólo acierta nunca alerta, tenga la historia que tenga detrás.
        let sana = [prefijo, vec![true; 2 * ALERTA_VENTANA]].concat();
        prop_assert_eq!(alerta(&sana).nivel, "ninguna");
    }
}

fn serie(texto: &str) -> Vec<bool> {
    texto.chars().map(|c| c == '1').collect()
}

#[test]
fn la_alerta_exige_mas_fallos_y_mas_varianza_y_la_autocorrelacion_solo_refuerza() {
    assert_eq!(momentos(&[]), (0.0, 0.0));
    assert_eq!(momentos(&[true; 5]), (0.0, 0.0));
    let (varianza, autocorrelacion) = momentos(&[false, false, true, true, true]);
    assert!((varianza - 0.24).abs() < 1e-12 && (autocorrelacion - 0.44 / 1.2).abs() < 1e-12);
    assert_eq!(alerta(&[true; 19]).nivel, "sin_datos");
    assert_eq!(alerta(&[true; 20]).nivel, "ninguna");
    // Un solo fallo en diez no alcanza el umbral provisional; dos, sí.
    assert_eq!(alerta(&serie("11111111111111011111")).nivel, "ninguna");
    let alternada = alerta(&serie("11111111111110101011"));
    assert_eq!(alternada.nivel, "varianza");
    assert!(alternada.autocorrelacion_reciente < 0.0);
    let agrupada = alerta(&serie("11111111111110001111"));
    assert_eq!(agrupada.nivel, "varianza_y_autocorrelacion");
    assert_eq!(
        (agrupada.varianza_anterior, agrupada.ventana),
        (0.0, ALERTA_VENTANA)
    );
    assert!((agrupada.varianza_reciente - 0.21).abs() < 1e-12);
    // La autocorrelación se compara con la de la ventana anterior, no sola: estos dos
    // casos dan el nivel contrario si la diferencia se cambia por una suma o un cociente.
    assert_eq!(alerta(&serie("00111111110000010111")).nivel, "varianza");
    assert_eq!(
        alerta(&serie("01011111110000101110")).nivel,
        "varianza_y_autocorrelacion"
    );
    // Justo en el umbral (la autocorrelación sube exactamente 0,1) ya refuerza.
    let borde = alerta(&serie("11111111110001011110"));
    assert_eq!(
        borde.autocorrelacion_reciente - borde.autocorrelacion_anterior,
        0.1
    );
    assert_eq!(borde.nivel, "varianza_y_autocorrelacion");
    // Si ya venía fallando igual, no hay suba: no es alerta temprana.
    assert_eq!(alerta(&serie("11001111111100111111")).nivel, "ninguna");
}

#[test]
fn regresion_la_alerta_ve_el_fallo_total_la_degradacion_lenta_y_no_la_recuperacion() {
    // Ataque M2: con todos los últimos en fallo la varianza es cero y decía «ninguna».
    for fallos in [F_FALLOS, 5, 7, 10, 20] {
        let s = [vec![true; 20], vec![false; fallos]].concat();
        assert_eq!(
            alerta(&s).nivel,
            "falla_sostenida",
            "{fallos} fallos seguidos"
        );
    }
    assert_eq!(
        alerta(&[vec![true], vec![false; F_FALLOS]].concat()).nivel,
        "falla_sostenida"
    );
    assert_eq!(
        alerta(&[vec![true], vec![false; F_FALLOS - 1]].concat()).nivel,
        "sin_datos"
    );
    // Ataque M3: una degradación lenta tiene que avisar antes del fallo total.
    let lenta = serie("111111111111011110111011001001000000000000");
    let primera = (1..=lenta.len())
        .find(|n| {
            alerta(&lenta[..*n]).nivel != "ninguna" && alerta(&lenta[..*n]).nivel != "sin_datos"
        })
        .unwrap();
    assert!(
        lenta[..primera].iter().filter(|e| **e).count()
            > lenta[primera..].iter().filter(|e| **e).count()
    );
    assert!(
        lenta[primera..].contains(&true),
        "avisó recién en el fallo total"
    );
    // Ataque M4: recuperarse no es una alerta. Mientras la caída sigue en la ventana
    // reciente todavía avisa; cuando la ventana reciente es la recuperación, calla.
    assert_ne!(alerta(&serie("11111111110000000111")).nivel, "ninguna");
    assert_eq!(alerta(&serie("00000001111111111111")).nivel, "ninguna");
}

/// Características de operación de la alerta sobre series sintéticas: fracción de
/// consultas que alertan en un camino estacionario, y demora hasta la primera alerta
/// cuando un camino sano (p = 0.95) pasa a fallar de forma intermitente (p = 0.6).
fn operacion_de_la_alerta() -> Value {
    let corridas = 2000u64;
    let avisa = |s: &[bool]| !["sin_datos", "ninguna"].contains(&alerta(s).nivel);
    let mut salida = json!({"datos":"sinteticos","corridas":corridas,"verificaciones":60});
    for prob in [0.9, 0.95] {
        let (mut consultas, mut alertas, mut alguna) = (0u64, 0u64, 0u64);
        for semilla in 0..corridas {
            let mut azar = Azar(semilla);
            let s: Vec<bool> = (0..60).map(|_| azar.uniforme() < prob).collect();
            let veces = (20..=60).filter(|n| avisa(&s[..*n])).count() as u64;
            consultas += 41;
            alertas += veces;
            alguna += u64::from(veces > 0);
        }
        salida[format!("estacionario_p_{prob}")] = json!({
            "consultas_con_alerta":alertas as f64 / consultas as f64,
            "corridas_con_alguna_alerta":alguna as f64 / corridas as f64});
    }
    let (mut detectadas, mut demora) = (0u64, 0u64);
    for semilla in 0..corridas {
        let mut azar = Azar(semilla);
        let s: Vec<bool> = (0..60)
            .map(|t| azar.uniforme() < if t < 30 { 0.95 } else { 0.6 })
            .collect();
        if let Some(n) = (31..=60).find(|n| avisa(&s[..*n])) {
            detectadas += 1;
            demora += n as u64 - 30;
        }
    }
    salida["degradacion_intermitente_0.95_a_0.6"] = json!({
        "detectadas_en_30":detectadas as f64 / corridas as f64,
        "demora_media":demora as f64 / detectadas.max(1) as f64});
    salida
}

/// `cargo test --test competencia operacion -- --nocapture` imprime las tasas.
#[test]
fn operacion_de_la_alerta_es_reproducible_y_respeta_las_cotas_declaradas() {
    let o = operacion_de_la_alerta();
    println!("{}", serde_json::to_string_pretty(&o).unwrap());
    assert_eq!(o, operacion_de_la_alerta());
    let n = |v: &Value| v.as_f64().unwrap();
    // Ataque M5: saltaba en el 72 % de las corridas de un camino sano con p = 0.9.
    assert!(n(&o["estacionario_p_0.95"]["consultas_con_alerta"]) < 0.01);
    assert!(n(&o["estacionario_p_0.9"]["consultas_con_alerta"]) < 0.04);
    assert!(n(&o["degradacion_intermitente_0.95_a_0.6"]["detectadas_en_30"]) > 0.9);
}
