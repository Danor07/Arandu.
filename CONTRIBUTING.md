# Cómo contribuir

Gracias por el interés. El proyecto es chico y experimental; estas reglas existen para que siga siendo confiable.

## Antes de escribir código

Abrí un issue contando qué querés cambiar y por qué. Para cambios chicos (un error, una prueba, un texto) podés ir directo al pull request.

## Lo que tiene que pasar

Desde `codigo/`:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

La CI corre lo mismo en cada pull request, más la mutación de las líneas que cambiaste (`cargo mutants --in-diff`), con un umbral del 90 % de mutantes detectados y ningún timeout.

## Reglas de la casa

- **No se editan las migraciones publicadas** (`codigo/migrations/` y `codigo/registro/`). Las bases verifican su hash. Lo nuevo va en una migración nueva.
- **Cada cambio de comportamiento trae sus pruebas:** una de regresión por cada defecto corregido, con un nombre que diga qué defecto fija, y una de propiedad (`proptest`) cuando hay lógica pura.
- **Las reglas que deciden llevan versión** (una constante `*_VERSION`) y las tablas nuevas son append-only.
- **Se declara lo que no se comprobó.** Si algo se probó sólo con datos sintéticos, o no se probó contra el host real, el README lo dice. Preferimos un límite escrito a una promesa.
- **El cambio más chico que resuelve el problema.** Sin dependencias nuevas por pocas líneas, sin opciones que nadie pidió.
- **Español** en nombres, mensajes y documentación, como el resto del código.
- **Nada de datos personales ni secretos** en el código, las pruebas o los ejemplos.

## Licencia de las contribuciones

Al enviar un pull request aceptás que tu contribución se publique bajo Apache-2.0, la licencia del proyecto.
