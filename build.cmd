@echo off
setlocal EnableExtensions EnableDelayedExpansion
chcp 65001 >nul
cd /d "%~dp0"

rem ============================================================================
rem  Typball - Windows one-click build (MinGW-w64 / GNU toolchain)
rem
rem  Double-click (or run `build.cmd`) to make a release build at dist\Typball.exe.
rem  Missing tools are installed automatically:
rem    Rust            : winget Rustlang.Rustup
rem    C/C++ toolchain : Code::Blocks 25.03 with MinGW, official setup.exe from SourceForge
rem                      (bundled MinGW-w64: gcc/g++, cmake, mingw32-make, ninja, libclang)
rem                      -> %ProgramFiles%\CodeBlocks  (asks for administrator rights)
rem                      winget only has the portable zip of Code::Blocks, so it is not used.
rem                      Without administrator rights the same MinGW is taken from WinLibs
rem                      -> %LOCALAPPDATA%\Typball\winlibs
rem    CMake           : winget Kitware.CMake   (only when the found MinGW lacks cmake.exe)
rem    libclang        : winget LLVM.LLVM       (only when the found MinGW lacks libclang.dll)
rem
rem  A MinGW whose Windows headers are too old for wxWidgets (e.g. GCC 8.1 bundled with
rem  Code::Blocks 20.03 lacks D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT) is skipped, and
rem  Code::Blocks 25.03 is installed instead when no usable one is found.
rem
rem  Environment variables:
rem    TYPBALL_TOOLCHAIN=gnu | msvc   toolchain (default: gnu). msvc uses Visual Studio
rem                                   Build Tools and installs them with winget when missing.
rem    TYPBALL_MINGW_DIR=<path>       directory holding gcc.exe / g++.exe (gnu only)
rem    TYPBALL_PROFILE=debug          debug build (default: release)
rem    TYPBALL_DRY_RUN=1              print install/build commands instead of running them
rem
rem  NOTE: keep this file ASCII-only (messages included). With non-ASCII text, cmd.exe
rem  (chcp 65001) sometimes mis-computes the file offset after a line and executes
rem  fragments of the following lines; where it happens depends on the byte layout of
rem  the whole file, so no placement rule for Japanese text is safe.
rem ============================================================================

set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
set "WINGET_ARGS=--accept-source-agreements --accept-package-agreements --disable-interactivity"
set "CODEBLOCKS_MINGW=%ProgramFiles%\CodeBlocks\MinGW\bin"
set "LLVM_BIN=%ProgramFiles%\LLVM\bin"
set "WINLIBS_DIR=%LOCALAPPDATA%\Typball\winlibs"
set "RUN="
if defined TYPBALL_DRY_RUN set "RUN=echo [dry-run]"
if not defined TYPBALL_PROFILE set "TYPBALL_PROFILE=release"
if not defined TYPBALL_TOOLCHAIN set "TYPBALL_TOOLCHAIN=gnu"

echo.
echo [Typball] Starting the Windows build (profile=%TYPBALL_PROFILE%, toolchain=%TYPBALL_TOOLCHAIN%)
if defined TYPBALL_DRY_RUN echo [Typball] Dry run: install and build commands are printed, not executed
echo.

rem ---------------------------------------------------------------- Rust ----
where rustup >nul 2>nul
if errorlevel 1 (call :install_rust || goto :fail) else echo [Typball] Rust: found

if /i "%TYPBALL_TOOLCHAIN%"=="gnu" goto :setup_gnu
if /i "%TYPBALL_TOOLCHAIN%"=="msvc" goto :setup_msvc
echo [Typball] TYPBALL_TOOLCHAIN must be gnu or msvc.
goto :fail

rem -------------------------------------------------- GNU / MinGW (default) ----
:setup_gnu
call :detect_gcc
if not defined GCC_DIR call :install_mingw || goto :fail
echo [Typball] MinGW: %GCC_DIR%
set "PATH=%GCC_DIR%;%PATH%"
set "CC_x86_64_pc_windows_gnu=%GCC_DIR%\gcc.exe"
set "CXX_x86_64_pc_windows_gnu=%GCC_DIR%\g++.exe"
set "AR_x86_64_pc_windows_gnu=%GCC_DIR%\ar.exe"
set "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=%GCC_DIR%\gcc.exe"

