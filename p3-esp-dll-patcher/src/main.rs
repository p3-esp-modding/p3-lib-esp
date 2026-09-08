// p3-esp-patcher
//
// Parcheador del EXE de Patrician 3 espanol. Anade la importacion de
// p3_esp_modloader.dll a la import table del EXE para que el modloader
// se cargue automaticamente al iniciar el juego.
//
// Uso:
//   p3-esp-patcher.exe [ruta]
//
//   ruta: directorio donde esta Patrician3.exe (por defecto, el
//         directorio actual).
//
// Comportamiento de entrada/salida:
//   - Si existe Patrician3_original.exe junto al EXE, se toma como
//     original y se genera el parcheado Patrician3.exe (util para
//     desarrollo: lanzarlo varias veces regenera el parcheado).
//   - Si no, se toma Patrician3.exe y se genera Patrician3_modloader.exe
//     (no se toca el original).
//
// Solo anade el import del modloader; el resto de cambios son mods DLL
// en ./mods que el usuario elige.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const MODLOADER_DLL: &str = "p3_esp_modloader.dll";
const MODLOADER_FUNC: &str = "DllMain";
const EXPECTED_TIMESTAMP: u32 = 0x4118A8B2;
const EXPECTED_IMAGE_BASE: u32 = 0x00400000;
const EXPECTED_WINMAIN_CALL_OFFSET: usize = 0x0023E082;
const EXPECTED_WINMAIN_CALL: [u8; 5] = [0xE8, 0xB9, 0xF4, 0x00, 0x00];

fn read_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

