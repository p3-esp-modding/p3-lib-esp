# Análisis — Misión "fundar ciudad" (recálculo de ciudades y productos)

Análisis sobre `Patrician3.exe` (español, timestamp `0x4118A8B2`, ImageBase `0x00400000`,
`VA = 0x00400000 + file_offset` en `.text`). Contrastado con la API inglesa
(`/mnt/datos/Workspace/p3-lib/p3-api`) del mod `mod-town-hall-details`.

## TL;DR

- El cálculo de **productos** (máscara) y **ciudad candidata** se hace en
  `FUN_005343F0` y solo lo llama un sitio: la call en `0x00533D7E`.
- Esa call está dentro del **generador de misiones de almirantazgo** `FUN_00533CA0`,
  que se ejecuta **1 vez por misión generada** (check diario de la partida +
  init de partida). El resultado se **serializa en la scheduled task** y **no se
  recalcula** ni al abrir el ayuntamiento ni al reprogramar la misión.
- El mod inglés `mod-town-hall-details` **solo lee** esa máscara ya almacenada
  (`FoundTownPtr::get_production_effective_raw`, `@misión+0x08`).
- El **límite de 26 ciudades** (`cmp [world+0x10],0x1A` en `0x00533D1C`) es
  exactamente el check que parchea `mod-limite-ciudades` (doc: `0x133D1A`).

## 1. Cuándo se recalcula

### Generador: `FUN_00533CA0` (thiscall, `ECX = GameWorld`)

El `GameWorld` del exe español está en `0x00701B20` (el inglés es `0x006DE4A0`).
Campos usados (coinciden con `GameWorldPtr` inglés):

| Offset | Campo | Uso aquí |
|--------|-------|----------|
| `+0x01` | mes (`u8`) | cálculo de fecha de vencimiento |
| `+0x02` | palabra de fecha | ídem |
| `+0x10` | `towns_count` (`u16`) | **check del límite 26** |
| `+0x14` | `game_time_raw` | se copia a `world+0x0C` |
| `+0x68` | array de ciudades | escasez de productos |
| `+0x74` | array de oficinas | ídem |

Estructura interna: `world+0x68` es array de `TOWN_SIZE = 0x9F8` (confirmado en
el bucle de escasez) y `world+0x74` oficinas de `0x44C` (`OFFICE_SIZE` inglés).

El generador hace un bucle de **5 iteraciones** (contador en `[esp+0x50]`,
incrementado en `0x005341DF`, `cmp al,5`), una por tipo de misión, con jump
table en `0x005341FC`:

| Caso | Target | Misión (enum inglés `AldermanMissionType`) |
|------|--------|--------------------------------------------|
| 0 | `0x00533D1C` | **FoundTown** (fundar ciudad) |
| 1 | `0x00533DA0` | OverlandTradeRoute |
| 2 | `0x00533EFD` | NotoriousPirate |
| 3 | `0x00534059` | PirateHideout |
| 4 | `0x0053411C` | SupplyProblems |

Caso 0 (**fundar ciudad**, en `0x00533D1C`):

1. `cmp DWORD [ebp+0x10],0x1A` → `jae bail`: si `towns_count >= 26` **no genera
   la misión**. (Aquí vive el parche `limite_ciudades` 26→36.)
2. Calcula el **timestamp de vencimiento** con la fecha del world: `0x16D00`
   por año (= `TICKS_PER_YEAR` 93440) y `0x673D88[mes]` (tabla de palabras,
   días/longitud de mes). La misión queda programada ~2 años en el futuro.
3. `call 0x005343F0` en `0x00533D7E` con:
   - `ECX` = GameWorld
   - `param_2` = `&mask` (u32, en `[esp+0x54]` pre-push)
   - `param_3` = `&ciudad_elegida` (u8, en `[esp+0x58]` pre-push)
4. Post-call (`0x00533D83`): `nombre = 0x673D60[ciudad]` (tabla estática
   ciudad → id/nombre) y `0xFF` final.
