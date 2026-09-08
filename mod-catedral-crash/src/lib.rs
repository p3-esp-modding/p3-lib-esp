// mod-catedral-crash
//
// Fix del crash al abrir la catedral tras recargar partida.
//
// Causa raiz (en C++): el objeto interior (param_1/ESI) guarda en
// [ESI+4] un puntero a una tabla de texturas. Al recargar partida el
// juego no reinicializa el objeto: la tabla queda NULL y varias
// funciones (independientes entre si) la usan como
//   tabla[idx]  ==  *(base_reg + idx_reg*4)
// con base_reg == NULL, lo que pete en:
//   - 0x00491CBD (FUN_00491930) escribe table[slot]
//   - 0x00491E68 (FUN_00491d50) lee   table[slot]
//   - 0x00491EDA / 0x00491EF0 (FUN_00491ec0) leen table[slot]
//   - y posiblemente otros sitios con el mismo patron [reg+idx*4].
//
// Estrategia v2 (tras ver en juego imagenes del cura fuera de sitio):
//
//   El objeto interior es una maquina de estados:
//     [obj+0x04] tabla de texturas     [obj+0x1f] frame de fundido actual
//     [obj+0x20] texturas cargadas     [obj+0x38] slot activo (6..9 iglesia)
//   El cargador escribe en tabla[[0x20]] y los lectores dibujan
//   tabla[[0x1f]]. FUN_00491f30 (cambio de slot) ya sabe reconstruir
//   TODO el estado: libera entradas y tabla, pone [obj+4]=0, resetea
//   contadores, realoca (prioridad*4) con el alloc del juego y recarga
//   (bloque 0x00491F77).
//
//   - CARGADOR (FUN_00491930): si [ESI+4]==NULL, el cave reconstruye el
//     mismo estado que dejaria FUN_00491f30 ([0x38]=slot pedido,
//     [0x1f]=0, [0x20]=0, [0x3a]=1) y rust_init_table asigna la tabla
//     con el ALLOC DEL JUEGO (0x00651019; free del juego 0x00651042).
//     La carga sigue normal.
//   - LECTORES (FUN_00491d50 / FUN_00491ec0): si [ESI+4]==NULL NO crean
//     tabla ni resetean nada: salen sin dibujar. La v1 inicializaba la
//     tabla tambien aqui, y eso reiniciaba los contadores POR DETRAS del
//     cargador con [0x38]/posiciones stale => maquina de estados rota,
//     recargas duplicadas y el cura pintado antes de tiempo y repetido
//     por otras partes de la pantalla.
//   - LIMPIEZA (FUN_00491f30): guarda intacto su bucle de liberacion
//     cuando la tabla es NULL (salta al bloque 0x00491F77, que ya
//     realoca y recarga correctamente por si mismo).
//
//   VEH de respaldo: ante un access violation con fault address < 64 KB
//   dentro de las funciones de la catedral, decodifica la instruccion
//   [base+idx*4], localiza el registro BASE y lo redirige a un buffer
//   temporal (red de seguridad por si algun sitio no cubierto).
//
// Los errores se escriben en Patrician3_modloader.log (via p3-esp-modlib).

#![allow(non_snake_case, non_camel_case_types)]

use p3_esp_modlib::{log_error, log_info};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Once;
use windows::core::s;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{
    VirtualAlloc, VirtualProtect, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE,
    PAGE_PROTECTION_FLAGS, PAGE_READWRITE, MEM_COMMIT, MEM_RESERVE,
};

const IMAGE_BASE: u32 = 0x00400000;
const EXCEPTION_ACCESS_VIOLATION: u32 = 0xC0000005;
const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

// ----------- asignacion de tabla con el alloc del juego -----------
//
// El juego gestiona la vida de [ESI+4] (tabla de texturas) con su PROPIO
// allocator:
//   - alloc: 0x0049F019 / 0x00651019  (stdcall: push size; eax = ptr)
//   - free : 0x0049F042 / 0x00651042  (stdcall: push ptr)
// que internamente son los wrappers malloc/free del CRT del juego
// (0x0063e1a1 / 0x0063ce60).
//
// Si asignamos la tabla con HeapAlloc, el juego la libera con su free
// (o la reasigna con su alloc) => bloque de un heap liberado con otro =>
// CORRUPCION DE HEAP que se serializa a las partidas guardadas.
// Por eso la tabla se crea SIEMPRE con el alloc del juego.