fn write_u32(data: &mut [u8], off: usize, v: u32) {
    data[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn write_u16(data: &mut [u8], off: usize, v: u16) {
    data[off..off + 2].copy_from_slice(&v.to_le_bytes());
}

/// Anade la importacion de MODLOADER_DLL/MODLOADER_FUNC al EXE.
///
/// A diferencia de la version anterior (que escribia la import table al
/// final del .rdata y CORROMPIA datos del juego que viven ahi, p. ej. la
/// tabla de punteros a funciones en 0x692FE8/0x693000), esta version crea
/// una SECCION PE NUEVA llamada ".mod" al final del archivo (igual que
/// hace el modloader ingles de la comunidad: anade una seccion .mod y
/// copia la import table ahi). Asi no se toca ninguna seccion existente.
fn add_dll_import(exe: &mut Vec<u8>, dll_name: &str, function_name: &str) -> bool {
    if exe.len() < 64 || &exe[0..2] != b"MZ" {
        return false;
    }
    if exe.len() < EXPECTED_WINMAIN_CALL_OFFSET + EXPECTED_WINMAIN_CALL.len()
        || exe[EXPECTED_WINMAIN_CALL_OFFSET
            ..EXPECTED_WINMAIN_CALL_OFFSET + EXPECTED_WINMAIN_CALL.len()]
            != EXPECTED_WINMAIN_CALL
    {
        return false;
    }
    let e_lfanew = read_u32(exe, 60) as usize;
    let pe_off = e_lfanew;
    let Some(file_header_end) = pe_off.checked_add(24) else {
        return false;
    };
    if file_header_end > exe.len() || read_u32(exe, pe_off) != 0x0000_4550 {
        return false;
    }
    let opt_start = pe_off + 4 + 20;
    let opt_header_size = read_u16(exe, pe_off + 4 + 16) as usize;
    let Some(opt_end) = opt_start.checked_add(opt_header_size) else {
        return false;
    };
    if opt_end > exe.len()
        || opt_header_size < 112
        || read_u16(exe, pe_off + 4) != 0x014C
        || read_u32(exe, pe_off + 8) != EXPECTED_TIMESTAMP
        || read_u16(exe, opt_start) != 0x010B
        || read_u32(exe, opt_start + 28) != EXPECTED_IMAGE_BASE
    {
        return false;
    }

    // DataDirectory[1] = Import Table (RVA at opt_start+96+8, size at +96+12)
    let import_rva = read_u32(exe, opt_start + 96 + 8) as usize;
    let dd_rva_offset = opt_start + 96 + 8;
    let dd_size_offset = opt_start + 96 + 12;

    if import_rva == 0 {
        return false;
    }

    let num_sections = read_u16(exe, pe_off + 4 + 2) as usize;
    if num_sections >= 96 {
        return false;
    }
    let section_start = pe_off + 4 + 20 + opt_header_size;
    let Some(section_table_end) = section_start.checked_add(num_sections * 40) else {
        return false;
    };
    let Some(new_section_table_end) = section_table_end.checked_add(40) else {
        return false;
    };
    if section_table_end > exe.len() {
        return false;
    }
    let first_raw = (0..num_sections)
        .map(|i| read_u32(exe, section_start + i * 40 + 20) as usize)
        .filter(|&offset| offset != 0)
        .min();
    if first_raw.is_some_and(|offset| new_section_table_end > offset) {
        return false;
    }
    let section_alignment = read_u32(exe, opt_start + 32) as usize;
    let file_alignment = read_u32(exe, opt_start + 36) as usize;
    if !section_alignment.is_power_of_two() || !file_alignment.is_power_of_two() {
        return false;
    }

    // Localizar la seccion que contiene la import table (para leer los
    // descriptores existentes y verificar si ya estamos importados).
    let mut rva_base = 0usize;
    let mut file_base = 0usize;
    for i in 0..num_sections {
        let sh = section_start + i * 40;
        let vaddr = read_u32(exe, sh + 12) as usize;
        let vsize = read_u32(exe, sh + 8) as usize;
        let rsize = read_u32(exe, sh + 16) as usize;
        let roff = read_u32(exe, sh + 20) as usize;
        if vaddr <= import_rva && import_rva < vaddr + vsize.max(rsize) {
            rva_base = vaddr;
            file_base = roff;
            break;
        }
    }
    if rva_base == 0 {
        return false;
    }

    let file_off = file_base + (import_rva - rva_base);

    // Recoger los descriptores de import existentes (hasta el null)
    let mut descriptors: Vec<[u8; 20]> = Vec::new();
    let mut pos = file_off;
    loop {
        let desc = &exe[pos..pos + 20];
        if desc.iter().all(|&b| b == 0) {
            break;
        }
        let name_rva = read_u32(exe, pos + 12) as usize;
        let name_fo = file_base + (name_rva - rva_base);
        let dll_bytes: Vec<u8> = exe[name_fo..]
            .iter()
            .take_while(|&&b| b != 0)
            .copied()
            .collect();
        if dll_bytes == dll_name.as_bytes() {
            return true; // ya importado
        }
        descriptors.push(desc.try_into().unwrap());
        pos += 20;
    }
    let num_existing = descriptors.len();

    // ---- Crear la seccion nueva ".mod" al final del archivo ----

    // Nuevo RVA de seccion: al final de la imagen (SizeOfImage alineado).
    let size_of_image = read_u32(exe, opt_start + 56) as usize;
    let new_section_rva = align_up(size_of_image, section_alignment);

    // Nuevo file offset: al final del archivo, alineado a FileAlignment.
    let new_file_off = align_up(exe.len(), file_alignment);

    // Tamano de lo que vamos a escribir en la seccion nueva:
    //   descriptores (num_existing + 1 nuevo + 1 null) + ILT + IAT +
    //   hint/name + nombre del DLL
    let desc_bytes = (num_existing + 1 + 1) * 20;
    let thunk_bytes = 8 + 8 + 2 + function_name.len() + 1 + dll_name.len() + 1;
    let mod_size = desc_bytes + thunk_bytes;
    let mod_raw_size = align_up(mod_size, file_alignment);

    // RVA del array de descriptores en la seccion nueva (file offset == RVA
    // de la seccion porque es nueva y alineada).
    let new_desc_array_rva = new_section_rva;
    let data_start = new_file_off;

    // Extender el archivo con la seccion nueva (rellenando con ceros)
    let total = data_start + mod_raw_size;
    if total > exe.len() {
        exe.resize(total, 0);
    }

    // Copiar descriptores existentes a la nueva ubicacion
    for (i, desc) in descriptors.iter().enumerate() {
        exe[data_start + i * 20..data_start + (i + 1) * 20].copy_from_slice(desc);
    }

    // Escribir descriptor nuevo para nuestro DLL
    let new_desc_pos = data_start + num_existing * 20;
    let null_pos = new_desc_pos + 20;
    for b in exe[null_pos..null_pos + 20].iter_mut() {
        *b = 0;
    }

    let thunk_start = null_pos + 20;

    let ilt_rva = new_section_rva + (thunk_start - data_start);
    let iat_rva = ilt_rva + 8;
    let hintname_rva = iat_rva + 8;
    let name_rva = hintname_rva + 2 + function_name.len() + 1;

    // Hint/Name y nombre del DLL
    let hintname: Vec<u8> = {
        let mut v = Vec::with_capacity(2 + function_name.len() + 1);
        v.extend_from_slice(&[0, 0]); // hint = 0
        v.extend_from_slice(function_name.as_bytes());
        v.push(0);
        v
    };
    let name_bytes = {
        let mut v = Vec::with_capacity(dll_name.len() + 1);
        v.extend_from_slice(dll_name.as_bytes());
        v.push(0);
        v
    };

    // Descriptor nuevo: 5 x u32 (ILT, TimeDateStamp, ForwarderChain, Name, IAT)
    write_u32(exe, new_desc_pos, ilt_rva as u32);
    write_u32(exe, new_desc_pos + 4, 0);
    write_u32(exe, new_desc_pos + 8, 0);
    write_u32(exe, new_desc_pos + 12, name_rva as u32);
    write_u32(exe, new_desc_pos + 16, iat_rva as u32);

    // ILT: una entrada apuntando al hint/name + null
    write_u32(exe, thunk_start, hintname_rva as u32);
    write_u32(exe, thunk_start + 4, 0);
    // IAT: igual al ILT (el loader lo sobreescribe en runtime)
    write_u32(exe, thunk_start + 8, hintname_rva as u32);
    write_u32(exe, thunk_start + 12, 0);

    exe[hintname_rva - new_section_rva + data_start..hintname_rva - new_section_rva + data_start + hintname.len()]
        .copy_from_slice(&hintname);
    exe[name_rva - new_section_rva + data_start..name_rva - new_section_rva + data_start + name_bytes.len()]
        .copy_from_slice(&name_bytes);

    // ---- Anadir el header de la seccion ".mod" ----

    let new_sh = section_start + num_sections * 40;
    // Nombre: ".mod\0\0\0\0"
    exe[new_sh..new_sh + 8].copy_from_slice(b".mod\0\0\0\0");
    write_u32(exe, new_sh + 8, mod_size as u32); // VirtualSize
    write_u32(exe, new_sh + 12, new_section_rva as u32); // VirtualAddress
    write_u32(exe, new_sh + 16, mod_raw_size as u32); // SizeOfRawData
    write_u32(exe, new_sh + 20, data_start as u32); // PointerToRawData
    write_u32(exe, new_sh + 24, 0); // PointerToRelocations
    write_u32(exe, new_sh + 28, 0); // PointerToLinenumbers
    write_u16(exe, new_sh + 32, 0); // NumberOfRelocations
    write_u16(exe, new_sh + 34, 0); // NumberOfLinenumbers
    // Characteristics: IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ |
    //                  IMAGE_SCN_MEM_WRITE = 0x40000040 | 0x80000000 = 0xC0000040
    write_u32(exe, new_sh + 36, 0xC0000040);

    // Actualizar NumberOfSections y SizeOfImage
    write_u16(exe, pe_off + 4 + 2, (num_sections + 1) as u16);
    let new_size_of_image = align_up(new_section_rva + mod_size, section_alignment);
    write_u32(exe, opt_start + 56, new_size_of_image as u32);

    // Actualizar el Import Data Directory para que apunte al nuevo array
    write_u32(exe, dd_rva_offset, new_desc_array_rva as u32);
    write_u32(exe, dd_size_offset, ((num_existing + 1 + 1) * 20) as u32);

    true
}

/// Alinea un valor hacia arriba al multiplo dado.
fn align_up(v: usize, align: usize) -> usize {
    if align <= 1 {
        return v;
    }
    (v + align - 1) & !(align - 1)
}

fn find_exe(dir: &Path) -> Option<PathBuf> {
    let p = dir.join("Patrician3.exe");
    if p.is_file() {
        Some(p)
    } else {
        None
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let dir = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    if !dir.is_dir() {
        println!("[ERROR] No es un directorio: {}", dir.display());
        std::process::exit(1);
    }

    // Decidir entrada/salida
    let original = dir.join("Patrician3_original.exe");
    let (input, output) = if original.is_file() {
        // Modo desarrollo: original -> Patrician3.exe
        (original.clone(), dir.join("Patrician3.exe"))
    } else {
        let exe = find_exe(&dir);
        match exe {
            Some(e) => (e, dir.join("Patrician3_modloader.exe")),
            None => {
                println!("[ERROR] No se encuentra Patrician3.exe en {}", dir.display());
                println!("        Usa: p3-esp-patcher.exe <directorio_del_juego>");
                std::process::exit(1);
            }
        }
    };

    println!("Entrada : {}", input.display());
    println!("Salida  : {}", output.display());

    let mut exe_data = match fs::read(&input) {
        Ok(d) => d,
        Err(e) => {
            println!("[ERROR] No se pudo leer {}: {}", input.display(), e);
            std::process::exit(1);
        }
    };

    // El DLL del modloader debe estar junto al EXE
    let dll_path = dir.join(MODLOADER_DLL);
    if !dll_path.is_file() {
        println!(
            "[ERROR] {} no encontrado junto al EXE; no se genera una salida sin parchear",
            MODLOADER_DLL
        );
        std::process::exit(1);
    }
    if !add_dll_import(&mut exe_data, MODLOADER_DLL, MODLOADER_FUNC) {
        println!("[ERROR] No se pudo anadir el import de {}", MODLOADER_DLL);
        std::process::exit(1);
    }
    println!("[OK] Import de {} anadido", MODLOADER_DLL);

    match fs::write(&output, &exe_data) {
        Ok(_) => println!("[OK] Generado: {}", output.display()),
        Err(e) => {
            println!("[ERROR] No se pudo escribir {}: {}", output.display(), e);
            std::process::exit(1);
        }
    }
}
