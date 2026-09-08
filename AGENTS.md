# AGENTS.md — Patrician 3 Español (p3-esp)

Contexto de trabajo para agentes (OpenCode/Claude/etc.) sobre este repositorio.
Léelo antes de tocar nada.

## Qué es esto

Proyecto de modding para **Patrician 3** (ejecutable español). El objetivo es
parchear el `Patrician3.exe` (español) para corregir bugs y añadir mejoras,
**sin corromper las partidas** (problema histórico del proyecto).

## Estado actual (2026-08): una sola rama, DOS parcheadores

Hay **1 rama** (`main`) en este repo (DLLs). El static patcher está en un
repo separado: `p3-esp-static-patcher` (respaldo, ya no es el enfoque).

### Mods DLL (ENFOQUE PRINCIPAL)
- Workspace con los mods como DLLs: `p3-esp-modloader`, `p3-esp-modlib`,
  `mod-fullhd`, `mod-limite-ciudades`, `mod-mendigos-taberna`,
  `mod-satisfaccion-mendigos`, `mod-catedral-crash`, `mod-fundacion`,
  `mod-refresco-misiones`.
- `p3-esp-dll-patcher/`: añade el import de `p3_esp_modloader.dll`
  al exe para que el modloader se cargue (binario `p3_esp_dll_patcher`).
- Los mods usan **offsets absolutos + VirtualProtect** (estilo inglés), ver
  `p3-esp-modlib::apply_offset_patch`.
- Compila con `deploy.bat` (MSVC) → `output\`.

## DIAGNÓSTICO CRÍTICO — la corrupción (conclusión clave)

Después de semanas de pruebas, se determinó que **la corrupción de
elecciones/subastas NO la causaban los mods**, sino:

1. **PRINCIPAL: el mapa modificado de 32 ciudades** (mal creado). El mapa de 30
   funciona 3 años perfecto; el de 32 corrompía. El usuario borró el de 32 y
   juega con el de 30.
2. **MENOR: Wine 11** (gran rediseño). Con wine 10 + mapa 30 no corrompe.

Por tanto: **el trabajo Rust/patcher no era la causa de la corrupción.** Se
reanuda la estrategia DLL con límites seguros.

## Límites seguros determinados (IMPORTANTE — confirmado por la comunidad)

- **Marineros (`mod-mendigos-taberna`, 50→100)**: **100 es el tope natural
  del juego**. El sailor pool es un `u8` (máx 255) indexado por ciudad
  (`merchant.sailor_pools[town_index]`, offset 0xf0), pero la lógica de
  `st_update_sailor_pools` (0x004F6C10) lo capa internamente a **100**
  (`beggar_multiplier ≤ 100` y `sailor_reputation ≤ 20` →
  `(100*20)//20 = 100`). Subir la comparación de 50 a 100 coincide con ese
  tope: **no desborda nada**.
- **Ciudades (`mod-limite-ciudades`, 26→36)**: el índice de ciudad se maneja
  con `and $0xff` (máx 255) en las tablas 0x673d88/0x673d8a (palabras `,2`),
  así que **no desborda el array de índice hasta 255**. El tope del mapa es
  40 (comunidad usa `0x28`=40). El "colapso" documentado empieza **>36** por
  **no dejar 4 ciudades a los piratas** (riesgo de diseño de mapa, no de
  array). **Límite seguro recomendado = 36** (el actual, deja piratas
  cubiertos).
