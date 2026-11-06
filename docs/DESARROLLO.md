# Desarrollo

Notas internas del proyecto (no necesarias para instalar los mods).

## Ramas y publicación

- `main` (desarrollo, la única rama local), `pre-release` y `stable`
  (solo en el remoto).
- Para publicar: `./subir-pre-release.sh` (build de prueba) o
  `./subir-stable.sh` (release estable). Ambos empujan `main` a la rama
  correspondiente y disparan GitHub Actions, que compila y actualiza el
  release homónimo.

## Compilación local (Windows)

`deploy.bat` (MSVC, `i686-pc-windows-msvc`) genera `output\`.

## Verificación

Tras tocar cualquier cave o parche:

```
python3 verificar_mods.py Patrician3.exe
```

Reconstruye los bytes de cada cave, los desensambla con objdump y comprueba
los parches contra el exe original.

## Detalles técnicos

- `docs/DIRECCIONES_EXE_ESPANOL.md` — direcciones del exe español.
- `docs/analisis_mision_fundar_ciudad.md` — generador de misiones de gobernador.
- `AGENTS.md` — contexto completo del proyecto.
