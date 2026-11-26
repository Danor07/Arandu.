//! Lo único que difiere entre hosts. La puerta de sesión, el canal de control,
//! la captura idempotente, la recuperación y la entrega viven una sola vez en
//! `adapter.rs`; una plataforma aporta su nombre, sus versiones, cómo traducir
//! su evento al vocabulario común y qué garantías da el host.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};

pub struct Plataforma {
    /// Prefijo de grupo, clave de evento e intento: `<nombre>/<instalación>/<sesión>`,
    /// donde `<sesión>` es la del registro (ver `sesion`).
    pub nombre: &'static str,
    pub adaptador: &'static str,
    /// Versión del host con prueba nativa, o la declaración de que no la hay.
    pub host_probado: &'static str,
    /// Campo del payload que correlaciona los eventos de un mismo turno.
    pub(crate) turno: &'static str,
    /// Sólo si el host garantiza que `prompt` es texto tipeado por el usuario.
    /// Sin esa garantía las órdenes `msp::` no se ejecutan y se usa la CLI.
    pub(crate) control_por_prompt: bool,
    /// El host no avisa el inicio de sesión: se materializa en el primer prompt.
    pub(crate) inicio_implicito: bool,
    /// Renombra el evento del host al vocabulario común. `None`: ya lo habla.
    pub(crate) traducir: Option<fn(&Value) -> Value>,
    /// Cómo llega al modelo lo que `msp --hook` escribe en stdout.
    pub(crate) canal: &'static str,
    pub cobertura: fn() -> Value,
}

pub const CLAUDE_CODE: Plataforma = Plataforma {
    nombre: "claude-code",
    adaptador: crate::control::ADAPTADOR_VERSION,
    host_probado: crate::control::HOST_PROBADO,
    turno: "prompt_id",
    control_por_prompt: true,
    inicio_implicito: false,
    traducir: None,
    canal: "stdout de hook UserPromptSubmit como contexto de texto plano",
    cobertura: crate::control::cobertura,
};

pub const CODEX: Plataforma = Plataforma {
    nombre: "codex",
    adaptador: "codex-adaptador-v1",
    host_probado:
        "ninguno: solo documentacion leida el 2026-11-25, que no publica numero de version",
    turno: "turn_id",
    // La documentación no excluye que un subagente o la continuación de un hook
    // Stop («acts as a new user prompt») disparen UserPromptSubmit.
    control_por_prompt: false,
    inicio_implicito: false,
    traducir: None,
    canal: "stdout de hook UserPromptSubmit, que Codex agrega como contexto de desarrollador",
    cobertura: cobertura_codex,
};

pub const OPENCODE: Plataforma = Plataforma {
    nombre: "opencode",
    adaptador: "opencode-adaptador-v1",
    host_probado: "ninguno: solo documentacion y tipos publicados de @opencode/plugin 2.4.1",
    turno: "turn_id",
    // El hook `prompt` también corre para comandos y «other prompt sources».
    control_por_prompt: false,
    inicio_implicito: true,
    traducir: Some(de_opencode),
    canal: "stdout devuelto al plugin, que lo agrega a event.system en el hook context",
    cobertura: cobertura_opencode,
};

pub const LOCAL: Plataforma = Plataforma {
    nombre: "local",
    adaptador: "local-adaptador-v1",
    host_probado: "no aplica: el contrato de eventos lo define este repo y lo cumple el envoltorio",
    turno: "turn_id",
    // Ningún host respalda que el envoltorio mande sólo texto tipeado.
    control_por_prompt: false,
    inicio_implicito: false,
    traducir: None,
    canal: "stdout de msp --hook, que el envoltorio antepone al mensaje que manda al modelo",
    cobertura: cobertura_local,
};

pub const PLATAFORMAS: &[&Plataforma] = &[&CLAUDE_CODE, &CODEX, &OPENCODE, &LOCAL];

