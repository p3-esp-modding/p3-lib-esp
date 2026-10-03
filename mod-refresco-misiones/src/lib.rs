// mod-refresco-misiones
//
// OBSERVADOR de la mision de fundar ciudad (gobernador/almirantazgo).
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
// + PUSH 0x10) por un JMP a la cave; la cave reejecuta esos bytes, loguea
// y salta a 0x005341BA.
//
// TERCER HOOK (0x004F8C9F, tick diario): el guard ds:0x70299C decide si el
// generador corre (0 = sin mision pendiente). Si el usuario cambia el cfg
// ([fundacion] modo/productos o [misiones] ciudad), el helper firma el cfg
// (p3_esp_firma.txt), resetea el guard a 0 y EN ESE MISMO tick el generador
// recalcula con la config nueva (sin tener que jugar meses ni aceptar/cerrar
// la mision).
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
//   ciudad = indice 0..39 o nombre (p. ej. "gdansk"): ciudad PREFERIDA para
//   la mision de fundar. Si ya esta fundada se ignora y se usa la que calcula
//   el juego. Sin clave o vacio = sin preferencia.
// Solo registra la mision de FUNDAR ciudad; los demas tipos se ignoran.
// El log se escribe en misiones_log.txt (carpeta del juego).

#![allow(non_snake_case, non_camel_case_types)]

use p3_esp_modlib::{log_error, log_info};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, Once};
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ,
    PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, PAGE_READWRITE,
};
use windows::Win32::System::SystemInformation::GetLocalTime;

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
/// Gestor de scheduled tasks 0x702970. Layout ESPANOL VERIFICADO en runtime
/// (desensamblado de FUN_0054C050/FUN_005454D0 + volcado hex):
///   +0x00 estado/flags u32 (0x12, 0x4...; NO es un puntero)
///   +0x08 puntero a otro objeto de heap (NO son las tareas)
///   +0x48 contenedor de tareas inline:
///     +0x0E => gestor+0x56: count u16 (max 0x34=52 inline)
///     +0x10 => gestor+0x58: tareas inline, stride 0x14 (20 bytes)
///   +0x468: puntero a bloque overflow si count >= 52
/// scheduled_task (0x14 bytes): +0 opcode u32 (0x85 = mision almirantazgo,
/// 0x86 = limpieza), payload 16 bytes en +4:
///   +4 due u32, +8 tipo u8 (0=fundar..4=suministro), +9 merchant u8 (0xFF),
///   +0xA reschedule_counter u8, +0xC mascara u32, +0x10 ciudad u8.
const TASK_MGR_ADDR: u32 = 0x00702970;
const TASK_COUNT_OFF: usize = 0x56;
const TASKS_OFF: usize = 0x58;
const TASK_OPCODE_MISION: u32 = 0x85;
const TASK_SIZE: usize = 0x14;
const MAX_INLINE_TASKS: usize = 0x34;

/// 0x0054C0AE: RET 4 de FUN_0054C050 (registra la tarea en el contenedor
/// y YA la contiene al llegar aqui). Bytes: C2 04 00 90 90 (ret 4 + 2 nops).
/// Verificado: ningun salto aterriza en 0x54C0AE..0x54C0B2.
const HOOK2_RVA: u32 = 0x0014C0AE;
const HOOK2_EXPECTED: [u8; 5] = [0xC2, 0x04, 0x00, 0x90, 0x90];
/// Cave2: pushfd/pushad/push arg/call logger/add esp/popad/popfd/ret 4.
const CAVE2_SIZE: usize = 19;

/// Tamano de cave: 42 bytes (32 del logger + 10 del call al helper de
/// ciudad preferida: lea eax,[esp+0x58] (4) + push eax (1) + call (5)).
const CAVE_SIZE: usize = 42;

