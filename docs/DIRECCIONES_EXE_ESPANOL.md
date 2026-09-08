# Direcciones del EXE español (Patrician3_original.exe)

Este documento guarda las direcciones del **ejecutable español** de Patrician 3
(timestamp PE `0x4118A8B2`, ImageBase `0x00400000`), para que no se pierdan.

Las direcciones de los mods ingleses pertenecen al exe **inglés** (binario
distinto) y no son válidas aquí. En `.text`, `RVA == file offset` porque las
secciones están alineadas a 0x1000.

## Estado actual

El enfoque principal son las DLLs. El patcher estático
está en un repo separado (`p3-esp-static-patcher`, respaldo).

Los mods de **catedral** y **fundación** se retoman como DLLs (en
`p3-esp-dll-patcher/`). El límite de ciudades seguro es **36** y el de marineros
**100** (los que ya usa la comunidad inglesa sin corromper).


## VAs de parche (runtime, no-ASLR: VA = 0x00400000 + RVA)

### mod-fullhd (resolución)
| RVA/FileOff | VA | bytes originales | bytes parcheados | nota |
|---|---|---|---|---|
| 0x0506BB | 0x004506BB | `3d 00 05 00 00 0f 85` | `3d 80 07 00 00 0f 85` | cmp width 1280->1920 [1] |
| 0x050BDC | 0x00450BDC | `3d 00 05 00 00 75 45` | `3d 80 07 00 00 75 45` | cmp width 1280->1920 [2] |
| 0x04A78A | 0x0044A78A | `c7 44 24 4c 00 04 00 00 c7 44 24 50 00 03 00 00` | `c7 44 24 4c 80 07 00 00 c7 44 24 50 38 04 00 00` | 1024x768->1920x1080 |
| 0x053D71 | 0x00453D71 | `c7 44 24 18 00 04 00 00 c7 44 24 1c 00 03 00 00` | `c7 44 24 18 80 07 00 00 c7 44 24 1c 38 04 00 00` | 1024x768->1920x1080 |
| 0x0599A8 | 0x004599A8 | `c7 44 24 3c 00 04 00 00 c7 44 24 40 00 03 00 00` | `c7 44 24 3c 80 07 00 00 c7 44 24 40 38 04 00 00` | 1024x768->1920x1080 |
| 0x086A02 | 0x00486A02 | `c7 44 24 48 00 04 00 00 c7 44 24 4c 00 03 00 00` | `c7 44 24 48 80 07 00 00 c7 44 24 4c 38 04 00 00` | 1024x768->1920x1080 |
| 0x08AA16 | 0x0048AA16 | `c7 44 24 24 00 04 00 00 c7 44 24 28 00 03 00 00` | `c7 44 24 24 80 07 00 00 c7 44 24 28 38 04 00 00` | 1024x768->1920x1080 |

NOTA: el JSON original (p3-esp-patch) añade 2 parches de scroll en
0x005EC32B y 0x005EC33D (mov 100->250). Se quit posteriores porque no
estaban en el fullhd de la comunidad; el fullhd inglés (mod-high-res)
usa detours, no estos bytes. VERIFICAR si hacen falta.

### mod-limite-ciudades (26 -> 36)
| RVA/FileOff | VA | bytes | nota |
|---|---|---|---|
| 0x133D1A | 0x00533D1A | `53 00 83 7d 10 1a` -> `53 00 83 7d 10 24` | cmp [ebp+0x10] 26->36 |
| (valor en +5) | 0x00533D1F | byte 0x1a -> 0x24 | |

### mod-mendigos-taberna (50 -> 100)
| RVA/FileOff | VA | bytes | nota |
|---|---|---|---|
| 0x1D62C0 | 0x005D62C0 | `ff 00 00 00 83 f8 32 89` -> `ff 00 00 00 83 f8 64 89` | cmp eax 50->100 |