rem wxWidgets is built by CMake with the "MinGW Makefiles" generator -> needs mingw32-make
where mingw32-make >nul 2>nul
if errorlevel 1 if not defined TYPBALL_DRY_RUN call :err_no_make & goto :fail

call :ensure_cmake || goto :fail

if not defined LIBCLANG_PATH if exist "%GCC_DIR%\libclang.dll" set "LIBCLANG_PATH=%GCC_DIR%"
if not defined LIBCLANG_PATH call :ensure_llvm || goto :fail
echo [Typball] LIBCLANG_PATH: %LIBCLANG_PATH%
set "RUST_TOOLCHAIN=stable-x86_64-pc-windows-gnu"
goto :build

rem ------------------------------------------------------- MSVC (opt-in) ----
:setup_msvc
call :detect_msvc
if not defined VSINSTALL call :install_msvc || goto :fail
if defined VSINSTALL echo [Typball] Visual Studio: %VSINSTALL%
if defined VSINSTALL call "%VSINSTALL%\Common7\Tools\VsDevCmd.bat" -arch=x64 -host_arch=x64 -no_logo
if errorlevel 1 goto :fail

call :ensure_cmake || goto :fail
if not defined LIBCLANG_PATH call :ensure_llvm || goto :fail
echo [Typball] LIBCLANG_PATH: %LIBCLANG_PATH%
set "RUST_TOOLCHAIN=stable-x86_64-pc-windows-msvc"
goto :build

rem ---------------------------------------------------------------- build ----
:build
echo [Typball] Checking the Rust toolchain %RUST_TOOLCHAIN%...
rem rustfmt: bindgen formats the generated bindings with it (only a warning when missing)
%RUN% rustup toolchain install %RUST_TOOLCHAIN% --profile minimal --component rustfmt --no-self-update
if errorlevel 1 goto :fail
set "RUSTUP_TOOLCHAIN=%RUST_TOOLCHAIN%"

echo.
echo [Typball] Running cargo build (the first build compiles wxWidgets and takes 10+ minutes)
echo.
set "CARGO_FLAGS=--locked"
set "OUT=target\debug\Typball.exe"
if /i "%TYPBALL_PROFILE%"=="release" set "CARGO_FLAGS=--release --locked"
if /i "%TYPBALL_PROFILE%"=="release" set "OUT=target\release\Typball.exe"
if defined GCC_DIR call :reset_stale_cmake || goto :fail
%RUN% cargo build %CARGO_FLAGS%
if errorlevel 1 goto :fail
if defined TYPBALL_DRY_RUN echo [Typball] Dry run finished
if defined TYPBALL_DRY_RUN goto :end

if not exist dist mkdir dist
copy /y "%OUT%" dist\Typball.exe >nul
echo.
echo [Typball] Done: %~dp0dist\Typball.exe
goto :end

rem ---------------------------------------------------------- subroutines ----
rem (keep this file ASCII-only: see the NOTE at the top)

:reset_stale_cmake
rem CMake keeps the full MinGW paths in its cache under target\<profile>. When the
rem MinGW changes (for example WinLibs -> Code::Blocks), that cache still points
rem at the old compiler, so remove the CMake build dirs and configure again.
set "CMAKE_DIR=target\debug"
if /i "%TYPBALL_PROFILE%"=="release" set "CMAKE_DIR=target\release"
set "CMAKE_CACHE=%CMAKE_DIR%\wxdragon_sys_cmake_build\build\CMakeCache.txt"
if not exist "%CMAKE_CACHE%" exit /b 0
set "CMAKE_GCC=%GCC_DIR:\=/%"
findstr /i /b /l /c:"CMAKE_AR:FILEPATH=%CMAKE_GCC%/" "%CMAKE_CACHE%" >nul && exit /b 0
echo [Typball] The last build used another MinGW. Removing its CMake files in %CMAKE_DIR%
echo [Typball] (wxWidgets is compiled again, 10+ minutes).
%RUN% rmdir /s /q "%CMAKE_DIR%\wxdragon_sys_cmake_build"
if exist "%CMAKE_DIR%\wxwidgets_cmake_build" %RUN% rmdir /s /q "%CMAKE_DIR%\wxwidgets_cmake_build"
if defined TYPBALL_DRY_RUN exit /b 0
if not exist "%CMAKE_CACHE%" exit /b 0
echo [Typball] Could not remove %CMAKE_DIR%\wxdragon_sys_cmake_build. Close programs using it and retry.
exit /b 1