/// world+0x10 = towns_count (u32), world+0x18 = ids de ciudades fundadas (u8),
/// tabla ciudad->id usada en la mision (byte [ciudad] en 0x673D60).
const WORLD_TOWNS_COUNT_ADDR: u32 = 0x00701B30;
const WORLD_TOWN_IDS_ADDR: u32 = 0x00701B38;
const TOWN_ID_TABLE_ADDR: u32 = 0x00673D60;
const MAX_SITES: u8 = 0x28; // 40 emplazamientos (cmp de FUN_00516FC0)

/// Ciudades preferidas del cfg ([misiones] ciudad=a,b,c): lista ordenada de
/// hasta 8, probadas en cascada (primera no fundada gana). 0xFF = fin de lista.
static PREFS: Mutex<[u8; 8]> = Mutex::new([0xFF; 8]);
static AVISA_PREF: Once = Once::new();

/// 0x004F8C9F: `mov eax,[0x70299C]` en el tick diario, justo ANTES del test
/// que decide si el generador corre (0x4F8CA4 test eax,eax; jne skip).
/// Bytes: A1 9C 29 70 00. Verificado: ningun salto aterriza en
/// 0x4F8C9F..0x4F8CA4. La cave llama al helper (firma del cfg) y relee el
/// guard, asi que si el cfg cambio, el MISMO tick regenera las misiones.
const HOOK3_RVA: u32 = 0x000F8C9F;
const HOOK3_EXPECTED: [u8; 5] = [0xA1, 0x9C, 0x29, 0x70, 0x00];
const HOOK3_CONT: u32 = 0x004F8CA4;
/// Cave3: pushfd/pushad/call helper/popad/popfd/mov eax,[guard]/jmp 0x4F8CA4.
const CAVE3_SIZE: usize = 19;

unsafe fn hook_target_matches() -> bool {
    std::slice::from_raw_parts((IMAGE_BASE + HOOK_RVA) as *const u8, 5) == HOOK_EXPECTED
}

unsafe fn hook2_target_matches() -> bool {
    std::slice::from_raw_parts((IMAGE_BASE + HOOK2_RVA) as *const u8, 5) == HOOK2_EXPECTED
}

unsafe fn hook3_target_matches() -> bool {
    std::slice::from_raw_parts((IMAGE_BASE + HOOK3_RVA) as *const u8, 5) == HOOK3_EXPECTED
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

/// [misiones] ciudad = una o varias separadas por coma, en orden de preferencia
/// (p. ej. "memel,windau,konigsberg"). Valores invalidos se saltan con error
/// en el log. Lista vacia = sin preferencia.
fn cfg_ciudades_pref() -> Vec<u8> {
    let v = match read_cfg_value("misiones", "ciudad") {
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
                    "mod-refresco-misiones: ciudad={} fuera de rango (0..39); se salta",
                    n
                ));
            }
            continue;
        }
        let l = s.to_ascii_lowercase();
        match CIUDADES.iter().position(|c| c.to_ascii_lowercase() == l) {
            Some(i) => out.push(i as u8),
            None => log_error(&format!(
                "mod-refresco-misiones: ciudad=\"{}\" no reconocida; se salta",
                s
            )),
        }
    }
    out.truncate(8);
    out
}

// ---------- firma del cfg (para forzar recalculo al cambiar config) ----------

/// Claves que obligan a recalcular las misiones. Se concatenan y comparan con
/// la ultima firma guardada en p3_esp_firma.txt (carpeta del juego).
fn cfg_firma() -> String {
    let modo = read_cfg_value("fundacion", "modo").unwrap_or_default();
    let prods = read_cfg_value("fundacion", "productos").unwrap_or_default();
    let ciudad = read_cfg_value("misiones", "ciudad").unwrap_or_default();
    format!("{}|{}|{}", modo.trim(), prods.trim(), ciudad.trim())
}

const FIRMA_FILE: &str = "p3_esp_firma.txt";

fn firma_guardada() -> String {
    std::fs::read_to_string(FIRMA_FILE).unwrap_or_default()
}

