// mod-refresco-misiones
//
// OBSERVADOR de las misiones de gobernador (almirantazgo), con opcion
// EXPERIMENTAL de acortar la fecha de la mision de fundar ciudad.
//
// ===========================================================================
// QUE HACE EL JUEGO (resumen del analisis, exe espanol VA=0x400000+fileoff)
// ===========================================================================
//
// El generador de misiones es FUN_00533CA0 (ECX = GameWorld 0x701B20). Corre:
//   - al iniciar/cargar partida (call en 0x005E1BF5), y
//   - en el tick diario (call en 0x004F8CAD) SOLO SI ds:0x70299C == 0.
// Cada vez que corre intenta generar UNA mision de cada uno de los 5 tipos
// (bucle con jump table en 0x005341FC): fundar ciudad (caso 0), ruta
// terrestre, pirata notorio, esconderijo y problemas de suministro.
//
// Para cada tipo calcula una FECHA = 1er dia de (ahora + N meses):
//   fundar ciudad:  ADD ECX,0x19 (25 meses) en 0x00533D31
//   pirata notorio: ADD ECX,0x3  ( 3 meses) en 0x00533FBB
//   esconderijo:    ADD ECX,0x7  ( 7 meses) en 0x005340CC
// y registra un registro de 20 bytes (tipo 0x85) en el gestor 0x702970.
//
// NO ESTA CERRADO ESTATICAMENTE si esa fecha es la de OFERTA de la mision o
// el PLAZO para cumplirla una vez aceptada (la evidencia de las cartas del
// juego apunta a lo segundo). POR ESO el modo por defecto SOLO LOREA: jugando
// una sesion y comparando misiones_log.txt con lo que se ve en el juego
// (cuando aparece la mision vs. la fecha limite que muestra al aceptarla)
// se cierra la cuestion sin riesgo de estropear la partida.
//
// ===========================================================================
// DONDE ENGANCHA
// ===========================================================================
//
// Epilogo comun del generador en 0x005341B4: justo ANTES de que el payload
// de 16 bytes se copie al registro. En ese punto (ESP = frame del generador):
//   [esp+0x4C] = fecha calculada (u32, ticks)
//   [esp+0x50] = tipo de mision (0 = fundar ciudad; los demas casos no lo
//                escriben y queda 0 del memset inicial)
//   [esp+0x54] = mascara de productos (u32; solo caso fundar ciudad)
//   [esp+0x58] = ciudad elegida (u8; solo caso fundar ciudad)
// El hook sustituye los 5 bytes 0x005341B4 (8D 4C 24 4C 6A = LEA ECX,[esp+4C]
// + PUSH 0x10) por un JMP a la cave; la cave reejecuta esos bytes, loguea (y
// opcionalmente reescribe la fecha) y salta a 0x005341BA.
//
// ===========================================================================
// LA LECCION STDCALL (bug de la v1 de este mod, NO USAR builds antiguas)
// ===========================================================================
// FUN_005343F0 (escasez/productos) es STDCALL: acaba en RET 8 (0x00534794) y
// LIMPIA sus 2 argumentos. Hookear su call-site (0x00533D7E) con una cave que
// hace "call FUN_005343F0; <mas codigo>; ret 8" NO FUNCIONA: el RET 8 de la
// funcion devuelve DIRECTAMENTE a la call-site (se salta el resto de la cave)
// y el RET final de la cave saltaria a una direccion basura. Regla: al hookear
// la CALL a una funcion stdcall, hay que reejecutar los bytes desplazados y
// saltar detras (como hace esta cave), no llamar-la desde la cave.
// Verificado byte a byte con objdump (verificar_mods.py).
//
// ===========================================================================
// CONFIG ([misiones] en p3_esp_mods.cfg)
// ===========================================================================
//   modo = loguear (DEFAULT) | acortar
//   fundarCiudadMeses = 3   (solo con modo=acortar; 0..=36, vanilla 25)
//
// El log se escribe en misiones_log.txt (carpeta del juego) y en
// Patrician3_modloader.log.
//
// NOTA sobre el parche de 1 byte: tocar el inmediato 0x19 en 0x00533D33 solo
// permite N >= 12 (la division magica por meses exige mes+N >= 13 y el INC EDX
// fuerza >= 1 anio). La cave no tiene esa limitacion.

#![allow(non_snake_case, non_camel_case_types)]

