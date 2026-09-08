// mod-fullhd
//
// Resolucion 1920x1080 (antes 1280/1024x768).
//
// Reescrito a estilo mods ingleses: parche por VA absoluto + VirtualProtect
// (no find/replace). Los 7 VAs y bytes estan verificados contra el exe
// espanol original (ver DIRECCIONES_EXE_ESPANOL.md).

use p3_esp_modlib::{apply_offset_patch, hex};

#[no_mangle]
pub unsafe extern "C" fn start() -> u32 {
    let patches: [(&str, u32, &[u8], &[u8]); 7] = [
        ("width 1280->1920 [1]", 0x004506BB, &hex("3d000500000f85"), &hex("3d800700000f85")),
        ("width 1280->1920 [2]", 0x00450BDC, &hex("3d000500007545"), &hex("3d800700007545")),
        ("1024x768->1920x1080 [a]", 0x0044A78A, &hex("c744244c00040000c744245000030000"), &hex("c744244c80070000c744245038040000")),
        ("1024x768->1920x1080 [b]", 0x00453D71, &hex("c744241800040000c744241c00030000"), &hex("c744241880070000c744241c38040000")),
        ("1024x768->1920x1080 [c]", 0x004599A8, &hex("c744243c00040000c744244000030000"), &hex("c744243c80070000c744244038040000")),
        ("1024x768->1920x1080 [d]", 0x00486A02, &hex("c744244800040000c744244c00030000"), &hex("c744244880070000c744244c38040000")),
        ("1024x768->1920x1080 [e]", 0x0048AA16, &hex("c744242400040000c744242800030000"), &hex("c744242480070000c744242838040000")),
    ];

    let mut ok = true;
    for (tag, va, expected, new) in patches {
        ok &= apply_offset_patch("mod-fullhd", tag, va, expected, new);
    }
    ok as u32
}
