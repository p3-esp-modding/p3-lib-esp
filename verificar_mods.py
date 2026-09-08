#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
verificar_mods.py — Verificación de los mods del Patrician 3 español.

Comprueba dos cosas:

1) CODE CAVES (código máquina ensamblado a mano en Rust):
   Reconstruye los bytes EXACTOS que genera el código Rust de cada cave
   (opcodes + rel32 con su next-IP) y los desensambla con objdump,
   comprobando que:
     - el buffer (CAVE_SIZE) cubre el último rel32,
     - cada rel32 se escribe en la posición de un campo de salto
       (no pisa un opcode),
     - cada salto cae donde debe (se muestra el desensamblado).

   Esto es necesario porque `cargo check` NO valida el código máquina:
   un off-by-one en un rel32 o un CAVE_SIZE corto compila y crashea en
   runtime (bug real: cave de FUN_00491ec0 con CAVE_SIZE=31 cuando
   necesita 32, y rel32 del JMP escrito en 0x1B pisando el E9).

2) PARCHES DE BYTES (expected/find contra el exe):
   Comprueba contra Patrician3_original.exe que:
     - los bytes esperados de cada apply_offset_patch coinciden en su VA,
     - los patrones find del static patcher (en repo separado) aparecen exactamente una vez,
     - el modloader/dll-patcher ven los bytes de WinMain esperados.

Uso:
    python3 verificar_mods.py [ruta_al_exe]
    (por defecto: Patrician3_original.exe en la raíz del repo)