- **Fábricas ciudad fundada (`mod-fundacion`, `modo=2`, 3→4)**:
  **4 es el máximo intrínsecamente seguro** de `FUN_005343f0`. El bucle de
  inserción de escasez (`0x0053455F`: `MOV ECX,0x3`) mantiene **4 entradas
  nativas** (`local_d8[1..4]` valores, `local_d8[7..10]` índices) y el propio
  juego ya lee la 4ª por su camino de compensación de duplicados (`MOV EBP,0x4`
  en `0x005345C7`, cuando dos productos comparten bit/fábrica). El parche es
  1 byte: `0x00534599` `BD03000000→BD04000000` (`MOV EBP,3→4`). Con conteo 4 la
  compensación queda como no-op: el bucle **nunca** pasa de 4 iteraciones. Peor
  caso == vanilla (2 productos comparten bit → salen 3 distintos). **5+ NO**:
  leería `local_d8[11]` (scratch sin inicializar: el bucle de ciudades cercanas
  escribe ahí `-0x18 - local_a4`) o con la compensación extendida `local_d8[12]`
  (= `local_a8`, puntero real) → lectura salvaje + máscara corrupta
  **serializada a la partida guardada**. Por eso el antiguo `mod-mas-fabricas`
  (3→5) queda descartado definitivamente; el máximo soportado es 4.
- Conclusión: **los 2 límites actuales (36 ciudades y 100 marineros) son
  seguros** y no desbordan. La corrupción real venía del **mapa de 32
  ciudades mal creado**, no de subir los límites.

## Catedral y fundación (aplazados / a retomar como DLLs)

Ambos requieren runtime (no se hornean), por eso viven como DLLs en
`p3-esp-dll-patcher/`. Datos clave extraídos del análisis previo:

### mod-refresco-misiones (observador de misiones de gobernador)
- El generador de misiones de almirantazgo es `FUN_00533CA0` (ECX=GameWorld
  `0x701B20`); 5 casos con jump table en `0x5341FC` (caso 0 = fundar ciudad en
  `0x533D1C`, que contiene el check `cmp [world+0x10],0x1A` del límite de 26).
- Corre al iniciar partida (`0x5E1BF5`) y en el tick diario (`0x4F8CAD`) SOLO
  si `ds:0x70299C == 0` (sin misión de almirantazgo activa). Genera 1 misión
  por tipo; cada una lleva una fecha = 1º de (ahora + N meses): fundar 25
  (`ADD ECX,0x19` imm en `0x533D33`), pirata 3, esconderijo 7.
- **Semántica de esa fecha NO cerrada**: ¿fecha de oferta o plazo para
  cumplirla? (las cartas del juego y el UI que pinta mes+25 apuntan a plazo).
  El mod por defecto (modo=loguear) escribe `misiones_log.txt` para cerrarlo
  jugando; modo=acortar (EXPERIMENTAL) reescribe la fecha del caso fundar.
- El mod hookea el epílogo común `0x5341B4` (payload en `[esp+0x4C]`: fecha
  `+0`, tipo `+4`, máscara `+8`, ciudad `+0xC`). Cave llama al logger
  `p3esp_mision_log` (stdcall, RET 16) y salta a `0x5341BA`.
- **Lección stdcall**: `FUN_005343F0` es stdcall (`RET 8` en `0x534794`).
  Hookear su call-site `0x533D7E` con "call fun; ...; ret 8" NO funciona: el
  ret 8 de la fun regresa directo a la call-site y el ret de la cave salta a
  basura (la v1 del mod era un no-op/crasheo latente). Al hookear la call a
  una stdcall: reejecutar los bytes desplazados y saltar detrás.
- Análisis completo: `analisis_mision_fundar_ciudad.md`.

### mod-catedral-crash
- Bug: crash al abrir la catedral/interior. EIP `0x00491CBD`
  (`mov [edx+ecx*4], eax`) con EDX=ECX=0 → escritura en NULL, porque
  `*(ESI+4) == 0` (tabla interna de texturas no inicializada).
- Funciones implicadas: FUN_00491930 (0x00491930), FUN_00491d50,
  FUN_00491ec0, FUN_00491f30. 18 llamadas, principalmente desde 0x005ceb64.
- Intentos descartados: shellcode, VEH saltando al epílogo, parchear free(),
  tabla dummy via VirtualAlloc (causó crash en ntdll por ciclo de vida).
