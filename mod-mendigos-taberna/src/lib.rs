// mod-mendigos-taberna
//
// La taberna acepta hasta 100 marineros (antes 50).
//
// Reescrito a estilo mods ingleses: parche por VA absoluto + VirtualProtect.

use p3_esp_modlib::{apply_offset_patch, hex};

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    apply_offset_patch(
        "mod-mendigos-taberna",
        "limite 50->100",
        0x005D62C0,
        &hex("FF00000083F83289"),
        &hex("FF00000083F86489"),
    ) as u32
}