use p3_esp_modlib::{log_error, log_info};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Once;
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, PAGE_READWRITE,
};

const IMAGE_BASE: u32 = 0x00400000;

/// 0x005341B4: epilogo comun del generador de misiones FUN_00533CA0.
/// Bytes originales: 8D 4C 24 4C 6A (LEA ECX,[esp+0x4C] + PUSH 0x10).
const HOOK_RVA: u32 = 0x001341B4;
const HOOK_EXPECTED: [u8; 5] = [0x8D, 0x4C, 0x24, 0x4C, 0x6A];
/// Continuacion tras los 2 bytes desplazados (el PUSH 0x10 acaba en 0x5341BA).
const HOOK_CONT: u32 = 0x005341BA;
/// GameWorld+0x14 = tiempo raw (TICKS_PER_YEAR = 93440 ticks, dia = 256).
const WORLD_TIME_ADDR: u32 = 0x00701B34;
const TICKS_PER_YEAR: u32 = 93440;
const TICKS_PER_DAY: u32 = 256;

/// Tamanos de cave: loguear = 32 bytes; acortar = 53 bytes.
const CAVE_SIZE_LOG: usize = 32;
const CAVE_SIZE_ACORTAR: usize = 53;

unsafe fn hook_target_matches() -> bool {
    std::slice::from_raw_parts((IMAGE_BASE + HOOK_RVA) as *const u8, 5) == HOOK_EXPECTED
}

// ---------- config ----------