/// Llamada desde la cave3 (tick diario, ANTES del test del guard).
/// Si el cfg cambio respecto a la ultima firma guardada, resetea el guard
/// 0x70299C (0 = sin mision pendiente => el generador puede correr) y guarda
/// la firma nueva. El MISMO tick vuelve a leer el guard y regenera.
/// Se limita a 1 lectura de cfg por segundo real (el tick corre 256 veces
/// por dia de juego).
#[no_mangle]
pub unsafe extern "C" fn p3esp_tick_guard() {
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u64)
        .unwrap_or(0);
    static ULT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
    let ult = ULT.load(Ordering::Relaxed);
    if ahora.saturating_sub(ult) < 1 {
        return;
    }
    if ULT.compare_exchange(ult, ahora, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        return;
    }
    let firma = cfg_firma();
    if firma == firma_guardada() {
        return;
    }
    // Cambio detectado (o primera vez): resetear el guard y guardar firma.
    *((0x70299Cusize) as *mut u32) = 0;
    let _ = std::fs::write(FIRMA_FILE, &firma);
    log_info(&format!(
        "mod-refresco-misiones: config cambiada ({}); guard 0x70299C reseteado, \
         el generador recalcula las misiones",
        firma
    ));
}

// ---------- logger (stdcall, la cave la llama con 4 args) ----------

fn fecha(ticks: u32) -> (u32, u32) {
    (ticks / TICKS_PER_YEAR, (ticks % TICKS_PER_YEAR) / TICKS_PER_DAY)
}

/// Hora real del PC como "AAAA-MM-DD HH:MM:SS" (para ordenar el log por
/// sesion: cada vez que se abre el ayuntamiento salta una rafaga de lineas).
fn hora_real() -> String {
    unsafe {
        let st = GetLocalTime();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }
}

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

/// Llamada desde la cave con puntero a [esp+0x58] (ciudad + byte de id).
/// Aplica la PRIMERA ciudad preferida del cfg que no este fundada (cascada);
/// si todas estan fundadas o coincide con la actual, deja la que calculo el
/// juego (preferencia, nunca fuerza).
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
                "mod-refresco-misiones: ninguna ciudad preferida disponible \
                 (ya fundadas); se usa la que calcula el juego",
            );
        });
    }
}

/// Vuelca las tareas vivas (opcode 0x85) del contenedor inline del gestor:
/// count u16 en gestor+0x56 (max 52), tareas en gestor+0x58 stride 0x14.
/// Layout verificado con el desensamblado de FUN_005454D0 (memcpy por
/// [ebx+eax*4+0x10] con eax*5 => stride 0x14) y el volcado hex en runtime.
/// Solo lee memoria del juego; no modifica nada.
unsafe fn volcar_gestor(f: &mut std::fs::File, ts: &str) {
    let count = *((TASK_MGR_ADDR as usize + TASK_COUNT_OFF) as *const u16) as usize;
    let n = count.min(MAX_INLINE_TASKS);
    let now = *(WORLD_TIME_ADDR as *const u32);
    let mut vivas = 0usize;
    for i in 0..n {
        let t = (TASK_MGR_ADDR as usize + TASKS_OFF + i * TASK_SIZE) as *const u8;
        let opcode = std::ptr::read_unaligned(t as *const u32);
        if opcode != TASK_OPCODE_MISION {
            continue;
        }
        vivas += 1;
        let due = std::ptr::read_unaligned(t.add(4) as *const u32);
        let tipo = *t.add(8);
        if tipo != 0 {
            continue;
        }
        let recont = *t.add(0xA);
        let mmask = std::ptr::read_unaligned(t.add(0xC) as *const u32);
        let mciudad = *t.add(0x10);
        let (va, vd) = fecha(due);
        let _ = writeln!(
            f,
            "[{}] viva[{}] tipo={}({}) mascara=0x{:08X} ciudad={}({}) recont={} \
             fecha=(anio {}, dia {}) restante_dias={}",
            ts, i, tipo, nombre_mision(tipo), mmask, mciudad, nombre_ciudad(mciudad), recont,
            va, vd, due.wrapping_sub(now) / TICKS_PER_DAY
        );
    }
    let _ = writeln!(f, "[{}] gestor: {} tareas, {} misiones vivas", ts, n, vivas);
}

