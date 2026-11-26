#!/usr/bin/env python3
"""Ejemplo minimo de envoltorio: un chat con Ollama que avisa cada evento a
`msp --hook --plataforma local`. No es un cliente de chat: muestra el contrato.

Uso: python3 envoltorio_ollama.py MODELO   (msp en el PATH, Ollama en localhost:11434)
"""
import json
import os
import shutil
import subprocess
import sys
import urllib.request
import uuid

URL = os.environ.get("OLLAMA_URL", "http://localhost:11434/api/chat")
BIN = os.environ.get("MSP_BIN", "msp")
# Se valida antes de capturar nada: un prompt guardado sin modelo no sirve.
if len(sys.argv) != 2:
    sys.exit("uso: envoltorio_ollama.py MODELO")
if not shutil.which(BIN):
    sys.exit(f"no se encuentra msp ({BIN}): ponerlo en el PATH o en MSP_BIN")
sesion, mensajes = f"ollama-{uuid.uuid4()}", []


def msp(evento, **campos):
    """Manda un evento del contrato y devuelve lo que msp ofrece como contexto."""
    payload = {"hook_event_name": evento, "session_id": sesion, "cwd": os.getcwd(), **campos}
    r = subprocess.run([BIN, "--hook", "--plataforma", "local"],
                       input=json.dumps(payload), capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


msp("SessionStart")
print(f"sesion local:{sesion}; encender con: "
      f"msp --control sesion-usar --sesion local:{sesion} --cwd . --memoria ID")
motivo = "salida_del_envoltorio"
try:
    while True:
        texto, turno = input("> "), str(uuid.uuid4())
        # Solo texto tipeado por la persona viaja como UserPromptSubmit.
        antecedentes = msp("UserPromptSubmit", prompt=texto, turn_id=turno)
        mensajes.append({"role": "user", "content": antecedentes + texto})
        pedido = urllib.request.Request(
            URL,
            json.dumps({"model": sys.argv[1], "messages": mensajes, "stream": False}).encode(),
            {"Content-Type": "application/json"})
        respuesta = json.load(urllib.request.urlopen(pedido))
        mensaje = respuesta.get("message") if isinstance(respuesta, dict) else None
        if not isinstance(mensaje, dict) or not isinstance(mensaje.get("content"), str):
            raise ValueError(f"respuesta sin message.content: {str(respuesta)[:200]}")
        mensajes.append(mensaje)
        print(mensaje["content"])
        msp("Stop", last_assistant_message=mensaje["content"], turn_id=turno)
except (EOFError, KeyboardInterrupt):
    pass
except (OSError, ValueError) as error:
    # URLError es OSError; un cuerpo que no es JSON, ValueError.
    motivo = "ollama_no_respondio"
    sys.exit(f"ollama no respondio como se esperaba: {error}")
finally:
    # Siempre: sin SessionEnd el intervalo de captura queda abierto.
    msp("SessionEnd", reason=motivo)