pub fn plataforma(nombre: &str) -> Result<&'static Plataforma> {
    PLATAFORMAS
        .iter()
        .copied()
        .find(|p| p.nombre == nombre)
        .with_context(|| {
            let conocidas: Vec<_> = PLATAFORMAS.iter().map(|p| p.nombre).collect();
            format!(
                "plataforma desconocida: {nombre}; conocidas: {}",
                conocidas.join(", ")
            )
        })
}

/// La plataforma de una sesión del registro: la del prefijo `<nombre>:`, y sin
/// prefijo Claude Code, cuyas sesiones conservan el ID del host tal cual.
pub fn de_sesion(sesion: &str) -> &'static Plataforma {
    PLATAFORMAS
        .iter()
        .copied()
        .find(|p| {
            sesion
                .strip_prefix(p.nombre)
                .is_some_and(|resto| resto.starts_with(':'))
        })
        .unwrap_or(&CLAUDE_CODE)
}

impl Plataforma {
    /// Nombre de la sesión en el registro de control, que es el que lleva
    /// `--control ... --sesion`. Fuera de Claude Code incluye la plataforma
    /// (`codex:<id>`): dos hosts con el mismo ID nunca comparten fila.
    pub fn sesion(&self, id: &str) -> Result<String> {
        if self.nombre == CLAUDE_CODE.nombre {
            ensure!(
                de_sesion(id).nombre == self.nombre,
                "session_id reservado a otra plataforma: {id:?}"
            );
            return Ok(id.to_string());
        }
        // El ID viaja a claves, a la CLI y al aviso de control: sólo texto llano.
        ensure!(
            (1..=128).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
            "session_id invalido en {}: se exigen de 1 a 128 caracteres entre letras ASCII, digitos, '.', '_' y '-'",
            self.nombre
        );
        Ok(format!("{}:{id}", self.nombre))
    }
}

/// Lo que cada adaptador declara de sí mismo, para `--control plataformas`.
pub fn tabla() -> Value {
    PLATAFORMAS
        .iter()
        .map(|p| {
            json!({"plataforma":p.nombre,"adaptador":p.adaptador,"host_probado":p.host_probado,
            "control_por_prompt":p.control_por_prompt,"canal_de_contexto":p.canal,
            "captura":(p.cobertura)()})
        })
        .collect()
}

fn renombrar(payload: &mut Value, de: &str, a: &str) {
    if let Some(valor) = payload.as_object_mut().and_then(|o| o.remove(de)) {
        payload[a] = valor;
    }
}

/// (ID del host, nombre común, [(campo de entrada del host, campo común)]).
type Equivalencias = &'static [(
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
)];

/// Herramientas de OpenCode cuyo nombre y entrada confirma su documentación
/// (<https://opencode.ai/docs/tools/>, leída el 2026-11-25). `shell`, `glob` y
/// `webfetch` no entran: la página no nombra el campo del comando, del patrón ni
/// de la URL; `patch` no tiene equivalente de un solo archivo. Codex no necesita
/// tabla: su documentación ya da `Bash` con `tool_input.command`.
const HERRAMIENTAS_OPENCODE: Equivalencias = &[
    ("read", "Read", &[("path", "file_path")]),
    ("edit", "Edit", &[("path", "file_path")]),
    ("write", "Write", &[("path", "file_path")]),
    ("grep", "Grep", &[]),
    ("websearch", "WebSearch", &[]),
];

/// Pasa al vocabulario común una herramienta que la tabla trae, para que cortes
/// y normalización la reconozcan; el ID original queda en `herramienta_host`.
/// Las que no están en la tabla siguen con el nombre y la entrada del host.
fn equivalencia(p: &mut Value, tabla: Equivalencias) {
    let Some(&(host, comun, campos)) = tabla.iter().find(|(host, ..)| p["tool_name"] == *host)
    else {
        return;
    };
    p["herramienta_host"] = json!(host);
    p["tool_name"] = json!(comun);
    if let Some(entrada) = p.get_mut("tool_input") {
        for (de, a) in campos {
            renombrar(entrada, de, a);
        }
    }
}

