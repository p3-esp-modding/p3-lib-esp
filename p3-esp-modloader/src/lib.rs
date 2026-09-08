// p3-esp-modloader
//
// Cargador de mods del Patrician 3 espanol.
//
// Diseno identico al p3-modloader de la comunidad inglesa (repo P3Modding/p3-lib):
//
//   - DllMain (DLL_PROCESS_ATTACH): parchea el call a WinMain del EXE para que
//     salte a WinMain_hook. No hace nada mas; no carga mods aqui.
//   - WinMain_hook: carga los mods de ./mods (LoadLibraryW + GetProcAddress
//     "start") y luego llama al WinMain original.
//
// El objetivo es replicar 1:1 la estrategia inglesa, que lleva anos sin
// corromper partidas. Antes se habia intentado cargar los mods en DllMain
// creyendo que era la causa de la corrupcion, pero la comunidad inglesa
// carga en WinMain_hook y no corrompe, asi que se restauro ese diseno.
//
// Para minimizar el runtime cargado en el proceso del juego (2003, heap
// sensible), esta lib importa lo minimo y se compila con el perfil release
// sin dependencias pesadas. En la maquina Windows se compila con MSVC
// (i686-pc-windows-msvc) como la comunidad inglesa.

use std::fs;
use std::path::Path;
use std::sync::Once;

use windows::core::{s, PCWSTR, HSTRING};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS};
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

const IMAGE_BASE: u32 = 0x00400000;
const WINMAIN_CALL_RVA: u32 = 0x0023E082;
const WINMAIN_VA: u32 = 0x0064D540;
const EXPECTED_TIMESTAMP: u32 = 0x4118A8B2;
const EXPECTED_WINMAIN_CALL: [u8; 5] = [0xE8, 0xB9, 0xF4, 0x00, 0x00];

/// Vacia el log compartido una sola vez al arrancar (el modloader es la
/// primera DLL que se carga). Asi cada sesion del juego empieza limpia.
static CLEARED: Once = Once::new();
fn clear_log() {
    CLEARED.call_once(|| {
        let _ = fs::File::create("Patrician3_modloader.log");
    });
}

unsafe fn target_is_supported() -> bool {
    let base = IMAGE_BASE as *const u8;
    if std::ptr::read_unaligned(base as *const u16) != 0x5A4D {
        return false;
    }
    let e_lfanew = std::ptr::read_unaligned(base.add(0x3C) as *const u32) as usize;
    if e_lfanew > 0x1000 - 24 {
        return false;
    }
    let pe = base.add(e_lfanew);
    if std::ptr::read_unaligned(pe as *const u32) != 0x0000_4550 {
        return false;
    }
    if std::ptr::read_unaligned(pe.add(4) as *const u16) != 0x014C
        || std::ptr::read_unaligned(pe.add(8) as *const u32) != EXPECTED_TIMESTAMP
    {
        return false;
    }
    let optional = pe.add(24);
    if std::ptr::read_unaligned(optional as *const u16) != 0x010B
        || std::ptr::read_unaligned(optional.add(28) as *const u32) != IMAGE_BASE
        || (std::ptr::read_unaligned(optional.add(56) as *const u32) as usize)
            < WINMAIN_CALL_RVA as usize + EXPECTED_WINMAIN_CALL.len()
    {
        return false;
    }
    std::slice::from_raw_parts(
        base.add(WINMAIN_CALL_RVA as usize),
        EXPECTED_WINMAIN_CALL.len(),
    ) == EXPECTED_WINMAIN_CALL
}

#[no_mangle]
extern "system" fn DllMain(_dll: *const u8, reason: u32, _reserved: *const u8) -> u32 {
    if reason != DLL_PROCESS_ATTACH {
        return 1;
    }

    // Solo parchear la llamada a WinMain para que salte a WinMain_hook.
    // Al igual que el modloader ingles, no se cargan mods aqui; se cargan en
    // WinMain_hook (despues de la init estatica del juego).
    unsafe {
        if !target_is_supported() {
            return 0;
        }
        let patch_addr = (IMAGE_BASE + WINMAIN_CALL_RVA) as *mut u8;
        let hook_fn = WinMain_hook as *const () as u32;
        let rel = hook_fn.wrapping_sub(IMAGE_BASE + WINMAIN_CALL_RVA + 5);

        let mut old = PAGE_PROTECTION_FLAGS(0);
        if !VirtualProtect(patch_addr as _, 5, PAGE_EXECUTE_READWRITE, &mut old).as_bool() {
            return 0;
        }

        *patch_addr = 0xE8;
        std::ptr::write_unaligned(patch_addr.add(1) as *mut u32, rel);

        if !VirtualProtect(patch_addr as _, 5, old, &mut old).as_bool() {
            return 0;
        }
    }

    1
}

#[no_mangle]
extern "stdcall" fn WinMain_hook(
    h_instance: windows::Win32::Foundation::HMODULE,
    h_prev_instance: windows::Win32::Foundation::HMODULE,
    lp_cmd_line: windows::core::PSTR,
    n_show_cmd: u32,
) -> i32 {
    clear_log();
    load_mods();

    let winmain: extern "stdcall" fn(
        windows::Win32::Foundation::HMODULE,
        windows::Win32::Foundation::HMODULE,
        windows::core::PSTR,
        u32,
    ) -> i32 = unsafe { std::mem::transmute(WINMAIN_VA) };
    winmain(h_instance, h_prev_instance, lp_cmd_line, n_show_cmd)
}

fn load_mods() {
    static LOADED: Once = Once::new();
    LOADED.call_once(|| {
        let mods_path = Path::new("./mods");
        if !mods_path.exists() {
            let _ = fs::create_dir(mods_path);
        }

        if let Ok(entries) = fs::read_dir(mods_path) {
            for entry in entries.flatten() {
                if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    continue;
                }
                let Some(name) = entry.file_name().to_str().map(|s| s.to_owned()) else {
                    continue;
                };
                if !name.ends_with(".dll") {
                    continue;
                }

                let filepath = format!("./mods/{}", name);
                unsafe {
                    let Ok(hmodule) = LoadLibraryW(PCWSTR(HSTRING::from(&filepath).as_ptr()))
                    else {
                        continue;
                    };
                    let Some(start) = GetProcAddress(hmodule, s!("start")) else {
                        continue;
                    };
                    let start_fn: unsafe extern "C" fn() -> u32 = std::mem::transmute(start);
                    start_fn();
                }
            }
        }
    });
}
