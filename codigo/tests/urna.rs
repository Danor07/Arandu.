//! La urna: funciones puras de la lectura observada y dos simulaciones SINTÉTICAS.
//! `cargo test --test urna -- --nocapture` imprime los números.
use msp::urna::{cuotas, en_desuso, no_compartidos, DESUSO_DIVISOR};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;

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

fn logistica(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

// ---------- Lectura observada: funciones puras ----------

#[test]
fn las_cuotas_comparan_el_ultimo_tramo_con_el_anterior() {
    // Diez verificaciones del tipo; el camino hizo las cuatro primeras y la última.
    let propias = [0, 1, 2, 3, 9];
    assert_eq!(cuotas(&propias, 10, 5), (0.2, Some(0.8)));
    // Sin tramo anterior completo no hay comparación.
    assert_eq!(cuotas(&[0, 1, 2, 3, 8], 9, 5), (0.2, None));
    assert_eq!(cuotas(&[0, 1], 2, 5), (1.0, None));
    assert_eq!(cuotas(&[], 0, 5), (0.0, None));
    assert_eq!(cuotas(&[0], 3, 0), (0.0, None));
    // Lo anterior al tramo anterior no cuenta.
    assert_eq!(cuotas(&[0, 1, 2], 13, 5), (0.0, Some(0.0)));
}

#[test]
fn desuso_es_perder_trafico_sin_fallar_y_el_costo_de_desacople_cuenta_pasos_propios() {
    assert!(en_desuso(0.0, Some(1.0), 0));
    assert!(en_desuso(0.5, Some(1.0), 0));
    assert!(!en_desuso(0.51, Some(1.0), 0));
    // Si falló, no es desuso: eso lo ve la alerta de fallos.
    assert!(!en_desuso(0.0, Some(1.0), 1));
    // Sin tramo anterior, o sin tráfico anterior, no hay caída que medir.
    assert!(!en_desuso(0.0, None, 0) && !en_desuso(0.0, Some(0.0), 0));
    assert_eq!(DESUSO_DIVISOR, 2);
    let conjunto =
        |pasos: &[&str]| -> BTreeSet<String> { pasos.iter().map(|p| p.to_string()).collect() };
    let (viejo, nuevo) = (
        conjunto(&["Read/archivo", "Bash/comando"]),
        conjunto(&["Write/archivo", "Bash/comando", "Edit/archivo"]),
    );
    assert_eq!(no_compartidos(&nuevo, &viejo), 2);
    assert_eq!(no_compartidos(&viejo, &nuevo), 1);
    assert_eq!(no_compartidos(&viejo, &viejo), 0);
    assert_eq!(no_compartidos(&conjunto(&[]), &viejo), 0);
}

proptest! {
    #[test]
    fn las_cuotas_de_todos_los_caminos_suman_uno_en_cada_tramo(
        duenos in prop::collection::vec(0u8..4, 0..120), tramo in 1usize..30,
    ) {
        let total = duenos.len();
        let (mut reciente, mut anterior) = (0.0, 0.0);
        for camino in 0u8..4 {
            let propias: Vec<usize> = (0..total).filter(|i| duenos[*i] == camino).collect();
            let (r, a) = cuotas(&propias, total, tramo);
            prop_assert!((0.0..=1.0).contains(&r));
            prop_assert_eq!(a.is_some(), total >= 2 * tramo);
            reciente += r;
            anterior += a.unwrap_or(0.0);
            // Desuso exige haber tenido tráfico antes, haberlo perdido y no haber fallado.
            prop_assert!(!en_desuso(r, a, 0) || a.is_some_and(|a| a > 0.0 && r < a));
            prop_assert!(!en_desuso(r, a, 1));
        }
        prop_assert!(total == 0 || (reciente - 1.0).abs() < 1e-9);
        prop_assert!(total < 2 * tramo || (anterior - 1.0).abs() < 1e-9);
    }

    #[test]
    fn el_costo_de_desacople_es_cero_solo_si_no_hay_pasos_nuevos(
        a in prop::collection::btree_set("[a-d]", 0..5), b in prop::collection::btree_set("[a-d]", 0..5),
    ) {
        prop_assert_eq!(no_compartidos(&a, &b) == 0, a.is_subset(&b));
        prop_assert!(no_compartidos(&a, &b) <= a.len());
        prop_assert_eq!(no_compartidos(&a, &b) + a.intersection(&b).count(), a.len());
    }
}

// ---------- Simulación 1: la urna pura y dónde bifurca ----------

const EXTRACCIONES: usize = 2000;
const CORRIDAS: u64 = 300;

/// Urna de Pólya no lineal: el próximo uso es del camino A con probabilidad
/// p(x) = logística(γ·(2x − 1) + b), con x la cuota de usos de A. Devuelve la cuota final.
fn urna(gamma: f64, sesgo: f64, semilla: u64) -> f64 {
    let mut azar = Azar(semilla);
    let (mut a, mut total) = (5.0, 10.0);
    for _ in 0..EXTRACCIONES {
        if azar.uniforme() < logistica(gamma * (2.0 * a / total - 1.0) + sesgo) {
            a += 1.0;
        }
        total += 1.0;
    }
    a / total
}

/// Puntos fijos estables de ẋ = p(x) − x: donde p cruza la diagonal desde arriba.
fn atractores(gamma: f64, sesgo: f64) -> Vec<f64> {
    let g = |x: f64| logistica(gamma * (2.0 * x - 1.0) + sesgo) - x;
    (0..10_000)
        .map(|i| (i as f64 / 10_000.0, (i + 1) as f64 / 10_000.0))
        .filter(|(a, b)| g(*a) > 0.0 && g(*b) <= 0.0)
        .map(|(a, b)| (a + b) / 2.0)
        .collect()
}

/// Sesgo crítico exacto de la urna: el menor b con un solo atractor.
fn sesgo_critico(gamma: f64) -> f64 {
    (0..2000)
        .map(|i| i as f64 / 1000.0)
        .find(|b| atractores(gamma, *b).len() == 1)
        .unwrap()
}

fn urna_pura() -> Value {
    let mut filas = Vec::new();
    for (gamma, sesgo) in [(1.0, 0.0), (2.0, 0.0), (4.0, 0.0), (4.0, 0.2), (4.0, 1.5)] {
        let finales: Vec<f64> = (0..CORRIDAS).map(|s| urna(gamma, sesgo, s)).collect();
        let fraccion = |f: &dyn Fn(f64) -> bool| {
            finales.iter().filter(|x| f(**x)).count() as f64 / CORRIDAS as f64
        };
        filas.push(json!({"gamma":gamma,"sesgo":sesgo,
            // Hipótesis: la forma local es la horquilla con r = p'(1/2) − 1 = γ/2 − 1.
            "r":gamma / 2.0 - 1.0,
            "atractores_de_la_edo":atractores(gamma, sesgo),
            "corridas_que_terminan_abajo":fraccion(&|x| x < 0.25),
            "corridas_que_terminan_arriba":fraccion(&|x| x > 0.75),
            "corridas_que_quedan_en_el_medio":fraccion(&|x| (0.25..=0.75).contains(&x))}));
    }
    // h_c de la forma normal cúbica (h = b/4, r = γ/2 − 1, c = γ³/6) contra el exacto de la urna.
    let gamma: f64 = 4.0;
    let (r, c) = (gamma / 2.0 - 1.0, gamma.powi(3) / 6.0);
    let aproximado = 4.0 * (2.0 * r / 3.0) * (r / (3.0 * c)).sqrt();
    json!({"datos":"sinteticos","extracciones":EXTRACCIONES,"corridas":CORRIDAS,"filas":filas,
        "sesgo_critico_con_gamma_4":{"exacto_de_la_urna":sesgo_critico(gamma),"forma_normal_cubica":aproximado}})
}

/// La bifurcación aparece donde lo dice la ecuación de la urna: con r < 0 nadie se
/// encierra; con r > 0 cada corrida cae en un atractor u otro según sus primeros sorteos;
/// con un sesgo mayor al crítico el atractor perdedor desaparece.
#[test]
fn la_urna_pura_bifurca_donde_dice_su_ecuacion() {
    let u = urna_pura();
    println!("{}", serde_json::to_string_pretty(&u).unwrap());
    assert_eq!(u, urna_pura());
    let n = |fila: usize, campo: &str| u["filas"][fila][campo].as_f64().unwrap();
    let atractores = |fila: usize| {
        u["filas"][fila]["atractores_de_la_edo"]
            .as_array()
            .unwrap()
            .len()
    };
    // γ = 1 (r = −0,5): un solo atractor en el medio y ninguna corrida se encierra.
    assert_eq!(atractores(0), 1);
    assert_eq!(n(0, "corridas_que_quedan_en_el_medio"), 1.0);
    // γ = 4 (r = 1): dos atractores, y qué camino gana depende de la corrida.
    assert_eq!(atractores(2), 2);
    assert!(n(2, "corridas_que_quedan_en_el_medio") < 0.02);
    assert!(
        n(2, "corridas_que_terminan_abajo") > 0.3 && n(2, "corridas_que_terminan_arriba") > 0.3
    );
    // Horquilla imperfecta: un sesgo chico inclina sin extinguir; uno mayor al crítico extingue.
    assert_eq!(atractores(3), 2);
    assert!(n(3, "corridas_que_terminan_arriba") > n(3, "corridas_que_terminan_abajo"));
    assert!(n(3, "corridas_que_terminan_abajo") > 0.02);
    assert_eq!(atractores(4), 1);
    assert_eq!(n(4, "corridas_que_terminan_abajo"), 0.0);
    // La forma normal cúbica ubica bien DÓNDE aparece la bifurcación (r = 0 en γ = 2), pero
    // con una S fuerte los atractores quedan lejos del medio y subestima el sesgo crítico
    // a menos de la mitad: sirve como forma local, no para calcular umbrales.
    let critico = &u["sesgo_critico_con_gamma_4"];
    let (exacto, cubico) = (
        critico["exacto_de_la_urna"].as_f64().unwrap(),
        critico["forma_normal_cubica"].as_f64().unwrap(),
    );
    assert!((exacto - 1.066).abs() < 0.01, "{exacto}");
    assert!(
        cubico / exacto > 0.3 && cubico / exacto < 0.6,
        "exacto {exacto}, cúbico {cubico}"
    );
}

// ---------- Simulación 2: dependencias, coexistencia y sucesión ----------

const CONTEXTOS: usize = 8;
const SIN_DEPENDENCIAS: usize = 2;
const PASOS_POR_CAMINO: usize = 4;
const USOS: usize = 6000;
const APARECE: usize = 1000;
/// Cuánto frena cada paso que el contexto no tiene. Las dependencias de un contexto son
/// fijas: son su modelo de producción, no algo que se instala con un éxito.
const KAPPA: f64 = 1.5;
const CALIDAD: [f64; 2] = [0.8, 0.9];
/// Generación ciega: con esta probabilidad se prueba el camino nuevo sin mirar su
/// atracción, pero sólo donde probar no cuesta más que seguir igual. Sin esto nada
/// nuevo se prueba nunca, porque un camino sin historia no atrae a nadie.
const THETA: f64 = 0.05;

#[derive(Clone, Copy, PartialEq)]
enum Atraccion {
    /// Rendimientos crecientes puros: atrae la cuota de éxitos acumulados.
    Cuota,
    /// Atrae la tasa de éxito observada de cada camino, descontada mientras tiene pocos usos.
    Tasa,
}

/// Un tipo de objetivo con un camino viejo y uno nuevo, mejor, que comparte `solapados` de
/// sus cuatro pasos. Seis contextos dependen de los pasos del viejo; dos no dependen de
/// nada. Cada uso elige camino por su atracción menos el costo de los pasos que al contexto
/// le faltan. Devuelve, por contexto, la fracción de sus últimos usos que fueron del nuevo.
fn mundo(solapados: usize, semilla: u64, atraccion: Atraccion) -> Vec<f64> {
    let mut azar = Azar(semilla.wrapping_mul(7919) + solapados as u64);
    let pasos = [
        (0..PASOS_POR_CAMINO).collect::<BTreeSet<usize>>(),
        (PASOS_POR_CAMINO - solapados..2 * PASOS_POR_CAMINO - solapados).collect(),
    ];
    let (mut exitos, mut usos) = ([0.0f64; 2], [0.0f64; 2]);
    let mut ultimos: Vec<Vec<bool>> = vec![Vec::new(); CONTEXTOS];
    for t in 0..USOS {
        let (c, elige, sale) = (
            (azar.uniforme() * CONTEXTOS as f64) as usize,
            azar.uniforme(),
            azar.uniforme(),
        );
        // A un contexto sin dependencias le faltan los pasos de cualquier camino.
        let faltan = |camino: usize| {
            if c < SIN_DEPENDENCIAS {
                PASOS_POR_CAMINO as f64
            } else {
                pasos[camino].difference(&pasos[0]).count() as f64
            }
        };
        let ventaja = match atraccion {
            Atraccion::Cuota => 3.0 * (2.0 * exitos[1] / (exitos[0] + exitos[1]).max(1.0) - 1.0),
            Atraccion::Tasa => 30.0 * (exitos[1] / (usos[1] + 5.0) - exitos[0] / (usos[0] + 5.0)),
        };
        let costo = faltan(1) - faltan(0);
        let ciega = if costo <= 0.0 { THETA } else { 0.0 };
        let nueva = t >= APARECE && elige < logistica(ventaja - KAPPA * costo).max(ciega);
        let camino = usize::from(nueva);
        usos[camino] += 1.0;
        exitos[camino] += f64::from(u8::from(sale < CALIDAD[camino]));
        ultimos[c].push(nueva);
    }
    ultimos
        .iter()
        .map(|u| {
            let cola = &u[u.len().saturating_sub(100)..];
            cola.iter().filter(|n| **n).count() as f64 / cola.len().max(1) as f64
        })
        .collect()
}

fn destinos(atraccion: Atraccion) -> Value {
    let corridas = 200u64;
    let filas: Vec<Value> = (0..=PASOS_POR_CAMINO)
        .map(|solapados| {
            let (mut sucesion, mut coexistencia, mut sin_adopcion, mut mal_servidos) =
                (0u64, 0u64, 0u64, 0.0);
            for semilla in 0..corridas {
                let adopcion = mundo(solapados, semilla, atraccion);
                let nuevos = adopcion.iter().filter(|a| **a > 0.5).count();
                let viejos = CONTEXTOS - nuevos;
                sucesion += u64::from(viejos == 0);
                sin_adopcion += u64::from(nuevos == 0);
                coexistencia += u64::from(nuevos > 0 && viejos > 0);
                // Una sola vigente para todo el tipo deja mal servida a la minoría.
                mal_servidos += nuevos.min(viejos) as f64 / CONTEXTOS as f64;
            }
            let f = |n: u64| n as f64 / corridas as f64;
            json!({"pasos_compartidos":solapados,"pasos_nuevos_para_los_viejos":PASOS_POR_CAMINO - solapados,
                "sucesion":f(sucesion),"coexistencia":f(coexistencia),"sin_adopcion":f(sin_adopcion),
                "contextos_mal_servidos_por_una_sola_vigente":mal_servidos / corridas as f64})
        })
        .collect();
    json!({"datos":"sinteticos","contextos":CONTEXTOS,"sin_dependencias":SIN_DEPENDENCIAS,"usos":USOS,
        "kappa":KAPPA,"calidad":CALIDAD,"corridas":corridas,"filas":filas})
}

/// El riesgo de que la memoria misma sea el rendimiento creciente: si lo único que atrae es
/// la cuota acumulada, un camino mejor que llega tarde no entra, ni siquiera cuando
/// adoptarlo no cuesta nada y aunque se lo pruebe a ciegas una de cada veinte veces.
#[test]
fn con_rendimientos_crecientes_puros_un_camino_mejor_que_llega_tarde_no_entra() {
    let d = destinos(Atraccion::Cuota);
    println!("{}", serde_json::to_string_pretty(&d).unwrap());
    for fila in d["filas"].as_array().unwrap() {
        assert_eq!(fila["sin_adopcion"], 1.0, "{fila}");
    }
}

/// La tesis en chico, con parámetros elegidos a mano: ilustra, no prueba. El destino del
/// camino viejo depende de cuántos pasos comparte con el nuevo. Si comparten todo, el nuevo
/// lo reemplaza; si no comparten nada, conviven (lo adoptan los contextos sin dependencias)
/// y una sola vigente para el tipo sirve mal a una parte.
#[test]
fn el_solapamiento_de_pasos_separa_sucesion_de_coexistencia() {
    let d = destinos(Atraccion::Tasa);
    println!("{}", serde_json::to_string_pretty(&d).unwrap());
    assert_eq!(d, destinos(Atraccion::Tasa));
    let n = |fila: usize, campo: &str| d["filas"][fila][campo].as_f64().unwrap();
    // Con todos los pasos compartidos adoptar no cuesta: sucesión.
    assert!(n(PASOS_POR_CAMINO, "sucesion") > 0.9);
    assert!(
        n(
            PASOS_POR_CAMINO,
            "contextos_mal_servidos_por_una_sola_vigente"
        ) < 0.02
    );
    // Sin pasos compartidos conviven: los dos contextos sin dependencias adoptan y los
    // seis que dependen del viejo se quedan. Una sola vigente deja mal a 2 de 8.
    assert_eq!(n(0, "sucesion"), 0.0);
    assert!(n(0, "coexistencia") > 0.9);
    assert!((n(0, "contextos_mal_servidos_por_una_sola_vigente") - 0.25).abs() < 0.02);
    // Compartir más pasos nunca aleja la sucesión.
    for s in 0..PASOS_POR_CAMINO {
        assert!(n(s + 1, "sucesion") >= n(s, "sucesion") - 0.05, "{s}");
    }
}