:install_rust
call :need_winget || exit /b 1
echo [Typball] Rust not found. Installing Rustup with winget...
%RUN% winget install -e --id Rustlang.Rustup %WINGET_ARGS%
if errorlevel 1 exit /b 1
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where rustup >nul 2>nul
if not errorlevel 1 exit /b 0
if defined TYPBALL_DRY_RUN exit /b 0
echo [Typball] rustup still not found. Open a new terminal and run build.cmd again.
exit /b 1

:install_mingw
if defined OLD_GCC_DIR echo [Typball] Every MinGW found is too old. Setting up a new one.
echo [Typball] No usable MinGW (gcc/g++) found. Installing Code::Blocks 25.03 with MinGW.
rem an old Code::Blocks must go first: installing over it would mix in its old MinGW files
if defined OLD_CODEBLOCKS call :remove_old_codeblocks
if not defined OLD_CODEBLOCKS call :install_codeblocks
if defined TYPBALL_DRY_RUN set "GCC_DIR=%CODEBLOCKS_MINGW%"
if not defined TYPBALL_DRY_RUN call :detect_gcc
if defined GCC_DIR exit /b 0
rem no administrator rights (or the old Code::Blocks was kept): use the same MinGW that
rem Code::Blocks 25.03 bundles, from WinLibs (also used by CI); it needs no installer
echo [Typball] Downloading the same MinGW from WinLibs instead (no administrator rights needed).
call :install_winlibs || exit /b 1
call :detect_gcc
if defined GCC_DIR exit /b 0
if defined TYPBALL_DRY_RUN set "GCC_DIR=%WINLIBS_DIR%\mingw64\bin"
if defined GCC_DIR exit /b 0
echo [Typball] Could not find a usable MinGW after installing.
echo           Set TYPBALL_MINGW_DIR to the bin folder of a recent MinGW-w64 (e.g. GCC 14 from Code::Blocks 25.03 / WinLibs) and retry.
exit /b 1

:install_codeblocks
rem Official Code::Blocks 25.03 setup with MinGW (NSIS; /S = silent, default folder
rem %ProgramFiles%\CodeBlocks). SHA-256 from the release's checkums.txt on SourceForge.
rem A verified setup.exe from an earlier run is reused (it is 423 MB).
set "CB_SETUP_URL=https://downloads.sourceforge.net/project/codeblocks/Binaries/25.03/Windows/codeblocks-25.03mingw-setup.exe"
set "CB_SETUP_SHA256=8712227526eb3bb26c90dd5c78301b3fa32bf5869a43294bbf4e9c5512c52792"
set "CB_SETUP=%TEMP%\typball_codeblocks-25.03mingw-setup.exe"
echo           From: %CB_SETUP_URL%
echo           To  : %ProgramFiles%\CodeBlocks  (Windows asks for administrator rights)
if defined TYPBALL_DRY_RUN echo [dry-run] curl.exe -L --fail -o "%CB_SETUP%" "%CB_SETUP_URL%"
if defined TYPBALL_DRY_RUN echo [dry-run] "%CB_SETUP%" /S  (as administrator)
if defined TYPBALL_DRY_RUN exit /b 0
if exist "%CB_SETUP%" call :check_sha256 "%CB_SETUP%" %CB_SETUP_SHA256% || del "%CB_SETUP%" >nul 2>nul
if exist "%CB_SETUP%" echo [Typball] Using the setup.exe downloaded before.
if not exist "%CB_SETUP%" curl.exe -L --fail --progress-bar -o "%CB_SETUP%" "%CB_SETUP_URL%"
if errorlevel 1 echo [Typball] Failed to download the Code::Blocks setup.
if errorlevel 1 goto :install_codeblocks_fail
call :check_sha256 "%CB_SETUP%" %CB_SETUP_SHA256%
if errorlevel 1 echo [Typball] Code::Blocks setup SHA-256 mismatch.
if errorlevel 1 goto :install_codeblocks_fail
echo [Typball] Installing Code::Blocks 25.03 (silent, takes a few minutes)...
rem Start-Process -Verb RunAs shows the UAC prompt; a refused prompt throws (-> 1223)
powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "try { $p = Start-Process -FilePath $env:CB_SETUP -ArgumentList '/S' -Verb RunAs -Wait -PassThru -ErrorAction Stop; exit $p.ExitCode } catch { exit 1223 }"
if errorlevel 1223 if not errorlevel 1224 echo [Typball] Administrator rights were not given. Code::Blocks is not installed.
if errorlevel 1223 if not errorlevel 1224 exit /b 1
if errorlevel 1 echo [Typball] The Code::Blocks setup failed (exit code %errorlevel%).
if errorlevel 1 exit /b 1
del "%CB_SETUP%" >nul 2>nul
exit /b 0
:install_codeblocks_fail
del "%CB_SETUP%" >nul 2>nul
exit /b 1