/// VA del alloc del juego (cdecl: push size, call, add esp,4, eax = ptr).
const GAME_ALLOC_VA: u32 = 0x00651019;
/// VA del free del juego (cdecl: push ptr, call, add esp,4).
const _GAME_FREE_VA: u32 = 0x00651042;
/// Tamano de la tabla nueva. El juego usa slot*4 con slot byte (max 0xFF),
/// asi que 0x400 (1024 bytes) cubre hasta slot 255; el free del juego
/// acepta cualquier tamano, es seguro.
const TABLE_BYTES: usize = 0x400;

unsafe fn patch_targets_match() -> bool {
    let checks: &[(u32, &[u8])] = &[
        (0x0009194E, &[0x89, 0x44, 0x24, 0x04, 0x8B, 0x4C, 0x24, 0x18, 0x33, 0xD2, 0x8A, 0x56, 0x39]),
        (0x00091D5D, &[0x8A, 0x46, 0x39, 0x3B, 0xF8, 0x0F, 0x8D, 0x43, 0x01, 0x00, 0x00]),
        (0x00091ECB, &[0x8A, 0x48, 0x24, 0x84, 0xC9, 0x74, 0x4B]),
        (0x00091F5E, &[0x8B, 0x56, 0x04, 0x8B, 0x04, 0xBA]),
    ];
    checks.iter().all(|&(rva, expected)| {
        std::slice::from_raw_parts((IMAGE_BASE + rva) as *const u8, expected.len()) == expected
    })
}

/// Asigna una tabla nueva de texturas con el ALLOC DEL JUEGO y la escribe
/// en [obj+4]. Los contadores/slot activo los pone ANTES el cave del
/// cargador (FUN_00491930), replicando el estado que deja FUN_00491f30.
/// Llamada unicamente desde el cave del cargador cuando [ESI+4]==NULL.
///
/// OJO: se usa el alloc del juego (no un buffer estatico) porque el juego
/// gestiona la vida de [ESI+4] y lo libera con su free() (visto en
/// FUN_00491f30). Un buffer estatico compartido haria que el free del
/// juego liberara memoria que no es del heap del juego => crash/corrupcion.
#[no_mangle]
pub unsafe extern "C" fn rust_init_table(obj: *mut u8) {
    if obj.is_null() {
        return;
    }

    // Diagnostico temporal: contar cuantas veces se re-asigna la tabla en
    // una sesion. Si este numero crece mucho, la catedral esta filtrando
    // memoria de forma acumulativa (posible causa de corrupcion tardia).
    static INIT_COUNT: AtomicU32 = AtomicU32::new(0);
    let n = INIT_COUNT.fetch_add(1, Ordering::Relaxed);
    if n == 0 || (n + 1) % 50 == 0 {
        log_info(&format!(
            "mod-catedral-crash: rust_init_table usos en esta sesion = {}",
            n + 1
        ));
    }

    let alloc: unsafe extern "C" fn(usize) -> *mut u8 = std::mem::transmute(GAME_ALLOC_VA);
    let table = alloc(TABLE_BYTES);
    if table.is_null() {
        // Red de seguridad: count = 0xFF hace que la guarda del cargador
        // (count < prioridad) falle siempre => no se escribe en la tabla
        // NULL y no hay crash (la escena queda vacia, pero estable).
        *obj.add(0x20) = 0xFF;
        return;
    }
    std::ptr::write_bytes(table, 0u8, TABLE_BYTES);
    std::ptr::write_unaligned(obj.add(4) as *mut u32, table as u32); // [ESI+4] = tabla
    *obj.add(0x20) = 0;                        // reset slot 20
}

// ----------- layout del CONTEXT (winnt.h, x86) -----------
// EDI=0x9c, ESI=0xa0, EBX=0xa4, EDX=0xa8, ECX=0xac,
// EAX=0xb0, EBP=0xb4, EIP=0xb8, ESP=0xc4.

#[repr(C)]
struct CONTEXT {
    _pad_to_edi: [u32; 39],
    Edi: u32,
    Esi: u32,
    Ebx: u32,
    Edx: u32,
    Ecx: u32,
    Eax: u32,
    Ebp: u32,
    Eip: u32,
    _after_eip: [u32; 2],
    Esp: u32,
}

