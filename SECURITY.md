# Seguridad

## Cómo avisar de un problema

Usá el reporte privado de GitHub: pestaña **Security** del repositorio → **Report a vulnerability**. No abras un issue público para algo que se pueda explotar.

El proyecto lo mantiene una persona en su tiempo libre: no hay plazo de respuesta garantizado.

## Qué protege Arandu

- **Todo es local.** Los datos se guardan en SQLite en tu máquina. El programa no abre conexiones de red.
- **Cada chat nace apagado.** Instalar los hooks no captura nada hasta que el usuario vincula una memoria.
- **Lo recuperado no tiene autoridad.** Los antecedentes y los mensajes de otros agentes llegan al modelo dentro de un sobre marcado `autoridad="ninguna"`, con un identificador que el contenido no puede falsificar ni cerrar.
- **Las órdenes `msp::` sólo valen desde texto tipeado por el usuario**, y hoy sólo en Claude Code, que es el único host donde eso está verificado.
- **`git` se invoca sin shell** y sin dejar que la configuración del repo ejecute programas.
- **El historial no se reescribe.** Las tablas de evidencia rechazan `UPDATE`, `DELETE` e `INSERT OR REPLACE`.

## Qué no protege

- **La CLI no autentica a quien la invoca.** Cualquier proceso local con acceso a la carpeta de datos puede pasar el identificador de otro chat, publicar en su nombre o cambiar reglas y roles. Los topes de los canales frenan a un agente en bucle, no a un adversario.
- **Los datos no están cifrados.** La memoria contiene tus prompts y las salidas de tus herramientas en texto plano. Quien lea tu disco, los lee.
- **Quien puede escribir en la base con SQL directo puede hacer cualquier cosa**, incluido quitar los triggers.
- **Que el modelo respete `autoridad="ninguna"` no se puede garantizar.** Es una marca, no un control.
- **Los adaptadores de Codex, OpenCode y modelos locales no se probaron contra sus hosts reales.**

## Versiones

Sólo se corrige la rama `main`.