5. Empaqueta una **tarea de 20 bytes** y la registra en el gestor de scheduled
   tasks `0x00702970` vía `FUN_0054C050` (el inglés usa `0x006DD73C`):
   `[esp+0x30] = { opcode=0x85 @+0, payload 16 bytes @+4 }`.

Al principio de la función (`0x00533CC2`) también llama a `FUN_0054C050` con
`{0x86}` (limpia/búsqueda de tareas previas de misión).

### Layout del payload (16 bytes) = `AldermanMissionPtr` inglés

| Offset | Contenido |
|--------|-----------|
| `+0x00` | timestamp de vencimiento (u32) |
| `+0x04` | `mission_type` (0 = FoundTown) |
| `+0x05` | `merchant_index` (inicial `0xFF`) |
| `+0x06` | `reschedule_counter` |
| `+0x08` | **`production_mask` (u32)** ← `FoundTown::get_production_effective_raw` |
| `+0x0C` | **ciudad elegida (u8)** ← `FoundTown::get_town` |
| `+0x0D` | nombre/id de la ciudad (`0x673D60[ciudad]`) |
| `+0x0E` | `0xFF` |

### Puntos de llamada del generador (cuando se dispara)

- `0x004F8CAD` — dentro del **tick diario** de la partida (bucles FP sobre
  ciudades/oficinas). Condición: `ds:0x70299C == 0` (global `0x702970+0x2C`,
  gestor de tareas: si ya hay misión pendiente no genera otra; el mod inglés
  hace el check equivalente con `get_merchant_alderman_mission_task_index !=
  -1`, función inglesa `0x004EC694`). Antes marca `[merchant+0x37] = 1`.
- `0x005E1BF5` — **init de partida / ventana del ayuntamiento** (misma zona
  `0x5E0xxx-0x5E1xxx` que engancha el mod inglés; carga string-IDs
  `0x3E81/0x3EBA/0x3EBB` e inicializa UI, luego genera sin guard).

**Consecuencia clave:** una vez generada la misión, la máscara y la ciudad
quedan fijas (se serializan en el save). El *rescheduling* solo mueve la fecha
(`reschedule_counter` @`+0x06`); **nunca vuelve a llamar a `FUN_005343F0`**
(tiene un único caller en todo el exe).

## 2. Cómo calcula productos y ciudad: `FUN_005343F0`

Decompilado completo en Ghidra (análisis previo, material de referencia).

### Escasez de productos

- Dos acumuladores de 20 dwords (20 wares): `local_50` (oferta) y `local_a0`
  (demanda, en punto fijo ×1024).
- Por cada ciudad del mapa (`world+0x10` ciudades, stride `0x9F8` desde
  `world+0x68`):
  - `oferta[i] += town[i@+0x66C] + town[i@+0x000]`
  - `demanda[i] += town[i@+0x30C] + town[i@+0x060]`
  - además, si la ciudad tiene oficinas (`u16 @town+0x784` < `0x701B28&0xffff`):
    recorre `world+0x74 + oficina*0x44C`: `oferta[i] += ofi[i+0x64]`,
    `demanda[i] += ofi[i+0x100]`.
- Por cada ware con `demanda > 0x3FF`: `ratio = oferta[i] / (demanda[i] >> 10)`.
  Inserción ordenada en un **top-4 de menor ratio** = **los 4 productos más
  escasos** (valores en `local_d8[1..4]`, índices en `local_d8[7..10]`,
  inicializados `{0xB,0,1,3}`).

### Máscara de producción (u32)

- Para los productos del top: `bit = 1 << (tabla_0x673C98[ware] - 3)` (tabla
  ware → fábrica/bit). El bit *n* de la máscara = facility `n+4`
  (`FacilityId` inglés: 4=`HuntingLodge`, 5=`FishermansHouse`, …).
- Si dos productos comparten bit/fábrica (dedup falla) → se procesa el 4º
  producto de compensación (`iVar11 = 3 → 4`): es el camino que el juego ya
  tiene nativo con `MOV EBP,4` en `0x005345C7`. Nuestro parche `fundacion_cuatro`
  (`MOV EBP,3→4` en `0x00534599`) fuerza siempre 4. **Máximo seguro = 4** (ver
  AGENTS.md).