:check_sha256
rem %1 = file, %2 = expected SHA-256 (hex)
certutil -hashfile "%~1" SHA256 | "%SystemRoot%\System32\findstr.exe" /i /x /c:"%~2" >nul
exit /b %errorlevel%

:remove_old_codeblocks
rem Code::Blocks installed by setup.exe (e.g. 20.03 with GCC 8.1): its NSIS uninstaller
rem runs silently with /S. Asks first; Y after 30 s, N keeps it (WinLibs is used instead).
set "CB_DIR=%ProgramFiles%\CodeBlocks"
if not exist "%CB_DIR%\uninstall.exe" exit /b 0
echo [Typball] The old Code::Blocks in %CB_DIR% has to be removed before installing 25.03.
echo           It will be uninstalled and replaced by 25.03. Windows may ask for administrator rights.
if defined TYPBALL_DRY_RUN echo [dry-run] "%CB_DIR%\uninstall.exe" /S
if defined TYPBALL_DRY_RUN exit /b 0
echo           Press N within 30 seconds to keep it (a separate WinLibs MinGW is downloaded instead).
choice /c YN /t 30 /d Y /m "[Typball] Replace Code::Blocks"
if errorlevel 2 echo [Typball] Keeping Code::Blocks.
if errorlevel 2 exit /b 0
"%CB_DIR%\uninstall.exe" /S
if errorlevel 1 echo [Typball] Could not start the Code::Blocks uninstaller. Keeping Code::Blocks.
if errorlevel 1 exit /b 0
rem the NSIS uninstaller copies itself to %TEMP% and returns at once: wait for the files to go
for /l %%i in (1,1,120) do if exist "%CB_DIR%\codeblocks.exe" ping -n 2 127.0.0.1 >nul
if exist "%CB_DIR%\codeblocks.exe" echo [Typball] Code::Blocks is still installed. Keeping it.
if exist "%CB_DIR%\codeblocks.exe" exit /b 0
echo [Typball] Uninstalled the old Code::Blocks.
set "OLD_CODEBLOCKS="
exit /b 0

:install_winlibs
rem keep in sync with .github/actions/setup-mingw/action.yml
set "WINLIBS_URL=https://github.com/brechtsanders/winlibs_mingw/releases/download/14.2.0posix-12.0.0-ucrt-r3/winlibs-x86_64-posix-seh-gcc-14.2.0-llvm-19.1.7-mingw-w64ucrt-12.0.0-r3.zip"
set "WINLIBS_SHA256=982be78644b2fe40187e11b209333d58d3cf17be87d87be62a7bc24f18040594"
set "WINLIBS_ZIP=%TEMP%\typball_winlibs_%RANDOM%.zip"
echo           From: %WINLIBS_URL%
echo           To  : %WINLIBS_DIR%
if defined TYPBALL_DRY_RUN echo [dry-run] curl.exe -L --fail -o "%WINLIBS_ZIP%" "%WINLIBS_URL%"
if defined TYPBALL_DRY_RUN echo [dry-run] tar.exe -xf "%WINLIBS_ZIP%" -C "%WINLIBS_DIR%"
if defined TYPBALL_DRY_RUN exit /b 0
curl.exe -L --fail --progress-bar -o "%WINLIBS_ZIP%" "%WINLIBS_URL%"
if errorlevel 1 echo [Typball] Failed to download WinLibs.
if errorlevel 1 goto :install_winlibs_fail
call :check_sha256 "%WINLIBS_ZIP%" %WINLIBS_SHA256%
if errorlevel 1 echo [Typball] WinLibs SHA-256 mismatch.
if errorlevel 1 goto :install_winlibs_fail
if exist "%WINLIBS_DIR%" rmdir /s /q "%WINLIBS_DIR%"
mkdir "%WINLIBS_DIR%"
tar.exe -xf "%WINLIBS_ZIP%" -C "%WINLIBS_DIR%"
if errorlevel 1 echo [Typball] Failed to extract WinLibs.
if errorlevel 1 goto :install_winlibs_fail
del "%WINLIBS_ZIP%" >nul 2>nul
exit /b 0
:install_winlibs_fail
del "%WINLIBS_ZIP%" >nul 2>nul
exit /b 1