/// Renombre sin pérdida de los eventos que reenvía `opencode-plugin/msp.js`.
/// Campos según los tipos publicados `SessionPrompt` y `ToolHooks["execute.after"]`.
fn de_opencode(host: &Value) -> Value {
    let mut p = host.clone();
    // Sólo entra lo que habla OpenCode: el vocabulario común no pasa tal cual.
    if let Some(objeto) = p.as_object_mut() {
        for comun in [
            "session_id",
            "turn_id",
            "tool_use_id",
            "tool_name",
            "tool_input",
            "tool_response",
            "last_assistant_message",
            "agent_id",
            "agent_type",
            "herramienta_host",
        ] {
            objeto.remove(comun);
        }
    }
    renombrar(&mut p, "sessionID", "session_id");
    renombrar(&mut p, "hook_event_name", "evento_host");
    let Some(nombre) = host["hook_event_name"].as_str() else {
        return p;
    };
    let evento = match (nombre, host["status"].as_str()) {
        ("prompt", _) => "UserPromptSubmit".to_string(),
        ("execute.after", Some("completed")) => "PostToolUse".to_string(),
        ("execute.after", Some("error")) => "PostToolUseFailure".to_string(),
        // Sin equivalente: un nombre que el núcleo no puede confundir con uno suyo.
        _ => format!("opencode:{nombre}"),
    };
    p["hook_event_name"] = json!(evento);
    if evento == "UserPromptSubmit" {
        // El `messageID` del prompt es estable ante reintentos de admisión. El de
        // una herramienta es de otro mensaje y no se usa como turno.
        renombrar(&mut p, "messageID", "turn_id");
        renombrar(&mut p, "prompt", "prompt_host");
        p["prompt"] = host["prompt"]["text"].clone();
    } else if evento.starts_with("PostToolUse") {
        renombrar(&mut p, "id", "tool_use_id");
        renombrar(&mut p, "tool", "tool_name");
        renombrar(&mut p, "input", "tool_input");
        renombrar(&mut p, "result", "tool_response");
        equivalencia(&mut p, HERRAMIENTAS_OPENCODE);
    }
    p
}

fn cobertura_codex() -> Value {
    json!([
        {"hook":"SessionStart","captura":"ninguna","control":"materializa la fila de sesion y aplica el predeterminado",
         "limite":"source startup, resume, clear, compact o fork; una sesion ya conocida no se reinicializa"},
        {"hook":"UserPromptSubmit","captura":"texto del prompt","campo":"prompt + turn_id","origen":"usuario","rol":"problema",
         "completitud":"completa segun el host","limite":"unico punto donde se ofrece contexto; no acepta ordenes msp::"},
        {"hook":"PostToolUse","captura":"entrada y salida de la herramienta","campo":"tool_input + tool_response + tool_use_id","origen":"herramienta","rol":"otro",
         "completitud":"lo que el host serializa","limite":"Codex tambien lo emite para Bash con salida no cero: herramienta-ok aqui no distingue exito de fallo"},
        {"hook":"Stop","captura":"texto final del asistente del turno","campo":"last_assistant_message + turn_id","origen":"modelo","rol":"solucion",
         "completitud":"solo el mensaje final; puede venir null","limite":"fin de turno no prueba exito"},
        {"hook":"SubagentStop","captura":"texto final del subagente","campo":"last_assistant_message + agent_id","origen":"modelo","rol":"otro",
         "completitud":"solo el mensaje final","limite":"llega con el session_id del padre"},
        {"hook":"PreCompact","captura":"marcador de frontera, sin contenido","origen":"herramienta","rol":"otro",
         "completitud":"solo el disparador","limite":"compactar no crea otra seleccion ni reimporta lo anterior"},
        {"hook":"SessionEnd","captura":"ninguna","control":"cierra el intervalo abierto",
         "limite":"no corre para subagentes; reason hoy es siempre other"},
        {"hook":"PreToolUse, PermissionRequest, PostCompact, SubagentStart, PreModelRequest, PostModelRequest, Interrupt",
         "captura":"ninguna","limite":"sin mapeo declarado: no se instalan y, si llegan, devuelven capturado=false"},
        {"fuente":"transcript_path","uso":"no se lee","limite":"el host declara que su formato no es una interfaz estable"}
    ])
}