- Caso especial ware `10` (WhaleOil): añade `0x40020` (bit 5 = `FishermansHouse`
  + bit extra; el inglés interpreta `raw & 0x20000` como "whale oil").

### Elección de la ciudad (emplazamiento)

- Itera emplazamientos de asentamiento con `FUN_00516FC0(prev)` (0 = primero,
  devuelve el siguiente; termina al volver atrás).
- Filtra por **tipo de terreno** adecuado al producto **más escaso**
  (`local_d8[7]`), tabla de tipos `0x701248` (stride 0x34):

| Wares | Tipo terreno |
|-------|--------------|
| 0 Grano, 4 Sal, 7 Vino, 0x12 Cerámica, 0x13 Ladrillo | 2 |
| 1 Carne, 0x0D Cuero, 0x0E Lana | 0 |
| 2 Pescado, 0x0A Aceite ballena, 0x10 Hierro fundido | 1 |
| 5 Miel, 0x11 Cáñamo | 3 |
| 9 Pieles, 0x0F Brea | 4 |
| resto | 5 |

- Puntúa cada emplazamiento por la **suma de distancias² a las 5 ciudades más
  cercanas** (coords en tablas `0x701238`/`0x70123C`, stride 0xD) y elige el
  **más alejado** (top-5 de menores distancias en `local_d8[1..5]`, gana el
  de suma máxima).

## 3. Aplicación práctica (mods DLL)

- **Leer la máscara cuando se calcula** (estilo "escribir a fichero"): hook de
  call rel32 en `0x00533D7E`. Tras retornar, la máscara está en el stack
  (`[esp+0x54]` pre-push → u32) y la ciudad en `[esp+0x58]` pre-push. Se
  dispara exactamente cuando el juego recalcula (1 vez por generación).
- **Forzar productos (modo 3 de `mod-fundacion`)**: el mismo hook permite
  sobrescribir la máscara en el stack antes de que se empaquete la tarea, o
  enganchar el epílogo (`0x53478A` doc.) como hace el patcher horneado.
- **Mostrar en el ayuntamiento (estilo `mod-town-hall-details`)**: la tarea se
  obtiene del gestor `0x702970`; el análogo español de la función inglesa
  `get_merchant_alderman_mission_task_index` (`0x004EC694` en el exe inglés)
  está por la zona `0x4F8Cxx` (misma región que el caller del tick diario).
  La máscara se decodifica igual que en `aldermans_office.rs` (bit n →
  facility `n+4`; `0x20000` → aceite de ballena).
- El **límite de 26** es el `cmp [world+0x10],0x1A` en `0x00533D1C` (case 0
  del generador) — coherente con la dirección del parche `limite_ciudades`
  documentada (`0x133D1A`).

## 4. Cuándo se ofrece cada misión: los retardos (CUESTIÓN ABIERTA)

Cada caso del generador calcula una **fecha = 1er día de (ahora + N meses)**
(`world+0x01` = mes, `world+0x02` word = año; `año*0x16D00 +
tabla_0x673D88[mes]*256`; la fórmula es exacta: due = 1er día de `ahora+N`):

| Misión | Instrucción | VA del inmediato | N vanilla |
|--------|-------------|------------------|-----------|
| **Fundar ciudad** | `ADD ECX,0x19` | `0x00533D33` | **25 meses** |
| Pirata notorio | `ADD ECX,0x3` | `0x00533FBB` | 3 meses |
| Esconderijo pirata | `ADD ECX,0x7` | `0x005340CE` | 7 meses |
| Suministro (caso 4) | aleatorio (`0x7D1..0xFA1`) | — | variable |
| Ruta terrestre (caso 1) | derivado de distancias | — | variable |

**La fecha calculada va en el registro de 20 bytes (tipo `0x85`) del gestor
`0x702970`** y se serializa en el save.

### Semántica de la fecha: NO cerrada estáticamente

