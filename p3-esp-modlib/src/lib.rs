// p3-esp-modlib
//
// Crate compartido para los mods del Patrician 3 espanol.
//
// Proporciona:
//   - log_error: escribe en un log unico compartido Patrician3_modloader.log
//   - apply_patches: aplica parches find/replace sobre la imagen del EXE
//     en memoria (como hacia p3-patcher.py, pero en caliente al cargar
//     el mod). Verifica unicidad del patron antes de reemplazar.
//
// Los parches son los mismos que existian como JSON en p3-esp-patch, pero
// ahora se aplican en runtime por cada mod DLL.

#![allow(non_snake_case)]

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS};

pub const IMAGE_BASE: u32 = 0x00400000;
const LOG_FILE: &str = "Patrician3_modloader.log";

// ---------- log compartido ----------

fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86400) as i64;
    let ds = secs % 86400;
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let yr = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = mp + if mp < 10 { 3 } else { -9 };
    let yr = yr + if mo <= 2 { 1 } else { 0 };
    let h = ds / 3600;
    let m = (ds % 3600) / 60;
    let s = ds % 60;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", yr, mo, d, h, m, s)
}

/// Vacia Patrician3_modloader.log si existe. Solo se ejecuta una vez por
/// proceso (aunque varios mods la llamen), para que cada arranque del
/// juego empiece con el log limpio.
pub fn log_clear() {
    static CLEARED: Once = Once::new();
    CLEARED.call_once(|| {
        let _ = OpenOptions::new().write(true).truncate(true).open(LOG_FILE);
    });
}

/// Solo escribe errores (patron no encontrado, ambiguo, fallo de proteccion).
/// El log se crea solo si hay algun error que reportar.
pub fn log_error(msg: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(f, "{} [ERROR] {}", timestamp(), msg);
    }
}

/// Escribe informacion (p. ej. direccion de cada parche aplicado).
pub fn log_info(msg: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(LOG_FILE) {
        let _ = writeln!(f, "{} [INFO] {}", timestamp(), msg);
    }
}

// ---------- find/replace en memoria ----------

/// Devuelve el tamano de la imagen (SizeOfImage) desde el PE header
/// mapeado en IMAGE_BASE. 0 si no se puede leer.
unsafe fn size_of_image() -> usize {
    let base = IMAGE_BASE as *const u8;
    let e_lfanew = *(base.add(0x3C) as *const u32);
    if e_lfanew == 0 {
        return 0;
    }
    // Optional header starts at e_lfanew + 4 (signature) + 20 (file header)
    let opt = base.add(e_lfanew as usize + 24);
    // SizeOfImage is at offset 56 in the optional header (PE32)
    let magic = *(opt as *const u16);
    if magic != 0x10B {
        return 0; // no es PE32
    }
    *(opt.add(56) as *const u32) as usize
}

fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > haystack.len() {
        return out;
    }
    let mut pos = 0;
    while pos + needle.len() <= haystack.len() {
        if &haystack[pos..pos + needle.len()] == needle {
            out.push(pos);
        }
        pos += 1;
    }
    out
}

/// Aplica una lista de parches (find, replace) sobre la imagen en memoria.
/// Devuelve true si todos se aplicaron.
pub unsafe fn apply_patches(label: &str, patches: &[(Vec<u8>, Vec<u8>)]) -> bool {
    let size = size_of_image();
    if size == 0 {
        log_error(&format!("{}: no se pudo leer SizeOfImage del PE header", label));
        return false;
    }
    let base = IMAGE_BASE as *mut u8;
    let image = std::slice::from_raw_parts(IMAGE_BASE as *const u8, size);

    let mut all_ok = true;
    for (i, (find, replace)) in patches.iter().enumerate() {
        let hits = find_all(image, find);
        match hits.len() {
            0 => {
                log_error(&format!("{}: patch #{} patron no encontrado", label, i + 1));
                all_ok = false;
            }
            1 => {
                let off = hits[0];
                let addr = base.add(off);
                let mut old = PAGE_PROTECTION_FLAGS(0);
                let ok = VirtualProtect(
                    addr as _,
                    replace.len(),
                    PAGE_EXECUTE_READWRITE,
                    &mut old,
                );
                if ok.as_bool() {
                    std::ptr::copy_nonoverlapping(replace.as_ptr(), addr, replace.len());
                    let _ = VirtualProtect(addr as _, replace.len(), old, &mut old);
                    log_info(&format!(
                        "{}: patch #{} aplicado en 0x{:08X} ({} bytes)",
                        label,
                        i + 1,
                        IMAGE_BASE + off as u32,
                        replace.len()
                    ));
                } else {
                    log_error(&format!(
                        "{}: patch #{} VirtualProtect fallo en 0x{:08X}",
                        label,
                        i + 1,
                        IMAGE_BASE + off as u32
                    ));
                    all_ok = false;
                }
            }
            n => {
                log_error(&format!(
                    "{}: patch #{} patron ambiguo ({} coincidencias)",
                    label, i + 1, n
                ));
                all_ok = false;
            }
        }
    }
    all_ok
}

/// Aplica un parche por VA absoluto (estilo mods ingleses: escritura directa
/// en la direccion, no find/replace de patrones).
///
/// Verifica que los bytes actuales en `va` coincidan con `expected` antes de
/// escribir `new`, para no sobreescribir por accidente (p. ej. si el parche ya
/// esta aplicado o el exe es otra version).
///
/// Devuelve true si se escribio new (o ya estaba aplicado), false si fallo.
pub unsafe fn apply_offset_patch(label: &str, tag: &str, va: u32, expected: &[u8], new: &[u8]) -> bool {
    if expected.len() != new.len() {
        log_error(&format!("{}: {} longitudes distintas", label, tag));
        return false;
    }
    let ptr = va as *mut u8;
    let cur = std::slice::from_raw_parts(ptr, expected.len());
    if cur == new {
        log_info(&format!("{}: {} ya aplicado en 0x{:08X}", label, tag, va));
        return true;
    }
    if cur != expected {
        let mut shown = String::new();
        for (i, b) in cur.iter().enumerate().take(8) {
            if i > 0 {
                shown.push(' ');
            }
            shown.push_str(&format!("{:02X}", b));
        }
        log_error(&format!(
            "{}: {} bytes inesperados en 0x{:08X} ({}), no se parchea",
            label, tag, va, shown
        ));
        return false;
    }

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(ptr as _, expected.len(), PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error(&format!("{}: {} VirtualProtect fallo en 0x{:08X}", label, tag, va));
        return false;
    }
    std::ptr::copy_nonoverlapping(new.as_ptr(), ptr, new.len());
    let _ = VirtualProtect(ptr as _, expected.len(), old, &mut old);
    log_info(&format!(
        "{}: {} aplicado en 0x{:08X} ({} bytes)",
        label,
        tag,
        va,
        new.len()
    ));
    true
}

// ---------- helpers para los mods ----------

/// Convierte una cadena hex (con o sin espacios) a bytes.
pub fn hex(s: &str) -> Vec<u8> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    let bytes = cleaned.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16).unwrap_or(0) as u8;
        let lo = (bytes[i + 1] as char).to_digit(16).unwrap_or(0) as u8;
        out.push((hi << 4) | lo);
        i += 2;
    }
    out
}
