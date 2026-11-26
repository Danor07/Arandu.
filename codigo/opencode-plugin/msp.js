// Reenvia eventos de OpenCode a `msp --hook --plataforma opencode`. Sin logica
// propia: el mapeo, la puerta de sesion y la recuperacion viven en el binario.
// Copiar a .opencode/plugins/msp.js (o ~/.config/opencode/plugins/msp.js).
import { spawnSync } from "node:child_process"

// Tope por evento, como los de codex/hooks.json: spawnSync bloquea al host.
const TOPE_MS = Number(process.env.MSP_TOPE_MS) || 30_000

export default {
  id: "msp",
  async setup(ctx) {
    // sessionID -> bloque que devolvio el ultimo prompt de esa sesion.
    const contexto = new Map()
    // Un fallo de la memoria nunca frena el prompt ni la herramienta: ni un
    // dato que JSON no serializa, ni un binario ausente, colgado o con error.
    const msp = (evento, datos) => {
      try {
        const r = spawnSync(process.env.MSP_BIN || "msp", ["--hook", "--plataforma", "opencode"], {
          input: JSON.stringify({ hook_event_name: evento, cwd: ctx.location.directory, ...datos }),
          encoding: "utf8",
          timeout: TOPE_MS,
        })
        return r.status === 0 ? r.stdout : ""
      } catch {
        return ""
      }
    }
    await ctx.session.hook("prompt", (e) => {
      const bloque = msp("prompt", { sessionID: e.sessionID, messageID: e.messageID, prompt: e.prompt })
      if (bloque) contexto.set(e.sessionID, bloque)
      else contexto.delete(e.sessionID)
    })
    await ctx.session.hook("context", (e) => {
      const bloque = contexto.get(e.sessionID)
      if (bloque) e.system.push({ type: "text", text: bloque })
    })
    // `info` trae la definicion de la herramienta, con funciones: no se reenvia.
    await ctx.tool.hook("execute.after", ({ info, ...e }) => {
      msp("execute.after", e)
    })
  },
}