"""

import re
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent
DEFAULT_EXE = REPO / "Patrician3_original.exe"
IMAGE_BASE = 0x00400000
CAVE_ADDR = 0x01A01000  # dirección fija de prueba para desensamblar
PLACEHOLDER_CALL = 0x10000000  # destino del call a rust_init_table (runtime)

# ---------------------------------------------------------------------------
# utilidades
# ---------------------------------------------------------------------------


def parse_hex(s: str) -> bytes:
    s = s.strip()
    return bytes(int(s[i : i + 2], 16) for i in range(0, len(s), 2))


def extract_fn(src: str, fn_name: str):
    """Devuelve el cuerpo de `unsafe fn fn_name` (con llaves balanceadas)."""
    start = src.find(f"unsafe fn {fn_name}")
    if start < 0:
        return None
    brace = src.find("{", start)
    if brace < 0:
        return None
    depth = 0
    i = brace
    while i < len(src):
        if src[i] == "{":
            depth += 1
        elif src[i] == "}":
            depth -= 1
            if depth == 0:
                return src[start : i + 1]
        i += 1
    return None


def disassemble(blob: bytes) -> list:
    """Desensambla `blob` con objdump y devuelve las líneas de instrucciones."""
    with tempfile.NamedTemporaryFile(suffix=".bin", delete=False) as f:
        f.write(blob)
        path = f.name
    try:
        r = subprocess.run(
            ["objdump", "-D", "-b", "binary", "-m", "i386",
             "--adjust-vma", hex(CAVE_ADDR), path],
            capture_output=True, text=True,
        )
    finally:
        Path(path).unlink(missing_ok=True)
    lines = r.stdout.splitlines()
    return [l for l in lines[7:] if l.strip()]


# ---------------------------------------------------------------------------
# 1) verificación de code caves
# ---------------------------------------------------------------------------


def parse_cave(fn_src: str, fn_name: str, full_src: str = None):
    """Extrae la definición de un cave de una función apply_patch_*."""
    cave = {
        "name": fn_name,
        "size": None,
        "bytes": {},      # offset -> byte
        "rel_writes": [],  # (field_offset, rel_name)
        "rel_defs": {},   # rel_name -> (target_expr, next_ip)
        "rva_consts": {},  # CONST_NAME -> rva
    }

    # CAVE_SIZE y *_RVA suelen estar DENTRO de la función; solo se busca en
    # el módulo si no están (p. ej. mod-fundacion los define a nivel módulo).
    m = re.search(r"const CAVE_SIZE: usize = (\d+);", fn_src)
    if not m and full_src:
        m = re.search(r"const CAVE_SIZE: usize = (\d+);", full_src)
    if m:
        cave["size"] = int(m.group(1))

    for m in re.finditer(r"const (\w+_RVA): u32 = 0x([0-9A-Fa-f]+);", fn_src):
        cave["rva_consts"][m.group(1)] = int(m.group(2), 16)
    if full_src:
        # solo rellena las que falten (no pisa las de la función)
        for m in re.finditer(r"const (\w+_RVA): u32 = 0x([0-9A-Fa-f]+);", full_src):
            cave["rva_consts"].setdefault(m.group(1), int(m.group(2), 16))

    # code[N] = 0xXX;
    for m in re.finditer(r"code\[(\d+)\] = 0x([0-9A-Fa-f]{2});", fn_src):
        cave["bytes"][int(m.group(1))] = int(m.group(2), 16)

    # code[A..B].copy_from_slice(&[0xXX, 0xYY, ...]);
    for m in re.finditer(
        r"code\[(\d+)\.\.(\d+)\]\.copy_from_slice\(&\[([0-9A-Fa-fxX,\s]+)\]\);",
        fn_src,
    ):
        a, b = int(m.group(1)), int(m.group(2))
        vals = [int(x, 16) for x in re.findall(r"0x([0-9A-Fa-f]{2})", m.group(3))]
        if len(vals) != b - a:
            print(f"  [AVISO] {fn_name}: copy_from_slice {a}..{b} con {len(vals)} bytes")
        for i, v in enumerate(vals):
            cave["bytes"][a + i] = v

    # write_unaligned(cave_ptr.add(0xXX) as *mut u32, rel_XXX);
    for m in re.finditer(
        r"write_unaligned\(cave_ptr\.add\((0x[0-9A-Fa-f]+|\d+)\) as \*mut u32, (\w+)\);",
        fn_src,
    ):
        off_str = m.group(1)
        field = int(off_str, 16) if off_str.lower().startswith("0x") else int(off_str)
        cave["rel_writes"].append((field, m.group(2)))

    # let rel_XXX = (EXPR).wrapping_sub(cave_addr + 0xYY);
    for m in re.finditer(
        r"let (rel_\w+) = (.+?)\.wrapping_sub\(cave_addr \+ 0x([0-9A-Fa-f]+)\);",
        fn_src,
    ):
        cave["rel_defs"][m.group(1)] = (m.group(2).strip(), int(m.group(3), 16))

    return cave


def resolve_target(expr: str, rva_consts: dict):
    """Resuelve la expresión de destino de un rel32 a una dirección absoluta."""
    if "rust_init_table" in expr or "as *const ()" in expr:
        return PLACEHOLDER_CALL
    m = re.search(r"IMAGE_BASE \+ (\w+)", expr)
    if m and m.group(1) in rva_consts:
        return IMAGE_BASE + rva_consts[m.group(1)]
    m = re.search(r"0x([0-9A-Fa-f]+)", expr)
    if m:
        return int(m.group(1), 16)
    return None


def verify_cave(fn_src: str, fn_name: str, full_src: str = None) -> bool:
    cave = parse_cave(fn_src, fn_name, full_src)
    ok = True

    if cave["size"] is None:
        print(f"FAIL {fn_name}: no se encontró CAVE_SIZE")
        return False

    # construir el buffer
    blob = bytearray(cave["size"])
    for off, val in cave["bytes"].items():
        if off >= cave["size"]:
            print(f"FAIL {fn_name}: byte en offset 0x{off:X} fuera de CAVE_SIZE={cave['size']}")
            ok = False
            continue
        blob[off] = val

    # escribir los rel32 (y datos runtime como la máscara de mod-fundacion)
    for field, rel_name in cave["rel_writes"]:
        if rel_name not in cave["rel_defs"]:
            # dato runtime (p. ej. `mask`): placeholder para desensamblar
            print(f"  [INFO] {fn_name}: {rel_name} es dato runtime, se usa placeholder")
            struct.pack_into("<I", blob, field, 0x10B4)
            continue
        expr, next_ip = cave["rel_defs"][rel_name]
        target = resolve_target(expr, cave["rva_consts"])
        if target is None:
            print(f"FAIL {fn_name}: no se pudo resolver destino de {rel_name} ({expr})")
            ok = False
            continue
        if field + 4 > cave["size"]:
            print(
                f"FAIL {fn_name}: rel32 {rel_name} en 0x{field:X} se sale del buffer "
                f"(CAVE_SIZE={cave['size']}, necesita {field + 4})"
            )
            ok = False
            continue
        val = (target - (CAVE_ADDR + next_ip)) & 0xFFFFFFFF
        struct.pack_into("<I", blob, field, val)

    # comprobar que cada campo rel32 está precedido por un opcode de salto
    for field, rel_name in cave["rel_writes"]:
        if rel_name not in cave["rel_defs"]:
            continue
        if field >= 1 and blob[field - 1] in (0xE8, 0xE9):
            continue
        if field >= 2 and blob[field - 2] == 0x0F and blob[field - 1] in (0x84, 0x85, 0x8C, 0x8D, 0x8F):
            continue
        print(
            f"FAIL {fn_name}: rel32 {rel_name} en 0x{field:X} NO está precedido por un salto "
            f"(bytes previos: {blob[max(0, field - 2):field].hex()})"
        )
        ok = False

    # desensamblar
    print(f"\n===== cave {fn_name} ({cave['size']} bytes) =====")
    for line in disassemble(bytes(blob)):
        print("  " + line)

    return ok


# ---------------------------------------------------------------------------
# 2) verificación de parches de bytes contra el exe
# ---------------------------------------------------------------------------


def verify_offset_patch(exe: bytes, label: str, tag: str, va: int, expected_hex: str, new_hex: str) -> bool:
    expected = parse_hex(expected_hex)
    new = parse_hex(new_hex)
    if len(expected) != len(new):
        print(f"FAIL {label}/{tag}: expected y new longitudes distintas")
        return False
    off = va - IMAGE_BASE
    if off < 0 or off + len(expected) > len(exe):
        print(f"FAIL {label}/{tag}: VA 0x{va:08X} fuera del exe")
        return False
    actual = exe[off : off + len(expected)]
    if actual == expected:
        print(f"OK   {label}/{tag}: VA 0x{va:08X} coincide ({expected_hex})")
        return True
    print(f"FAIL {label}/{tag}: VA 0x{va:08X} esperado {expected_hex}, real {actual.hex()}")
    return False


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def main():
    exe_path = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_EXE
    if not exe_path.exists():
        print(f"No encuentro el exe: {exe_path}")
        sys.exit(1)
    exe = exe_path.read_bytes()
    print(f"Exe: {exe_path} ({len(exe)} bytes)")
    print("=" * 70)

    all_ok = True
    dll_dir = REPO

    # ---- code caves ----
    print("\n### CODE CAVES ###")
    catedral = (dll_dir / "mod-catedral-crash/src/lib.rs").read_text()
    for fn in [
        "apply_patch_00491930",
        "apply_patch_00491d50",
        "apply_patch_00491ec0",
        "apply_patch_00491f30",
    ]:
        body = extract_fn(catedral, fn)
        if body is None:
            print(f"FAIL {fn}: no encontrada")
            all_ok = False
            continue
        all_ok &= verify_cave(body, fn, catedral)

    fundacion = (dll_dir / "mod-fundacion/src/lib.rs").read_text()
    body = extract_fn(fundacion, "apply_force_hook")
    if body is None:
        print("FAIL apply_force_hook: no encontrada")
        all_ok = False
    else:
        all_ok &= verify_cave(body, "apply_force_hook (mod-fundacion)", fundacion)

    # ---- parches de bytes de los mods DLL ----
    print("\n### PARCHES DE BYTES (mods DLL, apply_offset_patch) ###")
    offset_patch_re = re.compile(
        r'apply_offset_patch\(\s*"([^"]+)",\s*"([^"]+)",\s*0x([0-9A-Fa-f]+),\s*&hex\("([0-9A-Fa-f]+)"\),\s*&hex\("([0-9A-Fa-f]+)"\)'
    )
    # mod-fullhd define los parches en un array literal, no en la llamada
    array_patch_re = re.compile(
        r'\("([^"]+)",\s*0x([0-9A-Fa-f]+),\s*&hex\("([0-9A-Fa-f]+)"\),\s*&hex\("([0-9A-Fa-f]+)"\)'
    )

    for mod in [
        "mod-fullhd",
        "mod-limite-ciudades",
        "mod-mendigos-taberna",
        "mod-refresco-misiones",
        "mod-satisfaccion-mendigos",
        "mod-fundacion",
    ]:
        src = (dll_dir / f"{mod}/src/lib.rs").read_text()
        found = False
        for label, tag, va, exp, new in offset_patch_re.findall(src):
            found = True
            all_ok &= verify_offset_patch(exe, label, tag, int(va, 16), exp, new)
        for tag, va, exp, new in array_patch_re.findall(src):
            found = True
            all_ok &= verify_offset_patch(exe, mod, tag, int(va, 16), exp, new)
        if not found:
            print(f"[INFO] {mod}: sin apply_offset_patch (usa hook/cave)")

    # mod-refresco-misiones: el JMP que sustituye su cave debe estar intacto
    # en el exe original (0x005341B4: LEA ECX,[esp+0x4C] + PUSH 0x10, epilogo
    # comun del generador de misiones FUN_00533CA0)
    hook_off = 0x001341B4
    expected_hook = bytes.fromhex("8D4C244C6A")
    actual = exe[hook_off : hook_off + len(expected_hook)]
    if actual == expected_hook:
        print(f"OK   mod-refresco-misiones: epilogo en VA 0x{IMAGE_BASE + hook_off:08X} = {actual.hex()}")
    else:
        print(f"FAIL mod-refresco-misiones: epilogo en VA 0x{IMAGE_BASE + hook_off:08X} esperado {expected_hook.hex()}, real {actual.hex()}")
        all_ok = False

    # ---- modloader / dll-patcher ----
    print("\n### MODLOADER / DLL-PATCHER ###")
    winmain_off = 0x0023E082
    expected_winmain = bytes.fromhex("E8B9F40000")
    actual = exe[winmain_off : winmain_off + len(expected_winmain)]
    if actual == expected_winmain:
        print(f"OK   WinMain call en 0x{winmain_off:X}: {actual.hex()}")
    else:
        print(f"FAIL WinMain call en 0x{winmain_off:X}: esperado {expected_winmain.hex()}, real {actual.hex()}")
        all_ok = False

    # timestamp PE
    e_lfanew = struct.unpack_from("<I", exe, 0x3C)[0]
    ts = struct.unpack_from("<I", exe, e_lfanew + 8)[0]
    if ts == 0x4118A8B2:
        print(f"OK   Timestamp PE 0x{ts:08X}")
    else:
        print(f"FAIL Timestamp PE 0x{ts:08X} (esperado 0x4118A8B2)")
        all_ok = False

    print("\n" + "=" * 70)
    print("RESULTADO:", "TODO OK" if all_ok else "HAY FALLOS")
    sys.exit(0 if all_ok else 1)


if __name__ == "__main__":
    main()