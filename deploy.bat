@echo off
setlocal
@REM Nos aseguramos de ejecutarnos desde el directorio del script (donde estan
@REM los crates), para no depender de donde se abra.
cd /d "%~dp0"

echo === Compilando DLLs y patcher (Windows nativo) ===
echo.

rustup target add i686-pc-windows-msvc 2>nul

@REM Cada crate se compila APUNTANDO A SU PROPIO Cargo.toml (--manifest-path).
@REM Asi NO dependemos de que el Cargo.toml del workspace raiz este presente o
@REM bien copiado (problema de WinSCP al copiar el anfitrion -> VM). cargo
@REM resuelve la dependencia "path" de p3-esp-modlib igualmente.
set MANIFESTS=p3-esp-modlib p3-esp-modloader mod-catedral-crash mod-fundacion mod-fullhd mod-limite-ciudades mod-mendigos-taberna mod-refresco-misiones mod-satisfaccion-mendigos p3-esp-dll-patcher
for %%M in (%MANIFESTS%) do (
    echo.
    echo --- Compilando %%M ---
    cargo build --release --target i686-pc-windows-msvc --manifest-path "%~dp0%%M\Cargo.toml"
    if %errorlevel% neq 0 goto :error
)

@REM Limpiamos el output para que no queden DLLs obsoletas de compilaciones
@REM anteriores (especialmente las de la cada vez mayor lista de mods)
if exist output rmdir /s /q output
mkdir output\mods

@REM En la raiz de output solo van el parcheador y el modloader (ambos se
@REM copian a la carpeta del juego); los mods van a output\mods
copy /Y target\i686-pc-windows-msvc\release\p3_esp_modloader.dll output\ >nul
copy /Y target\i686-pc-windows-msvc\release\p3_esp_dll_patcher.exe output\ >nul

copy /Y target\i686-pc-windows-msvc\release\mod_catedral_crash.dll output\mods\ >nul
copy /Y target\i686-pc-windows-msvc\release\mod_fundacion.dll output\mods\ >nul
copy /Y target\i686-pc-windows-msvc\release\mod_fullhd.dll output\mods\ >nul
copy /Y target\i686-pc-windows-msvc\release\mod_limite_ciudades.dll output\mods\ >nul
copy /Y target\i686-pc-windows-msvc\release\mod_mendigos_taberna.dll output\mods\ >nul
copy /Y target\i686-pc-windows-msvc\release\mod_refresco_misiones.dll output\mods\ >nul
copy /Y target\i686-pc-windows-msvc\release\mod_satisfaccion_mendigos.dll output\mods\ >nul

echo.
echo === Generados ===
dir output\
echo.
echo === Mods (en output\mods) ===
dir output\mods\
echo.
echo Para instalar:
echo   1. Genera el EXE parcheado:
echo        - Normal: abre una consola en la carpeta del juego y ejecuta:
echo          output\p3_esp_dll_patcher.exe (genera Patrician3_modloader.exe)
echo        - O si tienes Patrician3_original.exe junto al juego, genera Patrician3.exe
echo   2. Copia output\p3_esp_modloader.dll a la carpeta del juego
echo   3. Copia a la carpeta del juego los mods que quieras de output\mods,
echo      metiendolos dentro de la subcarpeta mods\ del juego (o creandola):
echo        mod_catedral_crash.dll        - fix catedral
echo        mod_fundacion.dll              - ciudad fundada: forzar productos (1) o 3 correctos (0)
echo        mod_fullhd.dll                 - resolucion 1920x1080
echo        mod_limite_ciudades.dll        - limite 36 ciudades
echo        mod_refresco_misiones.dll      - misiones de gobernador mas rapidas
echo        mod_mendigos_taberna.dll       - taberna hasta 100 marineros
echo        mod_satisfaccion_mendigos.dll  - satisfaccion mendigos
echo   4. Crea p3_esp_mods.cfg en la carpeta del juego con:
echo        [fundacion]
echo        forzarProductos=1
echo        productos=grano,madera,cerveza,vino,miel
echo   5. Ejecuta Patrician3_modloader.exe (o Patrician3.exe)
echo   NOTA: ya no hace falta p3-patcher.py ni Python
goto :end

:error
echo ERROR: Fallo la compilacion.
echo Asegurate de tener Rust: https://rustup.rs
:end
pause