#[repr(C)]
struct EXCEPTION_RECORD {
    ExceptionCode: u32,
    ExceptionFlags: u32,
    ExceptionRecord: *mut EXCEPTION_RECORD,
    ExceptionAddress: *mut u8,
    NumberParameters: u32,
}

#[repr(C)]
struct EXCEPTION_POINTERS {
    ExceptionRecord: *mut EXCEPTION_RECORD,
    ContextRecord: *mut CONTEXT,
}

// ----------- VEH de respaldo -----------

fn reg_field<'a>(ctx: &'a mut CONTEXT, reg: u32) -> Option<&'a mut u32> {
    match reg {
        0 => Some(&mut ctx.Eax),
        1 => Some(&mut ctx.Ecx),
        2 => Some(&mut ctx.Edx),
        3 => Some(&mut ctx.Ebx),
        4 => Some(&mut ctx.Esp),
        5 => Some(&mut ctx.Ebp),
        6 => Some(&mut ctx.Esi),
        7 => Some(&mut ctx.Edi),
        _ => None,
    }
}

unsafe fn decode_sib_base(eip: u32) -> Option<u32> {
    let b0 = *(eip as *const u8);
    let rm_operand;
    let modrm_off;
    match b0 {
        0x8B | 0x89 | 0x8A | 0x3B | 0x83 | 0x81 | 0xC7 => {
            rm_operand = true;
            modrm_off = 1;
        }
        0x0F => {
            let b1 = *((eip + 1) as *const u8);
            if b1 != 0xB6 {
                return None;
            }
            rm_operand = true;
            modrm_off = 2;
        }
        _ => {
            rm_operand = false;
            modrm_off = 0;
        }
    }
    if !rm_operand {
        return None;
    }
    let modrm = *((eip + modrm_off) as *const u8);
    let mod_ = modrm >> 6;
    let rm = modrm & 0x07;
    if rm != 0x04 {
        return None;
    }
    let sib = *((eip + modrm_off + 1) as *const u8);
    let scale = sib >> 6;
    let base = sib & 0x07;
    if scale != 0b10 {
        return None;
    }
    if ((sib >> 3) & 0x07) == 0x04 {
        return None;
    }
    if mod_ == 0 && base == 0x05 {
        return None;
    }
    Some(base as u32)
}