fn nombre_mision(tipo: u8) -> &'static str {
    match tipo {
        0 => "fundar",
        1 => "ruta",
        2 => "pirata",
        3 => "esconderijo",
        4 => "suministro",
        _ => "?",
    }
}

/// Llamada desde la cave2 (RET 4 de FUN_0054C050) con el puntero al record
/// recien registrado (arg stdcall de la funcion). cdecl aqui: el caller
/// limpia con add esp,4. Solo actua si el record es una mision (0x85).
#[no_mangle]
pub unsafe extern "C" fn p3esp_tarea_registrada(record: u32) {
    if record < 0x10000 {
        return;
    }
    let r = record as *const u8;
    let opcode = std::ptr::read_unaligned(r as *const u32);
    if opcode != TASK_OPCODE_MISION {
        return;
    }
    let ts = hora_real();
    let now = *(WORLD_TIME_ADDR as *const u32);
    let due = std::ptr::read_unaligned(r.add(4) as *const u32);
    let tipo = *r.add(8);
    if tipo != 0 {
        return;
    }
    let mmask = std::ptr::read_unaligned(r.add(0xC) as *const u32);
    let mciudad = *r.add(0x10);
    let (va, vd) = fecha(due);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("misiones_log.txt") {
        let _ = writeln!(
            f,
            "[{}] registrada tipo={}({}) mascara=0x{:08X} ciudad={}({}) \
             fecha=(anio {}, dia {}) restante_dias={}",
            ts, tipo, nombre_mision(tipo), mmask, mciudad, nombre_ciudad(mciudad), va, vd,
            due.wrapping_sub(now) / TICKS_PER_DAY
        );
        volcar_gestor(&mut f, &ts);
        let _ = writeln!(f);
    }
}

/// Llamada desde la cave con (due, tipo, mascara, ciudad).
/// stdcall: RET 16 limpia los 4 args empujados por la cave.
/// Solo loguea la mision de FUNDAR (tipo 0xFF00); las demas se ignoran.
/// Tras loguearla vuelca el estado del gestor (misiones vivas).
#[no_mangle]
pub unsafe extern "stdcall" fn p3esp_mision_log(due: u32, tipo: u32, mask: u32, town: u32) {
    // 0xFF00 = fundar ciudad. Los demas casos (0xFF01..0xFF04) no interesan.
    if tipo != 0xFF00 {
        return;
    }
    let ts = hora_real();
    let now = *(WORLD_TIME_ADDR as *const u32);
    let (ya, yd) = fecha(now);
    let (va, vd) = fecha(due);
    let gap = due.wrapping_sub(now) / TICKS_PER_DAY;
    let ciudad = (town & 0xFF) as u8;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("misiones_log.txt") {
        let _ = writeln!(
            f,
            "[{}] fundar mascara=0x{:08X} ciudad={}({}) generado_en=(anio {}, dia {}) \
             fecha_mision=(anio {}, dia {}) delta_dias={}",
            ts, mask, ciudad, nombre_ciudad(ciudad), ya, yd, va, vd, gap
        );
        volcar_gestor(&mut f, &ts);
        let _ = writeln!(f);
    }
    log_info(&format!(
        "mod-refresco-misiones: [{}] fundar ciudad={}({}) mascara=0x{:08X} delta_dias={}",
        ts, ciudad, nombre_ciudad(ciudad), mask, gap
    ));
}

// ---------- cave ----------

