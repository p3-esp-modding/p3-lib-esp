# p3-lib-esp — Mods para Patrician 3 (español)

Mods en forma de DLL para el **Patrician 3 español**. Corrigen bugs y añaden
mejoras sin modificar el ejecutable original: los cambios se aplican en memoria
cada vez que arranca el juego, así que tus partidas guardadas no se tocan.

Basado en el diseño del [modloader de la comunidad inglesa](https://github.com/P3Modding/p3-lib).

## Descarga

Ve a la pestaña [**Releases**](https://github.com/p3-esp-modding/p3-lib-esp/releases)
y descarga del release **`stable`**:

- **`p3-esp-dlls.zip`** — los mods.
- **`p3_esp_mods.cfg`** — la configuración.

Si sale una versión nueva, vuelve a descargar ambos: el zip se extrae igual,
pero el `p3_esp_mods.cfg` **no lo sobrescribas** si ya lo tienes configurado a
tu gusto (compara primero por si hay opciones nuevas).

## Instalación

Requisito: el juego original español instalado (vale la versión de Steam o CD).

1. **Haz una copia de seguridad** de tu `Patrician3.exe` (por ejemplo,
   renómbralo a `Patrician3_original.exe` en la misma carpeta).
2. Extrae el contenido de `p3-esp-dlls.zip` en la **carpeta del juego** (donde
   está `Patrician3.exe`). El zip ya trae la estructura correcta:
   `p3_esp_modloader.dll` y `p3_esp_dll_patcher.exe` en la raíz, y los
   `mod_*.dll` dentro de `mods\`. Si hay algún mod que no quieras usar, borra
   su DLL.
3. Si aún no tienes `p3_esp_mods.cfg` en la carpeta del juego, cópialo ahí.
   Si ya lo tienes de una instalación anterior, déjalo como esté
   (ver [Configuración](#configuración)).
4. **Inicia** `p3_esp_dll_patcher.exe` (en la carpeta del juego).
   Se abre una ventana que dice qué ha hecho y espera una tecla para cerrarse:
   - Si hay un `Patrician3_original.exe` al lado, regenera `Patrician3.exe`
     parcheado a partir de él.
   - Si no, genera `Patrician3_modloader.exe` a partir de `Patrician3.exe`
     (el original no se toca).
5. Juega con el ejecutable parcheado (`Patrician3.exe` o
   `Patrician3_modloader.exe` según el caso). No hace falta repetir el paso 4
   salvo que cambies de ejecutable o actualices el modloader.

Los mods escriben su actividad en `Patrician3_modloader.log` (carpeta del
juego). Si algo falla al arrancar, míralo ahí primero.

> **Importante:** el parcheador solo funciona con el ejecutable español
> original del juego. Si tu `Patrician3.exe` ya venía modificado (por ejemplo,
> con el parche FullHD de la comunidad), el parcheador lo detectará y no lo
> tocará; en ese caso, recupera tu copia de seguridad
> (`Patrician3_original.exe`, paso 1) y parchea a partir de ella.

## Mods incluidos

| DLL | Qué hace |
|-----|----------|
| `mod_fullhd.dll` | Resolución 1920×1080 (antes 1280 / 1024×768). |
| `mod_limite_ciudades.dll` | La misión del gobernador permite fundar hasta 36 ciudades (antes 26). |
| `mod_mendigos_taberna.dll` | La taberna acepta hasta 100 marineros (antes 50). |
| `mod_satisfaccion_mendigos.dll` | Ajusta la satisfacción de los mendigos (4 → 3). |
| `mod_fundacion.dll` | Ciudad fundada: corrige los 3 productos más escasos (antes salían mal por un error del juego) y opcionalmente da 4 productos o una lista personalizada. Ver `[fundacion]` abajo. |
| `mod_refresco_misiones.dll` | Observa las misiones del gobernador y las registra en `misiones_log.txt`. Opción experimental de acortar plazos (ver `[misiones]`). |
| `mod_catedral_crash.dll` | Corrige el crash al abrir la catedral tras recargar partida (tabla de texturas no reinicializada). |

## Configuración

El fichero `p3_esp_mods.cfg` (en la carpeta del juego) controla los mods que
lo necesitan. Ejemplo:

```ini
[fundacion]
# Qué produce la ciudad que fundas como regidor:
#   modo=1  los 3 productos más escasos de la Hansa (juego original, corregido)
#   modo=2  igual, pero 4 productos en vez de 3
#   modo=3  TÚ eliges los productos (lista "productos" abajo)
modo=2
productos=grano,madera,cerveza,vino,miel

[misiones]
#   modo=loguear  (RECOMENDADO) solo registra en misiones_log.txt, no cambia nada.
#   modo=acortar  EXPERIMENTAL: reescribe la fecha de la misión de fundar ciudad.
modo=loguear
fundarCiudadMeses=3
```

Productos válidos para `modo=3`: grano, madera, cerveza, vino, miel, pescado,
carne, cuero, pieles, tela, sal, hierro, herramientas, lana, brea, cáñamo,
alfarería, ladrillos, aceite. Nota: carne y cuero comparten fábrica; poner los
dos cuenta como uno.

## Nota para desarrolladores

- Ramas: `main` (desarrollo), `pre-release` (compilaciones de prueba) y
  `stable` (última versión buena). Cada push a `pre-release`/`stable` compila
  en GitHub Actions y actualiza el release homónimo automáticamente.
- Compilación local en Windows: `deploy.bat` (MSVC,
  `i686-pc-windows-msvc`) genera `output\`.
- Tras tocar cualquier cave o parche, ejecutar `python3 verificar_mods.py
  Patrician3.exe`: reconstruye los bytes de cada cave, los desensambla con
  objdump y comprueba los parches contra el exe original.
- Detalles técnicos (direcciones del exe español, análisis de misiones):
  `docs/DIRECCIONES_EXE_ESPANOL.md`, `docs/analisis_mision_fundar_ciudad.md`
  y `AGENTS.md`.
