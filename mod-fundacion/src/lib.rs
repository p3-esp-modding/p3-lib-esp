// mod-fundacion
//
// Unifica los antiguos mod-fundacion, mod-mas-fabricas y
// mod-productos-fundacion en un solo mod. La config [fundacion] en
// p3_esp_mods.cfg tiene UNA sola clave de modo, "modo", con 3 valores:
//
//   modo=3 / personalizado -> hook del epilogo de FUN_005343f0: al terminar
//     la funcion se sobreescribe en *param_2 la mascara de productos del
//     fichero (los que sean: 2, 5, 7...). No se aplica ningun parche
//     byte-a-byte (las correcciones de escasez serian anuladas por la
//     mascara forzada).
//
//   modo=1 / vainilla (DEFAULT) -> SOLO el parche de mod-productos-fundacion:
//     "670083E903" -> "670083E904" (SUB ECX,0x3 -> 0x4) en el calculo de
//     los productos mas escasos. Resultado: 3 productos como el juego
//     original, pero los correctos.
//
//   modo=2 / cuatro -> el fix anterior MAS el parche de 4a fabrica:
//     "BD03000000" -> "BD04000000" (MOV EBP,0x3 -> 0x4) en 0x00534599, el
//     conteo del bucle que arma la mascara. Resultado: 4 productos/
//     fabricas correctos. 4 es el MAXIMO ABSOLUTO de esta funcion:
//     - El bucle de insercion de escasez (0x0053455F: MOV ECX,0x3 +
//       DEC/CMP/JG) mantiene NATIVAMENTE 4 entradas: valores en
//       local_d8[1..4] e indices de producto en local_d8[7..10]. La 4a
//       ya se calcula siempre; el vanilla la descarta.
//     - El propio juego ya lee la 4a entrada por un camino nativo: la
//       compensacion de duplicados (0x005345C7: MOV EBP,0x4) extiende el
//       bucle a 4 cuando dos productos comparten bit/fabrica. Con el
//       conteo en 4, esa compensacion queda como no-op: el bucle NUNCA
//       puede pasar de 4 iteraciones.
//     - Peor caso == vanilla: si 2 de los 4 comparten bit (carne/cuero),
//       salen 3 productos distintos, exactamente lo que da el juego.
//   POR QUE NO 5 O MAS: el bucle leeria local_d8[11], que NO es una
//   entrada valida del top-N sino scratch sin inicializar (el bucle de
//   ciudades cercanas escribe ahi la longitud de lista: local_d8[0xb] =
//   -0x18 - local_a4). Un indice de producto basura -> MOV CL,[basura +
//   0x673c98] = lectura salvaje + bit basura en la mascara, que se
//   serializa a la partida guardada. Y con el viejo parche de
//   compensacion (MOV EBP,0x4 -> INC EBP) el bucle llegaba a leer
//   local_d8[12], que es local_a8 (puntero real a la lista de ciudades)
//   -> corrupcion garantizada si habia productos que compartian fabrica.
//   Por eso mod-mas-fabricas (3->5, 4->+1, iVar5 3->4) queda ELIMINADO:
//   solo hay 3 modos: 3 corregido, 4 corregido o lista personalizada.
//
// El hook del epilogo (0x0053478A) es un code cave en asm puro (23
// bytes) con la mascara incrustada: hace los pops del epilogo, el
// add esp,0xe0, y escribe la mascara en *param_2.
//
// ===========================================================================
// OTRAS CLAVES de [fundacion] (absorbidas de mod-refresco-misiones)
// ===========================================================================
//   ciudad = lista ordenada (hasta 8, separadas por coma) de ciudades
//     preferidas para la mision de fundar, p. ej. "memel,windau,konigsberg".
//     Se usa la PRIMERA no fundada (cascada replica FUN_00516FC0); si
//     ninguna esta disponible, la que calcula el juego. Por defecto: ninguna.
//     Se lee al arrancar; un hook en el epilogo del generador (0x005341B4,
//     cave de 21 bytes) la aplica justo antes de empaquetar la mision.
//   refresco = auto | defecto (DEFAULT). Con "auto", si cambias cualquier
//     clave de [fundacion] con el juego abierto, un hook en el tick diario
//     (0x004F8C9F, cave de 19 bytes) detecta el cambio (firma en
//     p3_esp_firma.txt), resetea el guard 0x70299C y EN ESE MISMO tick el
//     generador recalcula la mision con la config nueva (sin jugar meses).
//     "defecto": el juego gestiona el ciclo de la mision como siempre.
//     refresco se lee al arrancar; los cambios de modo/productos/ciudad
//     con "auto" se aplican sin reiniciar.
//
// Los errores se escriben en Patrician3_modloader.log (via p3-esp-modlib).

