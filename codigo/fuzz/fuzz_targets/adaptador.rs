//! El JSON que manda un hook: cualquier payload termina en Ok o Err, sin pánico.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::sync::{Mutex, OnceLock};

static ADAPTADOR: OnceLock<Mutex<(tempfile::TempDir, msp::adapter::Adapter)>> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(data) else { return };
    let celda = ADAPTADOR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let registro = msp::control::Registry::open(dir.path()).unwrap();
        Mutex::new((dir, msp::adapter::Adapter::new(registro, "fuzz".into(), 4096)))
    });
    let _ = celda.lock().unwrap().1.hook(&payload);
});
