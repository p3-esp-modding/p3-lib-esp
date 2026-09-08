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
// Los errores se escriben en Patrician3_modloader.log (via p3-esp-modlib).

#![allow(non_snake_case, non_camel_case_types)]

use p3_esp_modlib::{apply_offset_patch, hex, log_error, log_info};
use std::fs;
use std::sync::Once;
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
    });
    ok as u32
}
