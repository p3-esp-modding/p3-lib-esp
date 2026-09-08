// mod-satisfaccion-mendigos
//
// Ajusta la satisfaccion de los mendigos (4 -> 3).
//
// Reescrito a estilo mods ingleses: parche por VA absoluto + VirtualProtect.

use p3_esp_modlib::{apply_offset_patch, hex};

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    apply_offset_patch(
        "mod-satisfaccion-mendigos",
        "satisfaccion 4->3",
        0x00434E15,
        &hex("b932000000BA040000008950fc8918"),
        &hex("b932000000BA030000008950fc8918"),
    ) as u32
}
