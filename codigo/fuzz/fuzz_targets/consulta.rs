//! El prompt del usuario y el texto recuperado: la consulta queda acotada y el
//! sobre suma siempre los mismos bytes, para cualquier texto.
#![no_main]
use libfuzzer_sys::fuzz_target;
use msp::adapter::{consulta_literal, envolver, sobre_bytes};

fuzz_target!(|texto: &str| {
    let consulta = consulta_literal(texto);
    assert!(consulta.split(' ').filter(|t| !t.is_empty()).count() <= 64);
    assert_eq!(envolver(texto).len(), texto.len() + sobre_bytes());
});