fn read_cfg_value(section: &str, key: &str) -> Option<String> {
    let content = std::fs::read_to_string("p3_esp_mods.cfg").ok()?;
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

fn cfg_modo_acortar() -> bool {
    match read_cfg_value("misiones", "modo") {
        Some(v) => {
            let s = v.trim().to_ascii_lowercase();
            match s.as_str() {
                "loguear" | "log" | "loguea" | "observar" | "" => false,
                "acortar" | "rapido" | "fast" => true,
                _ => {
                    log_error(&format!(
                        "mod-refresco-misiones: modo=\"{}\" no reconocido (loguear|acortar); \
                         se usa loguear",
                        v.trim()
                    ));
                    false
                }
            }
        }
        None => false,
    }
}

fn cfg_meses() -> u32 {
    match read_cfg_value("misiones", "fundarCiudadMeses")
        .or_else(|| read_cfg_value("misiones", "fundar_ciudad_meses"))
    {
        Some(v) => match v.trim().parse::<u32>() {
            Ok(n @ 0..=36) => n,
            Ok(n) => {
                log_error(&format!(
                    "mod-refresco-misiones: fundarCiudadMeses={} fuera de rango (0..=36); se usa 36",
                    n
                ));
                36
            }
            Err(_) => {
                log_error(&format!(
                    "mod-refresco-misiones: fundarCiudadMeses=\"{}\" no es un numero; se usa 3",
                    v.trim()
                ));
                3
            }
        },
        None => 3,
    }
}

// ---------- logger (stdcall, la cave la llama con 4 args) ----------

fn fecha(ticks: u32) -> (u32, u32) {
    (ticks / TICKS_PER_YEAR, (ticks % TICKS_PER_YEAR) / TICKS_PER_DAY)
}

/// Llamada desde la cave con (due, tipo, mascara, ciudad).
/// stdcall: RET 16 limpia los 4 args empujados por la cave.
#[no_mangle]
pub unsafe extern "stdcall" fn p3esp_mision_log(due: u32, tipo: u32, mask: u32, town: u32) {
    let now = *(WORLD_TIME_ADDR as *const u32);
    let (ya, yd) = fecha(now);
    let (va, vd) = fecha(due);
    let gap = due.wrapping_sub(now) / TICKS_PER_DAY;
    let linea = format!(
        "tipo={} mascara=0x{:08X} ciudad={} generado_en=(anio {}, dia {}) \
         fecha_mision=(anio {}, dia {}) delta_dias={}\n",
        tipo, mask, town, ya, yd, va, vd, gap
    );
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("misiones_log.txt") {
        let _ = f.write_all(linea.as_bytes());
    }
    log_info(&format!("mod-refresco-misiones: {}", linea.trim_end()));
}

// ---------- cave ----------

unsafe fn apply_hook(acortar: bool, delta: u32) -> bool {
    if !hook_target_matches() {
        log_error(
            "mod-refresco-misiones: version o bytes del EXE no compatibles \
             (0x005341B4 no es LEA ECX,[esp+0x4C] + PUSH 0x10)",
        );
        return false;
    }
    let size = if acortar { CAVE_SIZE_ACORTAR } else { CAVE_SIZE_LOG };
    let cave = VirtualAlloc(None, size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-refresco-misiones: VirtualAlloc devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let p = cave as *mut u8;

    struct Cave {
        p: *mut u8,
        n: usize,
    }
    impl Cave {
        fn push(&mut self, bytes: &[u8]) {
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.p.add(self.n), bytes.len()) };
            self.n += bytes.len();
        }
        fn len(&self) -> usize {
            self.n
        }
    }
    let mut c = Cave { p, n: 0 };

    if acortar {
        // Solo reescribir la fecha del caso "fundar ciudad" (tipo == 0):
        //   cmp dword [esp+0x50],0 ; jne +14 ; mov eax,[0x701B34] ;
        //   add eax,delta ; mov [esp+0x4C],eax
        c.push(&[0x83, 0x7C, 0x24, 0x50, 0x00]); // cmp dword [esp+0x50],0
        c.push(&[0x75, 0x0E]); // jne +14 (salta el bloque de reescritura)
        c.push(&[0xA1]); // mov eax,[0x00701B34]
        c.push(&WORLD_TIME_ADDR.to_le_bytes());
        c.push(&[0x05]); // add eax, delta
        c.push(&delta.to_le_bytes());
        c.push(&[0x89, 0x44, 0x24, 0x4C]); // mov [esp+0x4C],eax  (fecha)
    }

    // logger(due, tipo, mask, town) - stdcall RET 16; args en orden inverso.
    // En este punto ESP = frame del generador:
    //   town=[esp+0x58] mask=[esp+0x54] tipo=[esp+0x50] due=[esp+0x4C]
    // Cada push desplaza ESP 4 abajo, asi que siempre toca [esp+0x58].
    for _ in 0..4 {
        c.push(&[0xFF, 0x74, 0x24, 0x58]); // push dword [esp+0x58]
    }

    // call p3esp_mision_log (rel32)
    let logger = p3esp_mision_log as usize as u32;
    c.push(&[0xE8]);
    let rel_call = logger.wrapping_sub(cave_addr + c.len() as u32 + 5);
    c.push(&rel_call.to_le_bytes());

    // Bytes originales desplazados: LEA ECX,[esp+0x4C] + PUSH 0x10
    c.push(&[0x8D, 0x4C, 0x24, 0x4C]);
    c.push(&[0x6A, 0x10]);

    // jmp 0x005341BA (rel32)
    c.push(&[0xE9]);
    let rel_jmp = HOOK_CONT.wrapping_sub(cave_addr + c.len() as u32 + 5);
    c.push(&rel_jmp.to_le_bytes());

    let n = c.len();
    if n != size {
        log_error("mod-refresco-misiones: tamano de cave desajustado (bug interno)");
        return false;
    }

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, size, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-refresco-misiones: VirtualProtect cave fallo");
        return false;
    }

    // JMP rel32 en 0x005341B4 -> cave
    let patch = (IMAGE_BASE + HOOK_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + HOOK_RVA + 5);
    if !VirtualProtect(patch as _, 5, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-refresco-misiones: VirtualProtect patch fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    if !VirtualProtect(patch as _, 5, old, &mut old).as_bool() {
        log_error("mod-refresco-misiones: restaurar proteccion patch fallo");
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
        let acortar = cfg_modo_acortar();
        let delta = (cfg_meses().saturating_mul(TICKS_PER_YEAR)) / 12;
        if apply_hook(acortar, delta) {
            if acortar {
                log_info(&format!(
                    "mod-refresco-misiones: modo ACORTAR (EXPERIMENTAL); la fecha de \
                     la mision de fundar ciudad se reescribe a ahora+delta={} ticks \
                     (25 meses vanilla = {} ticks). Los registros van a misiones_log.txt",
                    delta,
                    25 * TICKS_PER_YEAR / 12
                ));
            } else {
                log_info(
                    "mod-refresco-misiones: modo LOGUEAR; las misiones del gobernador \
                     se registran en misiones_log.txt (sin tocar nada del juego)",
                );
            }
        } else {
            ok = false;
        }
    });
    ok as u32
}
