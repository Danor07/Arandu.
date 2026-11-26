//! Un evento crudo cualquiera: la captura responde Ok o Err, nunca entra en
//! pánico, y si acepta un evento lo acepta igual la segunda vez.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::{Mutex, OnceLock};

static STORE: OnceLock<Mutex<(tempfile::TempDir, msp::Store)>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let Ok(crudo) = std::str::from_utf8(data) else { return };
    let celda = STORE.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let store = msp::Store::open(dir.path().join("fuzz.sqlite")).unwrap();
        Mutex::new((dir, store))
    });
    let mut guarda = celda.lock().unwrap();
    if let Ok(primero) = guarda.1.capture(crudo) {
        let segundo = guarda.1.capture(crudo).expect("un evento aceptado se acepta de nuevo");
        assert!(!segundo.insertado);
        assert_eq!(primero.instancia_id, segundo.instancia_id);
    }
});