Dos hipótesis:
- (A) es la fecha en que la misión **se ofrece** (el "refresco" tarda N meses).
- (B) es el **plazo para cumplirla** una vez aceptada (los 25/3/7 meses
  coinciden con los plazos que el juego da al aceptar, y el renderizador de
  cartas `0x4A5DEF` + handler `0x4AAE50` formatea cartas de misión fechadas:
  "Honorable..., tu entrega nos ha sido de gran ayuda...").

La evidencia disponible se inclina hacia (B), pero el dispatcher de cartas y
el de tareas no se han distinguido del todo estáticamente. **El mod
`mod-refresco-misiones` en modo `loguear` escribe `misiones_log.txt` con la
fecha, máscara y ciudad de cada misión generada**: jugando una sesión y
comparando con lo que muestra el juego (cuándo aparece la misión en la
oficina vs. la fecha límite al aceptarla) se cierra la cuestión sin riesgo.

### Lo que sí es sólido: el guard del tick diario

El generador solo corre en el tick diario (`0x004F8CAD`) si `ds:0x70299C == 0`
(sin misión de almirantazgo activa/pendiente). Ese guard — no la fecha — es el
candidato principal al "tardo en refrescar": mientras haya una misión activa,
no se generan nuevas ofertas.

### El mod y la lección stdcall

- `mod-refresco-misiones` hookea el **epílogo común `0x005341B4`** del
  generador (captura las 5 misiones): payload en `[esp+0x4C]` = fecha `+0`,
  tipo `+4`, máscara `+8`, ciudad `+0xC`. La cave reejecuta los bytes
  desplazados (`LEA ECX,[esp+0x4C]; PUSH 0x10`), llama al logger
  `p3esp_mision_log` (stdcall RET 16) y salta a `0x5341BA`.
- **Lección (v1 del mod era un no-op/crasheo latente):** `FUN_005343F0` es
  **stdcall** (`RET 8` en `0x534794`, limpia sus 2 args). NO se puede hookear
  su call-site `0x533D7E` con una cave que haga `call fun; ...; ret 8`: el
  `RET 8` de la función regresa directamente a la call-site (saltándose el
  resto de la cave) y el `ret` final de la cave salta a basura. Regla: al
  hookear la call a una stdcall, reejecutar los bytes desplazados y saltar
  detrás (o gestionar la dirección de retorno a mano).
- En modo `acortar` (EXPERIMENTAL) la cave reescribe `[esp+0x4C]` con
  `ahora + N meses` solo cuando tipo==0 (fundar ciudad). No tocar el
  inmediato `0x19` en `0x00533D33` con 1 byte salvo N≥12 (la división mágica
  exige `mes+N≥13` y `INC EDX` fuerza ≥1 año).

## Tabla de direcciones nuevas (exe español)

| VA | Qué es |
|----|--------|
| `0x00701B20` | GameWorld (inglés: `0x006DE4A0`) |
| `0x00702970` | Gestor de scheduled tasks (inglés: `0x006DD73C`) |
| `0x0070299C` | `gestor+0x2C`: misión pendiente (≠0 → no regenera) |
| `0x00533CA0` | Generador de misiones de almirantazgo (5 tipos) |
| `0x00533D1C` | Case 0 FoundTown: `cmp towns_count,26` (límite) |
| `0x00533D7E` | Única call a `FUN_005343F0` (hook ideal) |
| `0x005341B4` | Epílogo común: empaqueta tarea 20 bytes, opcode `0x85` |
| `0x005341FC` | Jump table de los 5 tipos de misión |
| `0x0054C050` | Registrar/buscar tarea en el gestor (`0x702970`) |
| `0x00516FC0` | Iterador de emplazamientos de asentamiento |
| `0x00673C98` | Tabla ware → bit de fábrica (bit = valor − 3) |
| `0x00673D60` | Tabla ciudad → id/nombre |
| `0x00673D88` | Tabla meses (palabras, fecha de vencimiento) |
| `0x00701238` / `0x70123C` | Coords X/Y de emplazamientos (stride 0xD) |
| `0x00701248` | Tipo de terreno de emplazamientos (stride 0x34) |
| `0x004F8CAD` | Caller: tick diario (guard `0x70299C == 0`) |
| `0x005E1BF5` | Caller: init partida / ventana ayuntamiento |