- Solución adoptada en el mod (v2): SOLO el cargador FUN_00491930 repara el
  estado cuando `[ESI+4]==NULL` (tabla nueva con el alloc del juego 0x00651019,
  `[0x38]=slot pedido`, contadores a 0, `[0x3a]=1`; free del juego 0x00651042).
  Los lectores FUN_00491d50/FUN_00491ec0 salen sin dibujar si la tabla es NULL
  (antes inicializaban tabla y reseteaban contadores por detrás del cargador →
  cura pintado antes de tiempo y repetido por pantalla). FUN_00491f30 conserva
  su guard (salta a 0x491F77, que ya realoca/recarga él solo).
- Layout del objeto interior (maquina de estados): `[obj+0x04]` tabla de
  texturas, `[obj+0x1f]` frame de fundido (lo que dibujan los lectores:
  `tabla[[0x1f]]`), `[obj+0x20]` texturas cargadas (donde escribe el cargador:
  `tabla[[0x20]]`), `[obj+0x38]` slot activo (6/7 u 8/9 segun nivel del
  edificio), prioridades en `[obj+0xc+i]`. El llamador per-frame esta en
  `0x5CABxx` (objeto global `ds:0x6F3EDC`); hay otra serie de llamadas en
  `0x5CEBxx-0x5CF0xx` (taberna) y destructores en `0x5D07xx-0x5D0Dxx`.
- Nota: el usuario considera la catedral de **baja prioridad** (a él solo le
  afecta al cargar múltiples partidas sin salir al menú; los demás no).
  Se puede descartar si complica.

### mod-fundacion (ciudad fundada)
- `FUN_005343f0` calcula cuántas ciudades se pueden fundar y qué
  productos/fábricas tendrá la ciudad nueva (los más escasos de la Hansa).
- Bug corregido (`mod-fundacion`): los 3 productos más escasos salían mal por
  un off-by-one (`sub ecx,3` debería ser `sub ecx,4`, fix `670083E903→04`).
- **Opción `fabricas=4`** (config `[fundacion]`, solo con `forzarProductos=0`):
  1 byte en `0x00534599` (`BD03000000→BD04000000`, `MOV EBP,3→4`) → 4
  productos/fábricas. **Seguro y máximo** (ver "Límites seguros").
- **Config `[fundacion]` = clave unica `modo`** (1/2/3, con alias de texto;
  default 1): `modo=1` vainilla+fix (3 productos correctos), `modo=2` 4+fix
  (añade el parche de `0x00534599`), `modo=3` personalizado (hook con la lista
  `productos=`). Ya no existen `forzarProductos` ni `fabricas`. El `.cfg` del
  usuario es deliberadamente escueto (solo modos y lista de productos); TODO
  el detalle tecnico vive aqui y en `DIRECCIONES_EXE_ESPANOL.md`, no en el cfg.
- Límites: **4 fábricas es el techo** (parche 3→5 descartado: leería
  `local_d8[11]`/`[12]` fuera del array de 12 elementos si hay productos que
  comparten fábrica → corrupción serializada). Forzar productos concretos =
  sección `.cod` / hook del epílogo que escribe la máscara.