:err_no_make
echo [Typball] mingw32-make.exe not found. Install the MinGW bundled with Code::Blocks 25.03
echo           or MSYS2's mingw-w64-ucrt-x86_64-toolchain.
exit /b 0

:install_msvc
call :need_winget || exit /b 1
echo [Typball] Installing Visual Studio 2022 Build Tools (C++ workload).
echo           This downloads several GB and asks for administrator rights (UAC)...
%RUN% winget install -e --id Microsoft.VisualStudio.2022.BuildTools %WINGET_ARGS% --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
call :detect_msvc
if defined VSINSTALL exit /b 0
if defined TYPBALL_DRY_RUN exit /b 0
echo [Typball] Could not find Build Tools after installing.
exit /b 1

:ensure_cmake
where cmake >nul 2>nul
if not errorlevel 1 exit /b 0
if exist "%ProgramFiles%\CMake\bin\cmake.exe" set "PATH=%ProgramFiles%\CMake\bin;%PATH%" & exit /b 0
call :need_winget || exit /b 1
echo [Typball] cmake not found. Installing CMake with winget...
%RUN% winget install -e --id Kitware.CMake %WINGET_ARGS%
if exist "%ProgramFiles%\CMake\bin\cmake.exe" set "PATH=%ProgramFiles%\CMake\bin;%PATH%" & exit /b 0
if defined TYPBALL_DRY_RUN exit /b 0
echo [Typball] cmake not found.
exit /b 1

:ensure_llvm
if exist "%LLVM_BIN%\libclang.dll" set "LIBCLANG_PATH=%LLVM_BIN%" & exit /b 0
call :need_winget || exit /b 1
echo [Typball] libclang.dll not found. Installing LLVM with winget...
%RUN% winget install -e --id LLVM.LLVM %WINGET_ARGS%
if exist "%LLVM_BIN%\libclang.dll" set "LIBCLANG_PATH=%LLVM_BIN%" & exit /b 0
if defined TYPBALL_DRY_RUN set "LIBCLANG_PATH=%LLVM_BIN%" & exit /b 0
echo [Typball] libclang.dll not found. Install LLVM and set LIBCLANG_PATH.
exit /b 1

:need_winget
where winget >nul 2>nul
if not errorlevel 1 exit /b 0
echo [Typball] winget not found. Install "App Installer" from the Microsoft Store.
exit /b 1

:detect_gcc
rem Every candidate goes through :try_gcc, which takes the first MinGW whose headers
rem are new enough for wxWidgets and remembers the skipped ones in OLD_GCC_DIR.
set "GCC_DIR="
set "OLD_GCC_DIR="
set "OLD_CODEBLOCKS="
call :write_gcc_probe
if defined TYPBALL_MINGW_DIR call :try_mingw_dir_env
rem Code::Blocks setup.exe (25.03 is fine; 20.03 bundles GCC 8.1 and is skipped)
call :try_gcc "%CODEBLOCKS_MINGW%"
rem WinLibs downloaded by :install_winlibs
call :try_gcc "%WINLIBS_DIR%\mingw64\bin"
rem Code::Blocks portable zip installed by hand with winget (machine scope / user scope)
for /d %%p in ("%ProgramFiles%\WinGet\Packages\CodeBlocks.CodeBlocks.MinGW_*" "%LOCALAPPDATA%\Microsoft\WinGet\Packages\CodeBlocks.CodeBlocks.MinGW_*") do (
    call :try_gcc "%%~p\MinGW\bin"
    for /d %%q in ("%%~p\*") do call :try_gcc "%%~q\MinGW\bin"
)
rem WinLibs portable zip installed by winget (BrechtSanders.WinLibs.*): same build as Code::Blocks' MinGW
for /d %%p in ("%ProgramFiles%\WinGet\Packages\BrechtSanders.WinLibs*" "%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs*") do (
    call :try_gcc "%%~p\mingw64\bin"
    for /d %%q in ("%%~p\*") do call :try_gcc "%%~q\mingw64\bin"
)
rem other common MinGW-w64 locations
for %%d in ("C:\msys64\ucrt64\bin" "C:\msys64\mingw64\bin" "C:\mingw64\bin" "C:\mingw64\mingw64\bin" "C:\winlibs\mingw64\bin") do call :try_gcc "%%~d"
for /f "delims=" %%i in ('where g++ 2^>nul') do call :try_gcc "%%~dpi."
del "%GCC_PROBE%" >nul 2>nul
exit /b 0