unsafe fn apply_hook() -> bool {
    if !hook_target_matches() {
        log_error(
            "mod-refresco-misiones: version o bytes del EXE no compatibles \
             (0x005341B4 no es LEA ECX,[esp+0x4C] + PUSH 0x10)",
        );
        return false;
    }
    let size = CAVE_SIZE;
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

    // ciudad preferida: si aplica, pisa [esp+0x58]/[esp+0x59] ANTES de
    // loguear, para que el logger ya vea la ciudad realmente ofrecida.
    // lea eax,[esp+0x58]; push eax; call p3esp_pref_ciudad (stdcall RET 4).
    c.push(&[0x8D, 0x44, 0x24, 0x58]); // lea eax,[esp+0x58]
    c.push(&[0x50]); // push eax
    let rel_pref = (p3esp_pref_ciudad as usize as u32)
        .wrapping_sub(cave_addr + c.len() as u32 + 5);
    c.push(&[0xE8]);
    c.push(&rel_pref.to_le_bytes());

    // logger(due, tipo, mask, town) - stdcall RET 16; args en orden inverso.
    // En este punto ESP = frame del generador:
    //   town=[esp+0x58] mask=[esp+0x54] tipo=[esp+0x50] due=[esp+0x4C]
    // Cada push desplaza ESP 4 abajo, asi que siempre toca [esp+0x58].
    for _ in 0..4 {
        c.push(&[0xFF, 0x74, 0x24, 0x58]); // push dword [esp+0x58]
    }

    // call p3esp_mision_log (rel32)
    let logger = p3esp_mision_log as usize as u32;
    let rel_call = logger.wrapping_sub(cave_addr + c.len() as u32 + 5);
    c.push(&[0xE8]);
    c.push(&rel_call.to_le_bytes());

    // Bytes originales desplazados: LEA ECX,[esp+0x4C] + PUSH 0x10
    c.push(&[0x8D, 0x4C, 0x24, 0x4C]);
    c.push(&[0x6A, 0x10]);

    // jmp 0x005341BA (rel32)
    let rel_jmp = HOOK_CONT.wrapping_sub(cave_addr + c.len() as u32 + 5);
    c.push(&[0xE9]);
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

/// Segundo hook: 0x0054C0AE (RET 4 de FUN_0054C050, el que REGISTRA la
/// tarea). Dispara justo DESPUES de que la tarea este en el contenedor, a
/// diferencia del epilogo del generador (donde aun no existe). La cave:
///   pushfd; pushad; push [esp+0x28] (arg original [esp+4] = record);
///   call p3esp_tarea_registrada (cdecl); add esp,4; popad; popfd; ret 4
/// El ret 4 final ejecuta la instruccion original desplazada (el JMP solo
/// pisa C2 04 00 90 90), asi que no hace falta saltar detras.
unsafe fn apply_hook2() -> bool {
    if !hook2_target_matches() {
        log_error("mod-refresco-misiones: bytes en 0x0054C0AE no compatibles");
        return false;
    }
    let cave = VirtualAlloc(
        None,
        CAVE2_SIZE,
        MEM_COMMIT | MEM_RESERVE,
        PAGE_READWRITE,
    );
    if cave.is_null() {
        log_error("mod-refresco-misiones: VirtualAlloc cave2 devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let p = cave as *mut u8;

    // Rel del call calculado ANTES de escribir el opcode (leccion E0503/rel32).
    let logger = p3esp_tarea_registrada as usize as u32;
    let rel_call = logger.wrapping_sub(cave_addr + 6 + 5);
    let mut i = 0usize;
    let put = |p: *mut u8, i: &mut usize, bytes: &[u8]| unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.add(*i), bytes.len());
        *i += bytes.len();
    };
    put(p, &mut i, &[0x9C]); // pushfd
    put(p, &mut i, &[0x60]); // pushad
    put(p, &mut i, &[0xFF, 0x74, 0x24, 0x28]); // push dword [esp+0x28]
    put(p, &mut i, &[0xE8]); // call rel32 (offset 6)
    put(p, &mut i, &rel_call.to_le_bytes());
    put(p, &mut i, &[0x83, 0xC4, 0x04]); // add esp,4 (cdecl limpia el arg)
    put(p, &mut i, &[0x61]); // popad
    put(p, &mut i, &[0x9D]); // popfd
    put(p, &mut i, &[0xC2, 0x04, 0x00]); // ret 4 (instruccion original)
    if i != CAVE2_SIZE {
        log_error("mod-refresco-misiones: tamano de cave2 desajustado (bug interno)");
        return false;
    }

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, CAVE2_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-refresco-misiones: VirtualProtect cave2 fallo");
        return false;
    }

    // JMP rel32 en 0x0054C0AE -> cave2 (pisa C2 04 00 90 90).
    let patch = (IMAGE_BASE + HOOK2_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + HOOK2_RVA + 5);
    if !VirtualProtect(patch as _, 5, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-refresco-misiones: VirtualProtect patch2 fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    if !VirtualProtect(patch as _, 5, old, &mut old).as_bool() {
        log_error("mod-refresco-misiones: restaurar proteccion patch2 fallo");
        return false;
    }

    true
}

/// Tercer hook: 0x004F8C9F (mov eax,[0x70299C] del tick diario). La cave:
///   pushfd; pushad; call p3esp_tick_guard (cdecl); popad; popfd;
///   mov eax,[0x70299C]; jmp 0x4F8CA4
/// Si el helper reseteo el guard, la relectura hace que el generador corra
/// EN ESTE MISMO tick (el test eax,eax de 0x4F8CA4 ve 0).
unsafe fn apply_hook3() -> bool {
    if !hook3_target_matches() {
        log_error("mod-refresco-misiones: bytes en 0x004F8C9F no compatibles");
        return false;
    }
    let cave = VirtualAlloc(None, CAVE3_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-refresco-misiones: VirtualAlloc cave3 devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let p = cave as *mut u8;

    let helper = p3esp_tick_guard as usize as u32;
    let rel_call = helper.wrapping_sub(cave_addr + 2 + 5);
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
    let rel_jmp = HOOK3_CONT.wrapping_sub(cave_addr + i as u32 + 5);
    put(p, &mut i, &[0xE9]); // jmp rel32 (offset 14)
    put(p, &mut i, &rel_jmp.to_le_bytes());
    if i != CAVE3_SIZE {
        log_error("mod-refresco-misiones: tamano de cave3 desajustado (bug interno)");
        return false;
    }

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, CAVE3_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-refresco-misiones: VirtualProtect cave3 fallo");
        return false;
    }
    let patch = (IMAGE_BASE + HOOK3_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + HOOK3_RVA + 5);
    if !VirtualProtect(patch as _, 5, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-refresco-misiones: VirtualProtect patch3 fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    if !VirtualProtect(patch as _, 5, old, &mut old).as_bool() {
        log_error("mod-refresco-misiones: restaurar proteccion patch3 fallo");
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
        let prefs = cfg_ciudades_pref();
        if !prefs.is_empty() {
            if let Ok(mut arr) = PREFS.lock() {
                for (i, p) in prefs.iter().enumerate() {
                    arr[i] = *p;
                }
            }
            let nombres: Vec<&str> = prefs.iter().map(|p| nombre_ciudad(*p)).collect();
            log_info(&format!(
                "mod-refresco-misiones: ciudades preferidas [{}] para la mision de \
                 fundar; se usa la primera no fundada (si no, la calculada)",
                nombres.join(", ")
            ));
        }
        if apply_hook() {
            log_info(
                "mod-refresco-misiones: observando la mision de fundar ciudad; \
                 los registros van a misiones_log.txt (sin tocar nada del juego)",
            );
        } else {
            ok = false;
        }
        // Hook2 (post-registro en FUN_0054C050): solo informativo; si falla
        // no se aborta el mod (el hook1 sigue funcionando).
        if !apply_hook2() {
            log_error(
                "mod-refresco-misiones: hook2 (post-registro) NO instalado; \
                 solo se loguearan las generaciones, no los volcados del gestor",
            );
        }
        // Hook3 (tick diario): detecta cambios del cfg y resetea el guard
        // para recalcular sin jugar meses. Informativo: si falla, el mod
        // sigue con hook1/hook2 (sin forzar recalculo).
        if !apply_hook3() {
            log_error(
                "mod-refresco-misiones: hook3 (guard del tick) NO instalado; \
                 los cambios de cfg NO forzaran recalculo hasta recargar partida",
            );
        }
    });
    ok as u32
}