fn cobertura_opencode() -> Value {
    json!([
        {"hook":"prompt","equivale_a":"UserPromptSubmit","captura":"texto del prompt admitido","campo":"prompt.text + messageID","origen":"usuario","rol":"problema",
         "completitud":"el texto; archivos, agentes y skills quedan en payload_hook.prompt_host",
         "limite":"tambien corre para comandos y otras fuentes de prompt: no acepta ordenes msp::. Materializa la sesion, porque no hay evento de inicio"},
        {"hook":"execute.after (status completed)","equivale_a":"PostToolUse","captura":"entrada y resultado de la herramienta","campo":"tool + input + result + id","origen":"herramienta","rol":"otro",
         "completitud":"lo que el plugin puede serializar a JSON","limite":"el messageID de la herramienta no identifica el turno: el enlace al intento queda pendiente"},
        {"hook":"execute.after (status error)","equivale_a":"PostToolUseFailure","captura":"error de la herramienta","campo":"tool + input + error","origen":"herramienta","rol":"otro",
         "completitud":"el objeto error del host","limite":"un fallo de herramienta no prueba fallo global"},
        {"fuente":"herramientas","uso":"read, edit, write, grep y websearch se guardan como Read, Edit, Write, Grep y WebSearch (path pasa a file_path), con el ID original en herramienta_host",
         "limite":"shell, glob, webfetch y patch quedan con el nombre del host y no producen pasos normalizados: la documentacion no nombra sus campos o no hay equivalente"},
        {"hook":"context","captura":"ninguna","control":"el plugin agrega a event.system el bloque que devolvio el ultimo prompt de esa sesion",
         "limite":"se repite en cada llamada al modelo hasta el proximo prompt; el estado vive en memoria del plugin"},
        {"hook":"respuesta del asistente, fin de turno, fin de sesion, compactacion, fork",
         "captura":"ninguna","limite":"sin mapeo: el texto del asistente no se captura y el intervalo solo se cierra por CLI"}
    ])
}

fn cobertura_local() -> Value {
    json!([
        {"hook":"SessionStart","captura":"ninguna","campo":"session_id + cwd","control":"materializa la fila de sesion y aplica el predeterminado",
         "limite":"el envoltorio elige session_id; repetirlo continua la misma sesion"},
        {"hook":"UserPromptSubmit","captura":"texto del prompt","campo":"prompt + turn_id","origen":"usuario","rol":"problema",
         "completitud":"lo que mande el envoltorio","limite":"unico punto donde se ofrece contexto; no acepta ordenes msp::"},
        {"hook":"PostToolUse","captura":"entrada y salida de la herramienta","campo":"tool_name + tool_input + tool_response + tool_use_id","origen":"herramienta","rol":"otro",
         "completitud":"lo que mande el envoltorio","limite":"ejecutar una herramienta no valida nada"},
        {"hook":"PostToolUseFailure","captura":"error de la herramienta","campo":"tool_name + tool_input + error + tool_use_id","origen":"herramienta","rol":"otro",
         "completitud":"lo que mande el envoltorio","limite":"un fallo de herramienta no prueba fallo global"},
        {"hook":"Stop","captura":"texto final del asistente del turno","campo":"last_assistant_message + turn_id","origen":"modelo","rol":"solucion",
         "completitud":"lo que mande el envoltorio","limite":"fin de turno no prueba exito"},
        {"hook":"SubagentStop, PreCompact","captura":"igual que en Claude Code","limite":"opcionales; un envoltorio simple no los manda"},
        {"hook":"SessionEnd","captura":"ninguna","control":"cierra el intervalo abierto","limite":"si el envoltorio muere sin mandarlo, el intervalo queda abierto"},
        {"fuente":"envoltorio","uso":"no hay host que respalde los campos",
         "limite":"origen y rol son los que el envoltorio declara al elegir el evento; sin turn_id o tool_use_id la identidad es el hash del payload"}
    ])
}