#![allow(non_snake_case, non_camel_case_types)]

use p3_esp_modlib::{apply_offset_patch, hex, log_error, log_info};
use std::fs;
use std::sync::{Mutex, Once};
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, PAGE_READWRITE,
};

const IMAGE_BASE: u32 = 0x00400000;
const HOOK_RVA: u32 = 0x0013478A; // 0x0053478A: POP EDI (epilogo FUN_005343f0)
const CAVE_SIZE: usize = 23;

unsafe fn force_hook_target_matches() -> bool {
    let expected = [0x5F, 0x5E, 0x5D, 0x5B, 0x81];
    std::slice::from_raw_parts((IMAGE_BASE + HOOK_RVA) as *const u8, expected.len()) == expected
}

unsafe fn mi_hook_target_matches() -> bool {
    std::slice::from_raw_parts((IMAGE_BASE + MI_HOOK_RVA) as *const u8, 5) == MI_HOOK_EXPECTED
}

unsafe fn guard_hook_target_matches() -> bool {
    std::slice::from_raw_parts((IMAGE_BASE + GUARD_HOOK_RVA) as *const u8, 5) == GUARD_HOOK_EXPECTED
}

// ---------- config ----------

fn read_cfg_value(section: &str, key: &str) -> Option<String> {
    let content = fs::read_to_string("p3_esp_mods.cfg").ok()?;
    let mut in_section = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line[1..line.len() - 1].trim().eq_ignore_ascii_case(section);
            continue;
        }
        if in_section {
            if let Some(eq) = line.find('=') {
                let k = line[..eq].trim();
                let v = line[eq + 1..].trim();
                if k.eq_ignore_ascii_case(key) {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Modos de [fundacion] para la clave "modo".
const MODO_VAINILLA: u8 = 1; // fix productos, 3 fabricas correctas (DEFAULT)
const MODO_CUATRO: u8 = 2; // fix + parche MOV EBP,4 -> 4 fabricas correctas
const MODO_PERSONALIZADO: u8 = 3; // hook con la lista "productos"

fn cfg_modo() -> u8 {
    match read_cfg_value("fundacion", "modo") {
        Some(v) => {
            let s = v.trim().to_ascii_lowercase();
            let n = match s.parse::<u8>() {
                Ok(n @ 1..=3) => Some(n),
                Ok(n) => {
                    log_error(&format!(
                        "mod-fundacion: modo={} no existe (validos: 1=vainilla+fix, \
                         2=4+fix, 3=personalizado); se aplica 1. 5 o mas NO es \
                         seguro: FUN_005343f0 leeria local_d8[11], scratch sin \
                         inicializar -> corrupcion serializada a la partida",
                        n
                    ));
                    None
                }
                Err(_) => match s.as_str() {
                    "vainilla" | "vanilla" | "fijar" => Some(MODO_VAINILLA),
                    "cuatro" | "fabricas4" | "4fabricas" => Some(MODO_CUATRO),
                    "personalizado" | "forzar" | "lista" => Some(MODO_PERSONALIZADO),
                    _ => None,
                },
            };
            n.unwrap_or_else(|| {
                if s.parse::<u8>().is_err() {
                    log_error(&format!(
                        "mod-fundacion: modo=\"{}\" no reconocido (validos: 1, 2, 3); \
                         se aplica 1",
                        v.trim()
                    ));
                }
                MODO_VAINILLA
            })
        }
        None => MODO_VAINILLA,
    }
}

fn parse_products(s: &str) -> u32 {
    let mut mask = 0u32;
    for name in s.split(',') {
        let n = name.trim().to_ascii_lowercase();
        let bit = match n.as_str() {
            "grano" => 0x20,
            "madera" => 0x80,
            "cerveza" => 0x04,
            "vino" => 0x1000,
            "miel" => 0x10,
            "pescado" => 0x02,
            "carne" => 0x40,
            "cuero" => 0x40,
            "cueros" | "pieles" => 0x01,
            "tela" => 0x100,
            "sal" => 0x200,
            "hierro" => 0x400,
            "herramientas" => 0x08,
            "lana" => 0x800,
            "brea" => 0x8000,
            "canamo" | "cañamo" | "cáñamo" => 0x10000,
            "alfareria" | "alfarería" => 0x2000,
            "ladrillos" => 0x4000,
            "aceite" => 0x20002,
            _ => 0,
        };
        mask |= bit;
    }
    mask
}

fn compute_forced_mask() -> u32 {
    match read_cfg_value("fundacion", "productos") {
        Some(list) => {
            let mask = parse_products(&list);
            if mask != 0 {
                mask
            } else {
                0x10B4 // default: grano, madera, cerveza, vino, miel
            }
        }
        None => 0x10B4,
    }
}

// ---------- ciudad preferida + refresco auto (absorbidos de mod-refresco-misiones) ----------

/// 0x005341B4: epilogo comun del generador de misiones FUN_00533CA0.
/// Bytes originales: 8D 4C 24 4C 6A (LEA ECX,[esp+0x4C] + PUSH 0x10).
const MI_HOOK_RVA: u32 = 0x001341B4;
const MI_HOOK_EXPECTED: [u8; 5] = [0x8D, 0x4C, 0x24, 0x4C, 0x6A];
/// Continuacion tras los 2 bytes desplazados (el PUSH 0x10 acaba en 0x5341BA).
const MI_HOOK_CONT: u32 = 0x005341BA;
/// Cave: lea eax,[esp+0x58] (4) + push eax (1) + call (5)
/// + LEA ECX,[esp+0x4C] (4) + PUSH 0x10 (2) + jmp (5) = 21 bytes.
const MI_CAVE_SIZE: usize = 21;

/// 0x004F8C9F: `mov eax,[0x70299C]` en el tick diario, justo ANTES del test
/// que decide si el generador corre (0x4F8CA4 test eax,eax). Bytes:
/// A1 9C 29 70 00. Verificado: ningun salto aterriza en 0x4F8C9F..0x4F8CA4.
const GUARD_HOOK_RVA: u32 = 0x000F8C9F;
const GUARD_HOOK_EXPECTED: [u8; 5] = [0xA1, 0x9C, 0x29, 0x70, 0x00];
const GUARD_HOOK_CONT: u32 = 0x004F8CA4;
/// Cave: pushfd/pushad/call helper/popad/popfd/mov eax,[guard]/jmp = 19 bytes.
const GUARD_CAVE_SIZE: usize = 19;

/// world+0x10 = towns_count (u32), world+0x18 = ids de ciudades fundadas (u8),
/// tabla ciudad->id de la mision (byte [ciudad] en 0x673D60).
const WORLD_TOWNS_COUNT_ADDR: u32 = 0x00701B30;
const WORLD_TOWN_IDS_ADDR: u32 = 0x00701B38;
const TOWN_ID_TABLE_ADDR: u32 = 0x00673D60;
const MAX_SITES: u8 = 0x28; // 40 emplazamientos (cmp de FUN_00516FC0)
/// Guard del tick: 0 = sin mision pendiente => el generador puede correr.
const GUARD_ADDR: usize = 0x70299C;

/// [fundacion] ciudad = lista ordenada de hasta 8, probada en cascada
/// (primera no fundada gana). 0xFF = fin de lista.
static PREFS: Mutex<[u8; 8]> = Mutex::new([0xFF; 8]);
static AVISA_PREF: Once = Once::new();

/// Nombre de ciudad por indice (mapeo de la UI de fundar, comunidad inglesa).
const CIUDADES: [&str; 40] = [
    "Edimburgo", "Newcastle", "Scarborough", "Boston", "Londres", "Brujas", // 0-5
    "Haarlem", "Harlingen", "Groninga", "Colonia", "Bremen", "Ribe", // 6-11
    "Hamburgo", "Flensburgo", "Lubeck", "Rostock", // 12-15
    "Bergen", "Stavanger", "Tonsberg", "Oslo", "Aalborg", "Gotemburgo", // 16-21
    "Naestved", "Malmo", "Ahus", "Estocolmo", "Visby", "Helsinki", // 22-27
    "Stettin", "Ruegenwald", "Gdansk", "Torun", "Konigsberg", "Memel", // 28-33
    "Windau", "Riga", "Pernau", "Reval", "Ladoga", "Novgorod", // 34-39
];

fn nombre_ciudad(idx: u8) -> &'static str {
    CIUDADES.get(idx as usize).copied().unwrap_or("?")
}

/// [fundacion] ciudad = una o varias separadas por coma, en orden de
/// preferencia (p. ej. "memel,windau,konigsberg"). Valores invalidos se
/// saltan con error en el log. Lista vacia = sin preferencia.
fn cfg_ciudades_pref() -> Vec<u8> {
    let v = match read_cfg_value("fundacion", "ciudad") {
        Some(v) => v,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for parte in v.split(',') {
        let s = parte.trim();
        if s.is_empty() {
            continue;
        }
        if let Ok(n) = s.parse::<u16>() {
            if n < MAX_SITES as u16 {
                out.push(n as u8);
            } else {
                log_error(&format!(
                    "mod-fundacion: ciudad={} fuera de rango (0..39); se salta",
                    n
                ));
            }
            continue;
        }
        let l = s.to_ascii_lowercase();
        match CIUDADES.iter().position(|c| c.to_ascii_lowercase() == l) {
            Some(i) => out.push(i as u8),
            None => log_error(&format!(
                "mod-fundacion: ciudad=\"{}\" no reconocida; se salta",
                s
            )),
        }
    }
    out.truncate(8);
    out
}

fn guardar_prefs(prefs: &[u8]) {
    if let Ok(mut arr) = PREFS.lock() {
        for (i, p) in prefs.iter().enumerate() {
            arr[i] = *p;
        }
    }
}

/// [fundacion] refresco = auto (activa el recalculo al cambiar el cfg).
/// Cualquier otra cosa (o la clave ausente) = defecto (el juego gestiona).
fn cfg_refresco_auto() -> bool {
    let v = match read_cfg_value("fundacion", "refresco") {
        Some(v) => v,
        None => return false,
    };
    let s = v.trim().to_ascii_lowercase();
    match s.as_str() {
        "auto" | "si" | "recalculo" => true,
        "defecto" | "pordefecto" | "off" | "no" | "" => false,
        _ => {
            log_error(&format!(
                "mod-fundacion: refresco=\"{}\" no reconocido (auto|defecto); \
                 se usa defecto",
                v.trim()
            ));
            false
        }
    }
}

/// Ciudades ocupadas: replica el filtro de FUN_00516FC0 (ids de la tabla
/// world+0x18 y ids de las ciudades con oficina de los 5 merchants).
fn ciudad_ya_fundada(id: u8) -> bool {
    unsafe {
        let count = *(WORLD_TOWNS_COUNT_ADDR as *const u32) as usize;
        let ids = WORLD_TOWN_IDS_ADDR as *const u8;
        for i in 0..count.min(64) {
            if *ids.add(i) == id {
                return true;
            }
        }
        for k in 0..5u32 {
            let p = *((0x00700E34 + k * 4) as *const u32);
            if p >= 0x10000 {
                let mid = *((p + 0xE) as *const u8);
                if mid == id {
                    return true;
                }
            }
        }
        false
    }
}

/// Llamada desde la cave del epilogo con puntero a [esp+0x58] (ciudad + byte
/// de id). Aplica la PRIMERA ciudad preferida no fundada (cascada); si ninguna
/// esta disponible deja la que calculo el juego (preferencia, nunca fuerza).
#[no_mangle]
pub unsafe extern "stdcall" fn p3esp_pref_ciudad(p_town: *mut u8) {
    let prefs = match PREFS.lock() {
        Ok(g) => *g,
        Err(_) => return,
    };
    let actual = *p_town;
    let mut alguna_fundada = false;
    for &pref in prefs.iter() {
        if pref == 0xFF || pref >= MAX_SITES {
            break;
        }
        if pref == actual {
            return; // ya se ofrece la preferida: nada que hacer
        }
        if ciudad_ya_fundada(pref) {
            alguna_fundada = true;
            continue; // probamos la siguiente de la cascada
        }
        *p_town = pref;
        // byte de id en +1, igual que hace 0x00533D91 (tabla 0x673D60[ciudad]).
        *p_town.add(1) = *((TOWN_ID_TABLE_ADDR + pref as u32) as *const u8);
        return;
    }
    if alguna_fundada {
        AVISA_PREF.call_once(|| {
            log_info(
                "mod-fundacion: ninguna ciudad preferida disponible \
                 (ya fundadas); se usa la que calcula el juego",
            );
        });
    }
}

/// Firma de la config relevante para las misiones (para refresco=auto).
fn cfg_firma() -> String {
    let modo = read_cfg_value("fundacion", "modo").unwrap_or_default();
    let prods = read_cfg_value("fundacion", "productos").unwrap_or_default();
    let ciudad = read_cfg_value("fundacion", "ciudad").unwrap_or_default();
    let refresco = read_cfg_value("fundacion", "refresco").unwrap_or_default();
    format!(
        "{}|{}|{}|{}",
        modo.trim(),
        prods.trim(),
        ciudad.trim(),
        refresco.trim()
    )
}

/// Firma de la ultima config vista (por refresco=auto).
const FIRMA_FILE: &str = "p3_esp_firma.txt";

fn firma_guardada() -> String {
    fs::read_to_string(FIRMA_FILE).unwrap_or_default()
}

/// Llamada desde la cave del tick diario (refresco=auto), ANTES del test del
/// guard. Si el cfg cambio respecto a la ultima firma guardada: guarda la
/// firma, refresca la lista de ciudades y resetea el guard 0x70299C (0 =
/// sin mision pendiente). La cave relee el guard y EN ESE MISMO tick el
/// generador recalcula con la config nueva. Limitado a 1 lectura/segundo
/// real (el tick corre 256 veces por dia de juego).
#[no_mangle]
pub unsafe extern "C" fn p3esp_tick_guard() {
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    static ULT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
    let ult = ULT.load(std::sync::atomic::Ordering::Relaxed);
    if ahora.saturating_sub(ult) < 1 {
        return;
    }
    if ULT
        .compare_exchange(ult, ahora, std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    let firma = cfg_firma();
    if firma == firma_guardada() {
        return;
    }
    // Cambio detectado (o primera vez): refrescar prefs, resetear el guard
    // y guardar la firma.
    guardar_prefs(&cfg_ciudades_pref());
    *(GUARD_ADDR as *mut u32) = 0;
    let _ = fs::write(FIRMA_FILE, &firma);
    log_info(&format!(
        "mod-fundacion: config cambiada ({}); guard 0x70299C reseteado, \
         el generador recalcula la mision",
        firma
    ));
}

// ---------- hook epilogo (asm puro) ----------

unsafe fn apply_force_hook(mask: u32) -> bool {
    if !force_hook_target_matches() {
        log_error("mod-fundacion: version o bytes del EXE no compatibles");
        return false;
    }
    let cave = VirtualAlloc(None, CAVE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-fundacion: VirtualAlloc devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let cave_ptr = cave as *mut u8;

    // pop edi; pop esi; pop ebp; pop ebx; add esp,0xe0
    // mov eax,[esp+4] (param_2); mov dword [eax], mask; ret 8
    let mut code = [0u8; CAVE_SIZE];
    code[0] = 0x5F; // pop edi
    code[1] = 0x5E; // pop esi
    code[2] = 0x5D; // pop ebp
    code[3] = 0x5B; // pop ebx
    code[4..10].copy_from_slice(&[0x81, 0xC4, 0xE0, 0x00, 0x00, 0x00]); // add esp,0xe0
    code[10..14].copy_from_slice(&[0x8B, 0x44, 0x24, 0x04]); // mov eax,[esp+4]  param_2
    code[14] = 0xC7; // mov dword [eax], mask
    code[15] = 0x00;
    code[20..23].copy_from_slice(&[0xC2, 0x08, 0x00]); // ret 8

    std::ptr::copy_nonoverlapping(code.as_ptr(), cave_ptr, CAVE_SIZE);

    std::ptr::write_unaligned(cave_ptr.add(16) as *mut u32, mask); // mascara DESPUES del copy

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, CAVE_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-fundacion: VirtualProtect cave fallo");
        return false;
    }

    // Patch 0x0053478A: JMP cave
    let patch = (IMAGE_BASE + HOOK_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + HOOK_RVA + 5);
    if !VirtualProtect(patch as _, 5, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-fundacion: VirtualProtect patch fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    if !VirtualProtect(patch as _, 5, old, &mut old).as_bool() {
        log_error("mod-fundacion: restaurar proteccion patch fallo");
        return false;
    }

    true
}

/// Cave en el epilogo del generador (0x005341B4) que llama al helper de
/// ciudad preferida ANTES de que se empaquete el registro:
///   lea eax,[esp+0x58]; push eax; call p3esp_pref_ciudad (stdcall RET 4);
///   LEA ECX,[esp+0x4C]; PUSH 0x10; jmp 0x005341BA
unsafe fn apply_hook_ciudad() -> bool {
    if !mi_hook_target_matches() {
        log_error(
            "mod-fundacion: version o bytes del EXE no compatibles \
             (0x005341B4 no es LEA ECX,[esp+0x4C] + PUSH 0x10)",
        );
        return false;
    }
    let cave = VirtualAlloc(None, MI_CAVE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-fundacion: VirtualAlloc cave ciudad devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let p = cave as *mut u8;

    let helper = p3esp_pref_ciudad as usize as u32;
    let rel_call = helper.wrapping_sub(cave_addr + 10); // call en offset 5, next=10
    let mut i = 0usize;
    let put = |p: *mut u8, i: &mut usize, bytes: &[u8]| unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.add(*i), bytes.len());
        *i += bytes.len();
    };
    put(p, &mut i, &[0x8D, 0x44, 0x24, 0x58]); // lea eax,[esp+0x58]
    put(p, &mut i, &[0x50]); // push eax
    put(p, &mut i, &[0xE8]); // call rel32 (offset 5)
    put(p, &mut i, &rel_call.to_le_bytes());
    put(p, &mut i, &[0x8D, 0x4C, 0x24, 0x4C]); // lea ecx,[esp+0x4C] (original)
    put(p, &mut i, &[0x6A, 0x10]); // push 0x10 (original)
    let rel_jmp = MI_HOOK_CONT.wrapping_sub(cave_addr + i as u32 + 5);
    put(p, &mut i, &[0xE9]); // jmp rel32 (offset 16)
    put(p, &mut i, &rel_jmp.to_le_bytes());
    if i != MI_CAVE_SIZE {
        log_error("mod-fundacion: tamano de cave ciudad desajustado (bug interno)");
        return false;
    }

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, MI_CAVE_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-fundacion: VirtualProtect cave ciudad fallo");
        return false;
    }
    let patch = (IMAGE_BASE + MI_HOOK_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + MI_HOOK_RVA + 5);
    if !VirtualProtect(patch as _, 5, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-fundacion: VirtualProtect patch ciudad fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    if !VirtualProtect(patch as _, 5, old, &mut old).as_bool() {
        log_error("mod-fundacion: restaurar proteccion patch ciudad fallo");
        return false;
    }
    true
}

/// Cave en el tick diario (0x004F8C9F) que detecta cambios del cfg y
/// resetea el guard (refresco=auto):
///   pushfd; pushad; call p3esp_tick_guard (cdecl); popad; popfd;
///   mov eax,[0x70299C]; jmp 0x004F8CA4
unsafe fn apply_hook_guard() -> bool {
    if !guard_hook_target_matches() {
        log_error("mod-fundacion: bytes en 0x004F8C9F no compatibles");
        return false;
    }
    let cave = VirtualAlloc(None, GUARD_CAVE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-fundacion: VirtualAlloc cave guard devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let p = cave as *mut u8;

    let helper = p3esp_tick_guard as usize as u32;
    let rel_call = helper.wrapping_sub(cave_addr + 2 + 5); // call en offset 2
    let mut i = 0usize;
    let put = |p: *mut u8, i: &mut usize, bytes: &[u8]| unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.add(*i), bytes.len());
        *i += bytes.len();
    };
    put(p, &mut i, &[0x9C]); // pushfd
    put(p, &mut i, &[0x60]); // pushad
    put(p, &mut i, &[0xE8]); // call rel32 (offset 2)
    put(p, &mut i, &rel_call.to_le_bytes());
    put(p, &mut i, &[0x61]); // popad
    put(p, &mut i, &[0x9D]); // popfd
    put(p, &mut i, &[0xA1, 0x9C, 0x29, 0x70, 0x00]); // mov eax,[0x70299C]
    let rel_jmp = GUARD_HOOK_CONT.wrapping_sub(cave_addr + i as u32 + 5);
    put(p, &mut i, &[0xE9]); // jmp rel32 (offset 14)
    put(p, &mut i, &rel_jmp.to_le_bytes());
    if i != GUARD_CAVE_SIZE {
        log_error("mod-fundacion: tamano de cave guard desajustado (bug interno)");
        return false;
    }

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, GUARD_CAVE_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-fundacion: VirtualProtect cave guard fallo");
        return false;
    }
    let patch = (IMAGE_BASE + GUARD_HOOK_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + GUARD_HOOK_RVA + 5);
    if !VirtualProtect(patch as _, 5, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-fundacion: VirtualProtect patch guard fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    if !VirtualProtect(patch as _, 5, old, &mut old).as_bool() {
        log_error("mod-fundacion: restaurar proteccion patch guard fallo");
        return false;
    }
    true
}

// ---------- start() (contrato del modloader) -----------

static STARTED: Once = Once::new();

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let mut ok = true;
    STARTED.call_once(|| {
        match cfg_modo() {
            MODO_PERSONALIZADO => {
                let mask = compute_forced_mask();
                if !apply_force_hook(mask) {
                    ok = false;
                } else {
                    log_info(&format!(
                        "mod-fundacion: modo personalizado, hook de forzado aplicado \
                         (mask=0x{:08X})",
                        mask
                    ));
                }
            }
            m => {
                // VAINILLA y CUATRO comparten el fix de los productos mas
                // escasos (antiguo mod-productos-fundacion). Sin hook.
                if !apply_offset_patch(
                    "mod-fundacion",
                    "productos sub 3->4",
                    0x005345AF,
                    &hex("670083E903"),
                    &hex("670083E904"),
                ) {
                    ok = false;
                } else {
                    log_info("mod-fundacion: parche de productos aplicado");
                }
                // CUATRO: 4 productos/fabricas en vez de 3.
                // MOV EBP,0x3 -> 0x4 en 0x00534599. Seguro: el bucle de
                // insercion ya mantiene 4 entradas y la compensacion de
                // duplicados no puede pasar de 4.
                if m == MODO_CUATRO && ok {
                    // 0x00534599: MOV EBP,0x3 -> conteo del bucle de mascara
                    if !apply_offset_patch(
                        "mod-fundacion",
                        "fabricas 3->4",
                        0x00534599,
                        &hex("BD03000000"),
                        &hex("BD04000000"),
                    ) {
                        ok = false;
                    } else {
                        log_info("mod-fundacion: modo 4+fix, parche de 4a fabrica aplicado");
                    }
                }
            }
        }

        // ---- ciudad preferida (siempre instalado; helper es no-op sin cfg) ----
        let prefs = cfg_ciudades_pref();
        if !prefs.is_empty() {
            let nombres: Vec<&str> = prefs.iter().map(|p| nombre_ciudad(*p)).collect();
            log_info(&format!(
                "mod-fundacion: ciudades preferidas [{}] para la mision de fundar; \
                 se usa la primera no fundada (si no, la calculada)",
                nombres.join(", ")
            ));
        }
        guardar_prefs(&prefs);
        if !apply_hook_ciudad() {
            // No aborta: el mod sigue sirviendo para productos.
            log_error(
                "mod-fundacion: hook de ciudad NO instalado; \
                 la clave ciudad no tendra efecto",
            );
        }

        // ---- refresco=auto: recalculo al cambiar el cfg ----
        if cfg_refresco_auto() {
            if apply_hook_guard() {
                log_info(
                    "mod-fundacion: refresco=auto; cambios en [fundacion] \
                     recalculan la mision al instante",
                );
            } else {
                log_error(
                    "mod-fundacion: hook de refresco NO instalado; \
                     los cambios de cfg NO forzaran recalculo",
                );
            }
        }
    });
    ok as u32
}