- Mapeo producto→bit (máscara u32) y producto→fábrica: ver wares/buildings/
  facilities de la comunidad (https://p3modding.github.io/) o el enum
  `FacilityId` en `p3-api/src/data/enums.rs` del repo inglés.

## Ejecutable español — datos de referencia

- `Patrician3_original.exe` (en la raíz del repo y en el juego): timestamp PE
  `0x4118A8B2`, ImageBase `0x00400000`, machine `0x014C`, no-ASLR.
- En `.text`, `RVA == file offset` (secciones alineadas a 0x1000). Por tanto
  `VA = 0x00400000 + fileoffset` para código.
- Direcciones de parche documentadas en **`DIRECCIONES_EXE_ESPANOL.md`**
  (¡NO perder!): fullhd (0x506BB, 0x50BDC, 0x4A78A, 0x53D71, 0x599A8,
  0x86A02, 0x8AA16), limite (0x133D1A), mendigos (0x1D62C0), satisfaccion
  (0x034E15), fundacion productos (0x5345AF), epilogo FUN_005343f0 (0x53478A).
- El juego **estándar tiene 26 ciudades**; los mapas modificados pueden tener
  30-40.

## Archivos clave

- `p3-esp-dll-patcher/` — patcher del modloader (añade el import).
- `p3-esp-modlib/src/lib.rs` — `apply_offset_patch` (VA
  absoluto + VirtualProtect), `apply_patches` (find/replace).
- `verificar_mods.py` — verificación de TODOS los mods: code caves
  (reconstruye los bytes y los desensambla con objdump, comprobando cada
  salto y que el buffer cubre el último rel32) + parches de bytes contra
  `Patrician3_original.exe` (expected/find de los mods DLL, static patcher,
  modloader/dll-patcher). Uso: `python3 verificar_mods.py`.
- `DIRECCIONES_EXE_ESPANOL.md` — registro de direcciones del exe español.
- `analisis_mision_fundar_ciudad.md` — análisis del generador de misiones
  de gobernador (FUN_00533CA0), semántica de fechas, y mod refresco.
- `Patrician3_fullhd.exe` — exe de la comunidad con fullhd horneado.

## Referencia de la comunidad inglesa

- Repo del modloader/mods inglés: https://github.com/P3Modding/p3-lib (Rust).
  Modloader inglés: `p3-modloader`, usa `hooklet`, `win_dbg_logger`,
  `windows 0.48`. Los mods ingleses usan offsets absolutos + VirtualProtect.
  (Si se clona localmente, hacerlo a `/tmp` o una ruta temporal, no dentro de
  este repo.)
- Docs: https://p3modding.github.io/ (ware types, buildings, facilities,
  known bugs, patches). URLs útiles:
  - Sailor pools: https://p3modding.github.io/ch05-04-sailor-pools.html
  - Update sailor pools: https://p3modding.github.io/scheduled-tasks/0026-update-sailor-pools.html
  - Fund settlement limit: https://p3modding.github.io/patches/increase-alderman-found-settlement-limit.html
  - New settlement ware production: https://p3modding.github.io/bugs/new-settlement-ware-production.html
  - Wares/buildings/facilities: .../basics/wares.html, .../basics/buildings.html, .../basics/facilities.html

## Instrucciones de compilación

- **DLLs**: `deploy.bat` (MSVC, compila cada crate con
  su propio `--manifest-path`) → `output\`.
  GitHub Actions: push a `main` con cambios en `**/*.rs` o `**/Cargo.toml` → artifact descargable.

## Gotchas / lecciones aprendidas

- **No cargar DLLs pesadas ni modloader en el proceso** si se busca evitar
  corrupción: desplazan el heap del juego de 2003 (aunque el diagnóstico final
  mostró que la causa real era el mapa, no el mecanismo).
- Los mods DLL deben usar **offsets absolutos + VirtualProtect**
  (estilo inglés), NO find/replace sobre toda la imagen.
- **`cargo check` NO valida el código máquina de los code caves** (solo
  sintaxis/tipos de Rust). Un off-by-one en un rel32 o un `CAVE_SIZE` corto
  compila y crashea en runtime. **Siempre verificar los bytes de cada cave
  desensamblándolos** (p. ej. reconstruir los bytes en Python y `objdump -D -b
  binary -m i386`), comprobando que cada salto cae donde debe y que el tamaño
  del buffer cubre el último rel32. (Bug real: cave de FUN_00491ec0 con
  `CAVE_SIZE=31` cuando necesita 32; y rel32 del JMP escrito en `0x1B` pisando
  el `E9` → crash al entrar en edificios.) **Usar `verificar_mods.py`
  tras tocar cualquier cave o parche.**
- `deploy.bat` y scripts deben estar en **CRLF** para que funcionen en cmd de
  Windows (usar `sed -i 's/\r$//' && sed -i 's/$/\r/'` tras editar).
- WinSCP puede no copiar bien el Cargo.toml raíz anfitrión→VM; usar
  `--manifest-path` explícito o compilar cada crate con su propio manifest.