### mod-satisfaccion-mendigos (4 -> 3)
| RVA/FileOff | VA | bytes | nota |
|---|---|---|---|
| 0x034E15 | 0x00434E15 | `b9 32 00 00 00 ba 04 00 00 00 89 50 fc 89 18` -> `... ba 03 00 ...` | 4->3 |

### mod-fundacion (sub 3 -> sub 4 de producto)
| RVA/FileOff | VA | bytes | nota |
|---|---|---|---|
| 0x5345B3 | 0x005345B3 | `83 e9 03` -> `83 e9 04` (con prefijo `67 00`) | sub ecx,3->4 |
| 0x134599 | 0x00534599 | `bd 03 00 00 00` -> `bd 04 00 00 00` | MOV EBP,3->4: 4a fabrica (`modo=2`) |

Nota de la 4a fabrica: el bucle de insercion de escasez (0x0053455F,
`MOV ECX,0x3`) mantiene 4 entradas nativamente (`local_d8[1..4]` valores y
`local_d8[7..10]` indices) y la compensacion de duplicados (`MOV EBP,0x4` en
0x005345C7) ya lee la 4a cuando dos productos comparten fabrica. Por eso 4 es
seguro y **el maximo**: 5 leeria `local_d8[11]` (scratch sin inicializar) o
`local_d8[12]` (= `local_a8`, puntero) -> corrupcion serializada a la partida.

### mod-catedral-crash (caves 0x491930-0x491f30)
Funciones: FUN_00491930, FUN_00491d50, FUN_00491ec0, FUN_00491f30.
- 0x0049194E init tabla; 0x00491D5D guard; 0x00491ECB guard; 0x00491F5E guard.
- alloc del juego: 0x00651019; free: 0x00651042.

Causa del crash (documentada): EIP 0x00491CBD (`mov [edx+ecx*4], eax`) con
EDX=ECX=0 → escritura en NULL porque `*(ESI+4)==0` (tabla de texturas no
inicializada al cargar múltiples partidas sin salir al menú). Fix en los code
caves: cuando `[ESI+4]==NULL` se inicializa la tabla con el **alloc del juego**
(0x00651019), no con HeapAlloc (el free del juego 0x00651042 gestiona su vida;
un buffer ajeno desborda el heap → crash en ntdll). Baja prioridad para el
usuario (solo le afecta a él); descartable si complica.

## Límites seguros (confirmado)

- **Marineros 100**: el sailor pool es `u8` (offset 0xf0 del merchant, por
  ciudad) y `st_update_sailor_pools` (0x004F6C10) lo capa internamente a 100
  (`(beggar_multiplier≤100 * sailor_reputation≤20)//20 = 100`). Subir 50→100
  no desborda (100 = tope natural).
- **Ciudades 36**: el índice de ciudad usa `and $0xff` (máx 255) en las tablas
  0x673d88/0x673d8a → no desborda el array hasta 255. El "colapso" >36 es por
  no dejar 4 ciudades a los piratas (diseño de mapa, no de array).
- **Fabricas de la ciudad fundada 4**: `FUN_005343f0` mantiene un top-4 de
  escasez en `local_d8[1..4]`/`[7..10]` y su propia compensacion de duplicados
  lee la 4a entrada (`MOV EBP,0x4` en 0x005345C7). Parche `MOV EBP,3->4` en
  0x00534599 (`modo=2` del mod-fundacion) = maximo seguro. 5+ desborda los
  12 elementos del array.

## Offsets modloader (exe español)
- WINMAIN_CALL_RVA = 0x0023E082 (byte a parchear: `e8 b9 f4 00 00` -> hook)
- WINMAIN_VA = 0x0064D540

## Fuente
- Repo inglés: https://github.com/P3Modding/p3-lib (usa offsets ABSOLUTOS, no
  find/replace; si se clona, hacerlo en /tmp, no en este repo)
- Docs comunidad: https://p3modding.github.io/
- Exe original: Patrician3_original.exe (timestamp 0x4118A8B2)