unsafe extern "system" fn cathedral_vh(info: *mut EXCEPTION_POINTERS) -> i32 {
    let rec_ptr = (*(info as *const EXCEPTION_POINTERS)).ExceptionRecord;
    if (*rec_ptr).ExceptionCode != EXCEPTION_ACCESS_VIOLATION {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let fault = if (*rec_ptr).NumberParameters >= 2 {
        *((rec_ptr as *const u32).add(5 + 1))
    } else {
        0xFFFFFFFF
    };

    if fault >= 0x10000 {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let ctx = &mut *((*info).ContextRecord);
    let eip = (*ctx).Eip;

    // Restringido a las funciones de la catedral y su caller
    let in_cathedral = eip >= 0x00491930 && eip < 0x00492000;
    let in_caller = eip >= 0x005CAF00 && eip < 0x005CB000;
    if !in_cathedral && !in_caller {
        return EXCEPTION_CONTINUE_SEARCH;
    }

    let Some(base) = decode_sib_base(eip) else {
        return EXCEPTION_CONTINUE_SEARCH;
    };
    let Some(field) = reg_field(ctx, base) else {
        return EXCEPTION_CONTINUE_SEARCH;
    };

    let old = *field;

    static SEEN: [AtomicU32; 8] = [const { AtomicU32::new(0) }; 8];
    let mut first_time = true;
    for e in &SEEN {
        if e.load(Ordering::Relaxed) == eip {
            first_time = false;
            break;
        }
    }
    if first_time {
        for e in &SEEN {
            if e.compare_exchange(0, eip, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                break;
            }
        }
        log_error(&format!(
            "mod-catedral-crash: VEH (solo log, no toca memoria) EIP=0x{:08X} reg={} old=0x{:08X} fault=0x{:08X}",
            eip, base, old, fault
        ));
    }

    // SOLO LOG: no se reescribe ningun registro ni se desvia el acceso a un
    // buffer temporal (eso corrompia el heap y las partidas guardadas). Si el
    // juego llega aqui, algo quedo descubierto y crasheara visiblemente; el
    // log indica exactamente que EIP cubrir con un cave.
    EXCEPTION_CONTINUE_SEARCH
}

// ----------- code cave en FUN_00491930 (reparar estado + continuar) -----------

unsafe fn apply_patch_00491930() -> bool {
    const PATCH_RVA: u32 = 0x0009194E; // entrada, tras MOV ESI,ECX
    const CONTINUE_RVA: u32 = 0x0009195B; // seguir ejecucion normal
    const CAVE_SIZE: usize = 55;
    const PATCH_SIZE: usize = 13;

    let cave = VirtualAlloc(None, CAVE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-catedral-crash: VirtualAlloc cave 00491930 devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let cave_ptr = cave as *mut u8;

    // 00: 50              push eax            ; guardar PTR_DAT
    // 01: 8B 46 04        mov eax,[esi+4]     ; tabla
    // 04: 85 C0           test eax,eax
    // 06: 75 1C           jnz +0x1C -> 0x24   ; tabla valida -> skip
    // 08: 8B 4C 24 1C     mov ecx,[esp+0x1C]  ; param_2 ([esp+0x18] original + push eax)
    // 0C: 88 4E 38        mov [esi+0x38],cl   ; slot activo = slot pedido
    // 0F: C6 46 3A 01     mov byte [esi+0x3A],1
    // 13: C6 46 1F 00     mov byte [esi+0x1F],0
    // 17: C6 46 20 00     mov byte [esi+0x20],0
    // 1B: 56              push esi            ; arg: this
    // 1C: E8 <rel>        call rust_init_table (rel@1D..20)
    // 21: 83 C4 04        add esp,4
    // 24: 58              pop eax             ; <<< skip: restaurar PTR_DAT
    // 25: 89 44 24 04     mov [esp+4],eax     ; original
    // 29: 8B 4C 24 18     mov ecx,[esp+0x18]  ; original
    // 2D: 33 D2           xor edx,edx         ; original
    // 2F: 8A 56 39        mov dl,[esi+0x39]   ; original
    // 32: E9 <rel>        jmp 0x0049195B      ; continuar (rel@33..36)
    let mut code = [0u8; CAVE_SIZE];
    code[0] = 0x50;
    code[1..4].copy_from_slice(&[0x8B, 0x46, 0x04]);
    code[4..6].copy_from_slice(&[0x85, 0xC0]);
    code[6..8].copy_from_slice(&[0x75, 0x1C]);
    code[8..12].copy_from_slice(&[0x8B, 0x4C, 0x24, 0x1C]);
    code[12..15].copy_from_slice(&[0x88, 0x4E, 0x38]);
    code[15..19].copy_from_slice(&[0xC6, 0x46, 0x3A, 0x01]);
    code[19..23].copy_from_slice(&[0xC6, 0x46, 0x1F, 0x00]);
    code[23..27].copy_from_slice(&[0xC6, 0x46, 0x20, 0x00]);
    code[27] = 0x56;
    code[28] = 0xE8;
    code[33..36].copy_from_slice(&[0x83, 0xC4, 0x04]);
    code[36] = 0x58;
    code[37..41].copy_from_slice(&[0x89, 0x44, 0x24, 0x04]);
    code[41..45].copy_from_slice(&[0x8B, 0x4C, 0x24, 0x18]);
    code[45..47].copy_from_slice(&[0x33, 0xD2]);
    code[47..50].copy_from_slice(&[0x8A, 0x56, 0x39]);
    code[50] = 0xE9;

    std::ptr::copy_nonoverlapping(code.as_ptr(), cave_ptr, CAVE_SIZE);

    let rel_call = (rust_init_table as *const () as usize as u32).wrapping_sub(cave_addr + 0x21);
    let rel_continue = (IMAGE_BASE + CONTINUE_RVA).wrapping_sub(cave_addr + 0x37);
    std::ptr::write_unaligned(cave_ptr.add(0x1D) as *mut u32, rel_call);
    std::ptr::write_unaligned(cave_ptr.add(0x33) as *mut u32, rel_continue);

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, CAVE_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-catedral-crash: VirtualProtect cave 00491930 fallo");
        return false;
    }

    let patch = (IMAGE_BASE + PATCH_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + PATCH_RVA + 5);
    if !VirtualProtect(patch as _, PATCH_SIZE, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-catedral-crash: VirtualProtect patch 00491930 fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    for i in 5..PATCH_SIZE {
        *patch.add(i) = 0x90;
    }
    if !VirtualProtect(patch as _, PATCH_SIZE, old, &mut old).as_bool() {
        log_error("mod-catedral-crash: restaurar proteccion patch 00491930 fallo");
        return false;
    }

    true
}

// ----------- code cave en FUN_00491d50 (salir sin dibujar si NULL) -----------

unsafe fn apply_patch_00491d50() -> bool {
    const PATCH_RVA: u32 = 0x00091D5D; // tras el guard JGE existente
    const CONTINUE_RVA: u32 = 0x00091D68;
    const EARLY_EXIT_RVA: u32 = 0x00091EAB; // epilogo (si slot fuera de rango)
    const CAVE_SIZE: usize = 28;
    const PATCH_SIZE: usize = 11;

    let cave = VirtualAlloc(None, CAVE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-catedral-crash: VirtualAlloc cave 00491d50 devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let cave_ptr = cave as *mut u8;

    // 00: 8B 46 04        mov eax,[esi+4]     ; tabla
    // 03: 85 C0           test eax,eax
    // 05: 0F 84 <rel>     je 0x00491EAB       ; NULL -> salir SIN dibujar (rel@07..0A)
    // 0B: 0F B6 46 39     movzx eax,byte [esi+0x39] ; original (EAX venia a cero)
    // 0F: 3B F8           cmp edi,eax         ; original
    // 11: 0F 8D <rel>     jge 0x00491EAB      ; original (rel@13..16)
    // 17: E9 <rel>        jmp 0x00491D68      ; continuar (rel@18..1B)
    let mut code = [0u8; CAVE_SIZE];
    code[0..3].copy_from_slice(&[0x8B, 0x46, 0x04]);
    code[3..5].copy_from_slice(&[0x85, 0xC0]);
    code[5] = 0x0F;
    code[6] = 0x84;
    code[11..15].copy_from_slice(&[0x0F, 0xB6, 0x46, 0x39]);
    code[15..17].copy_from_slice(&[0x3B, 0xF8]);
    code[17] = 0x0F;
    code[18] = 0x8D;
    code[23] = 0xE9;

    std::ptr::copy_nonoverlapping(code.as_ptr(), cave_ptr, CAVE_SIZE);

    let rel_exit = (IMAGE_BASE + EARLY_EXIT_RVA).wrapping_sub(cave_addr + 0x0B);
    let rel_jge = (IMAGE_BASE + EARLY_EXIT_RVA).wrapping_sub(cave_addr + 0x17);
    let rel_continue = (IMAGE_BASE + CONTINUE_RVA).wrapping_sub(cave_addr + 0x1C);
    std::ptr::write_unaligned(cave_ptr.add(0x07) as *mut u32, rel_exit);
    std::ptr::write_unaligned(cave_ptr.add(0x13) as *mut u32, rel_jge);
    std::ptr::write_unaligned(cave_ptr.add(0x18) as *mut u32, rel_continue);

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, CAVE_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-catedral-crash: VirtualProtect cave 00491d50 fallo");
        return false;
    }

    let patch = (IMAGE_BASE + PATCH_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + PATCH_RVA + 5);
    if !VirtualProtect(patch as _, PATCH_SIZE, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-catedral-crash: VirtualProtect patch 00491d50 fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    for i in 5..PATCH_SIZE {
        *patch.add(i) = 0x90;
    }
    if !VirtualProtect(patch as _, PATCH_SIZE, old, &mut old).as_bool() {
        log_error("mod-catedral-crash: restaurar proteccion patch 00491d50 fallo");
        return false;
    }

    true
}

// ----------- code cave en FUN_00491ec0 (salir sin dibujar si NULL) -----------

unsafe fn apply_patch_00491ec0() -> bool {
    const PATCH_RVA: u32 = 0x00091ECB; // tras MOV ESI,ECX (0x00491EC9)
    const CONTINUE_RVA: u32 = 0x00091ED2; // seguir ejecucion normal (MOV EDX,[ESI+4])
    const EARLY_EXIT_RVA: u32 = 0x00091F1D; // LAB_00491f1d (early exit de la funcion)
    const CAVE_SIZE: usize = 32; // 3+2+6+5+3+2+6+5 (el rel32 del JMP final ocupa 0x1C..0x1F)
    const PATCH_SIZE: usize = 7; // MOV CL, TEST, JZ (0x00491ECB..0x00491ED1)

    let cave = VirtualAlloc(None, CAVE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-catedral-crash: VirtualAlloc cave 00491ec0 devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let cave_ptr = cave as *mut u8;

    // 00: 8B 46 04        mov eax,[esi+4]     ; tabla
    // 03: 85 C0           test eax,eax
    // 05: 0F 84 <rel>     je 0x00491F1D       ; NULL -> salir SIN dibujar (rel@07..0A)
    // 0B: A1 D0 3A 6F 00  mov eax,ds:0x6f3ad0 ; restaurar EAX original (DAT_006f3ad0)
    // 10: 8A 48 24        mov cl,[eax+0x24]   ; original
    // 13: 84 C9           test cl,cl          ; original
    // 15: 0F 84 <rel>     je 0x00491F1D       ; original (rel@17..1A)
    // 1B: E9 <rel>        jmp 0x00491ED2      ; continuar (rel@1C..1F)
    let mut code = [0u8; CAVE_SIZE];
    code[0..3].copy_from_slice(&[0x8B, 0x46, 0x04]);
    code[3..5].copy_from_slice(&[0x85, 0xC0]);
    code[5] = 0x0F;
    code[6] = 0x84;
    code[11..16].copy_from_slice(&[0xA1, 0xD0, 0x3A, 0x6F, 0x00]);
    code[16..19].copy_from_slice(&[0x8A, 0x48, 0x24]);
    code[19..21].copy_from_slice(&[0x84, 0xC9]);
    code[21] = 0x0F;
    code[22] = 0x84;
    code[27] = 0xE9;

    std::ptr::copy_nonoverlapping(code.as_ptr(), cave_ptr, CAVE_SIZE);

    let rel_exit = (IMAGE_BASE + EARLY_EXIT_RVA).wrapping_sub(cave_addr + 0x0B);
    let rel_je = (IMAGE_BASE + EARLY_EXIT_RVA).wrapping_sub(cave_addr + 0x1B);
    let rel_continue = (IMAGE_BASE + CONTINUE_RVA).wrapping_sub(cave_addr + 0x20);
    std::ptr::write_unaligned(cave_ptr.add(0x07) as *mut u32, rel_exit);
    std::ptr::write_unaligned(cave_ptr.add(0x17) as *mut u32, rel_je);
    std::ptr::write_unaligned(cave_ptr.add(0x1C) as *mut u32, rel_continue);

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, CAVE_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-catedral-crash: VirtualProtect cave 00491ec0 fallo");
        return false;
    }

    let patch = (IMAGE_BASE + PATCH_RVA) as *mut u8;
    let rel = cave_addr.wrapping_sub(IMAGE_BASE + PATCH_RVA + 5);
    if !VirtualProtect(patch as _, PATCH_SIZE, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-catedral-crash: VirtualProtect patch 00491ec0 fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(patch.add(1) as *mut u32, rel);
    for i in 5..PATCH_SIZE {
        *patch.add(i) = 0x90;
    }
    if !VirtualProtect(patch as _, PATCH_SIZE, old, &mut old).as_bool() {
        log_error("mod-catedral-crash: restaurar proteccion patch 00491ec0 fallo");
        return false;
    }

    true
}

// ----------- guard en FUN_00491f30 (limpieza tras recarga) -----------

unsafe fn apply_patch_00491f30() -> bool {
    const PATCH_RVA: u32 = 0x00091F5E; // mov edx,[esi+4] dentro del bucle
    const CONTINUE_RVA: u32 = 0x00091F64; // mov eax,[edx+edi*4] / push eax
    const RESET_RVA: u32 = 0x00091F77; // bloque que libera/recrea la tabla
    const CAVE_SIZE: usize = 19;
    const PATCH_SIZE: usize = 6; // mov edx + primeros 2 bytes de mov eax

    let cave = VirtualAlloc(None, CAVE_SIZE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if cave.is_null() {
        log_error("mod-catedral-crash: VirtualAlloc cave 00491f30 devolvio NULL");
        return false;
    }
    let cave_addr = cave as u32;
    let cave_ptr = cave as *mut u8;

    // 00: 8B 56 04        mov edx,[esi+4]       ; tabla
    // 03: 85 D2           test edx,edx
    // 05: 0F 84 <rel>     je 0x00491F77         ; saltar limpieza si NULL
    // 0B: 8B 04 BA        mov eax,[edx+edi*4]   ; original
    // 0E: E9 <rel>        jmp 0x00491F64        ; seguir con push eax
    let mut code = [0u8; CAVE_SIZE];
    code[0..3].copy_from_slice(&[0x8B, 0x56, 0x04]);
    code[3..5].copy_from_slice(&[0x85, 0xD2]);
    code[5..7].copy_from_slice(&[0x0F, 0x84]);
    code[11..14].copy_from_slice(&[0x8B, 0x04, 0xBA]);
    code[14] = 0xE9;

    std::ptr::copy_nonoverlapping(code.as_ptr(), cave_ptr, CAVE_SIZE);

    let rel_reset = (IMAGE_BASE + RESET_RVA).wrapping_sub(cave_addr + 0x0B);
    let rel_continue = (IMAGE_BASE + CONTINUE_RVA).wrapping_sub(cave_addr + 0x13);
    std::ptr::write_unaligned(cave_ptr.add(0x07) as *mut u32, rel_reset);
    std::ptr::write_unaligned(cave_ptr.add(0x0F) as *mut u32, rel_continue);

    let mut old = PAGE_PROTECTION_FLAGS(0);
    if !VirtualProtect(cave as _, CAVE_SIZE, PAGE_EXECUTE_READ, &mut old).as_bool() {
        log_error("mod-catedral-crash: VirtualProtect cave 00491f30 fallo");
        return false;
    }

    let patch = (IMAGE_BASE + PATCH_RVA) as *mut u8;
    if !VirtualProtect(patch as _, PATCH_SIZE, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
        log_error("mod-catedral-crash: VirtualProtect patch 00491f30 fallo");
        return false;
    }
    *patch = 0xE9;
    std::ptr::write_unaligned(
        patch.add(1) as *mut u32,
        cave_addr.wrapping_sub(IMAGE_BASE + PATCH_RVA + 5),
    );
    *patch.add(5) = 0x90;
    if !VirtualProtect(patch as _, PATCH_SIZE, old, &mut old).as_bool() {
        log_error("mod-catedral-crash: restaurar proteccion patch 00491f30 fallo");
        return false;
    }

    true
}

// ----------- start() (contrato del modloader) -----------

static STARTED: Once = Once::new();

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    if !patch_targets_match() {
        log_error("mod-catedral-crash: version o bytes del EXE no compatibles");
        return 0;
    }
    let mut ok = true;
    STARTED.call_once(|| {
        ok &= apply_patch_00491930();
        ok &= apply_patch_00491d50();
        ok &= apply_patch_00491ec0();
        ok &= apply_patch_00491f30();
        if ok {
            log_info("mod-catedral-crash: 4 patches aplicados (0x491930, 0x491d50, 0x491ec0, 0x491f30)");
        }
        // Registrar el VEH (primero en la cadena) via GetProcAddress
        let kernel32 = match LoadLibraryW(
            windows::core::PCWSTR(windows::core::HSTRING::from("kernel32.dll").as_ptr()),
        ) {
            Ok(h) => h,
            Err(_) => {
                log_error("mod-catedral-crash: no se pudo cargar kernel32.dll");
                return;
            }
        };
        let add_veh = match GetProcAddress(kernel32, s!("AddVectoredExceptionHandler")) {
            Some(p) => p,
            None => {
                log_error("mod-catedral-crash: AddVectoredExceptionHandler no encontrado");
                return;
            }
        };
        let add_veh: unsafe extern "system" fn(
            u32,
            unsafe extern "system" fn(*mut EXCEPTION_POINTERS) -> i32,
        ) -> *mut core::ffi::c_void = std::mem::transmute(add_veh);
        let h = add_veh(1, cathedral_vh);
        if h.is_null() {
            log_error("mod-catedral-crash: AddVectoredExceptionHandler devolvio NULL");
        } else {
            log_info("mod-catedral-crash: VEH registrado");
        }
    });
    ok as u32
}