:try_mingw_dir_env
rem An explicitly given TYPBALL_MINGW_DIR is used even when the probe fails (with a warning)
if not exist "%TYPBALL_MINGW_DIR%\g++.exe" echo [Typball] TYPBALL_MINGW_DIR has no g++.exe: %TYPBALL_MINGW_DIR%
if not exist "%TYPBALL_MINGW_DIR%\g++.exe" exit /b 0
call :normalize_dir "%TYPBALL_MINGW_DIR%"
set "GCC_DIR=%TRY_DIR%"
call :probe_gcc "%GCC_DIR%" && exit /b 0
echo [Typball] Warning: the MinGW in TYPBALL_MINGW_DIR has old Windows headers; building wxWidgets may fail.
exit /b 0

:try_gcc
if defined GCC_DIR exit /b 0
if not exist "%~1\g++.exe" exit /b 0
call :normalize_dir "%~1"
call :probe_gcc "%TRY_DIR%" && set "GCC_DIR=%TRY_DIR%" && exit /b 0
if defined OLD_GCC_DIR if /i "%OLD_GCC_DIR%"=="%TRY_DIR%" exit /b 0
set "OLD_GCC_VER=?"
"%TRY_DIR%\g++.exe" -dumpversion > "%GCC_PROBE%.ver" 2>nul
set /p OLD_GCC_VER=<"%GCC_PROBE%.ver"
del "%GCC_PROBE%.ver" >nul 2>nul
echo [Typball] Skipping MinGW with too old Windows headers (GCC %OLD_GCC_VER%): %TRY_DIR%
set "OLD_GCC_DIR=%TRY_DIR%"
if /i "%TRY_DIR%"=="%CODEBLOCKS_MINGW%" set "OLD_CODEBLOCKS=1"
exit /b 0

:normalize_dir
rem TRY_DIR = full path of %1 without a trailing backslash
set "TRY_DIR=%~f1"
if "%TRY_DIR:~-1%"=="\" set "TRY_DIR=%TRY_DIR:~0,-1%"
exit /b 0

:write_gcc_probe
rem A tiny C++ file that only compiles with Windows headers new enough for wxWidgets'
rem src\msw\graphicsd2d.cpp (mingw-w64 bundled with GCC 8.1 fails here).
set "GCC_PROBE=%TEMP%\typball_gcc_probe_%RANDOM%.cpp"
> "%GCC_PROBE%" echo #include ^<windows.h^>
>> "%GCC_PROBE%" echo #include ^<d2d1_1.h^>
>> "%GCC_PROBE%" echo int typball_probe = D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT;
exit /b 0

:probe_gcc
rem errorlevel 0 when %1\g++.exe compiles the probe
setlocal
set "PATH=%~1;%PATH%"
"%~1\g++.exe" -fsyntax-only -x c++ "%GCC_PROBE%" >nul 2>nul
endlocal & exit /b %errorlevel%

:detect_msvc
set "VSINSTALL="
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if exist "%VSWHERE%" for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSINSTALL=%%i"
exit /b 0

rem ------------------------------------------------------------------ exit ----
:fail
echo.
echo [Typball] Build failed.
call :pause_if_doubleclicked
exit /b 1

:end
call :pause_if_doubleclicked
exit /b 0

:pause_if_doubleclicked
rem pause before the window closes when started by double-click (cmd /c)
echo %cmdcmdline% | "%SystemRoot%\System32\find.exe" /i "/c" >nul && pause
exit /b 0
