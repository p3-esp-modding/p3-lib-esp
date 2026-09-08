// mod-limite-ciudades
//
// Mision gobernador: sube el limite de 26 a 36 ciudades.
// Si se quiere 30 en vez de 36, cambiar 24 por 1E.
//
// Reescrito a estilo mods ingleses: parche por VA absoluto + VirtualProtect
// (no find/replace). Direccion y bytes verificados en el exe espanol
// (ver DIRECCIONES_EXE_ESPANOL.md).

use p3_esp_modlib::{apply_offset_patch, hex};

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    apply_offset_patch(
        "mod-limite-ciudades",
        "limite 26->36",
        0x00533D1A,
        &hex("5300837D101A"),
        &hex("5300837D1024"),
    ) as u32
}
