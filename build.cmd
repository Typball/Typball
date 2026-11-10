@echo off
setlocal EnableExtensions EnableDelayedExpansion
chcp 65001 >nul
cd /d "%~dp0"

rem ============================================================================
rem  Typball - Windows one-click build (MinGW-w64 / GNU toolchain)
rem
rem  Double-click (or run `build.cmd`) to make a release build at dist\Typball.exe.
rem  Missing tools are installed with winget:
rem    Rust            : Rustlang.Rustup
rem    C/C++ toolchain : CodeBlocks.CodeBlocks.MinGW  (Code::Blocks bundled MinGW-w64:
rem                      gcc/g++, cmake, mingw32-make, ninja, libclang) -> %ProgramFiles%\CodeBlocks\MinGW
rem    CMake           : Kitware.CMake   (only when the found MinGW lacks cmake.exe)
rem    libclang        : LLVM.LLVM       (only when the found MinGW lacks libclang.dll)
rem
rem  Environment variables:
rem    TYPBALL_TOOLCHAIN=gnu | msvc   toolchain (default: gnu). msvc uses Visual Studio
rem                                   Build Tools and installs them with winget when missing.
rem    TYPBALL_MINGW_DIR=<path>       directory holding gcc.exe / g++.exe (gnu only)
rem    TYPBALL_PROFILE=debug          debug build (default: release)
rem    TYPBALL_DRY_RUN=1              print install/build commands instead of running them
rem
rem  NOTE (cmd.exe + UTF-8): keep every `rem` line ASCII and never put non-ASCII text
rem  inside a multi-line ( ... ) block. cmd.exe mis-computes the file offset after such
rem  lines and executes fragments of the following lines. Japanese is only used in
rem  single `echo` lines at top level or inside `call :label` subroutines.
rem ============================================================================

set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
set "WINGET_ARGS=--accept-source-agreements --accept-package-agreements --disable-interactivity"
set "CODEBLOCKS_MINGW=%ProgramFiles%\CodeBlocks\MinGW\bin"
set "LLVM_BIN=%ProgramFiles%\LLVM\bin"
set "RUN="
if defined TYPBALL_DRY_RUN set "RUN=echo [dry-run]"
if not defined TYPBALL_PROFILE set "TYPBALL_PROFILE=release"
if not defined TYPBALL_TOOLCHAIN set "TYPBALL_TOOLCHAIN=gnu"

echo.
echo [Typball] Windows ビルドを開始します (profile=%TYPBALL_PROFILE%, toolchain=%TYPBALL_TOOLCHAIN%)
if defined TYPBALL_DRY_RUN echo [Typball] ドライラン: インストールとビルドは実行せずコマンドを表示します
echo.

rem ---------------------------------------------------------------- Rust ----
where rustup >nul 2>nul
if errorlevel 1 (call :install_rust || goto :fail) else echo [Typball] Rust: 導入済み

if /i "%TYPBALL_TOOLCHAIN%"=="gnu" goto :setup_gnu
if /i "%TYPBALL_TOOLCHAIN%"=="msvc" goto :setup_msvc
echo [Typball] TYPBALL_TOOLCHAIN は gnu か msvc を指定してください。
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
echo [Typball] Rust ツールチェーン %RUST_TOOLCHAIN% を確認します...
%RUN% rustup toolchain install %RUST_TOOLCHAIN% --profile minimal --no-self-update
if errorlevel 1 goto :fail
set "RUSTUP_TOOLCHAIN=%RUST_TOOLCHAIN%"

echo.
echo [Typball] cargo build を実行します (初回は wxWidgets のビルドを含むため 10 分以上かかります)
echo.
set "CARGO_FLAGS=--locked"
set "OUT=target\debug\Typball.exe"
if /i "%TYPBALL_PROFILE%"=="release" set "CARGO_FLAGS=--release --locked"
if /i "%TYPBALL_PROFILE%"=="release" set "OUT=target\release\Typball.exe"
%RUN% cargo build %CARGO_FLAGS%
if errorlevel 1 goto :fail
if defined TYPBALL_DRY_RUN echo [Typball] ドライラン終了
if defined TYPBALL_DRY_RUN goto :end

if not exist dist mkdir dist
copy /y "%OUT%" dist\Typball.exe >nul
echo.
echo [Typball] 完了: %~dp0dist\Typball.exe
goto :end

rem ---------------------------------------------------------- subroutines ----
rem (plain line-by-line code: Japanese `echo` is safe here)

:install_rust
call :need_winget || exit /b 1
echo [Typball] Rust が見つかりません。winget で Rustup をインストールします...
%RUN% winget install -e --id Rustlang.Rustup %WINGET_ARGS%
if errorlevel 1 exit /b 1
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where rustup >nul 2>nul
if not errorlevel 1 exit /b 0
if defined TYPBALL_DRY_RUN exit /b 0
echo [Typball] rustup が見つかりません。ターミナルを開き直してから再実行してください。
exit /b 1

:install_mingw
call :need_winget || exit /b 1
echo [Typball] MinGW (gcc/g++) が見つかりません。winget で Code::Blocks with MinGW をインストールします。
echo           管理者権限の確認 (UAC) が出ます。インストール先: %ProgramFiles%\CodeBlocks
%RUN% winget install -e --id CodeBlocks.CodeBlocks.MinGW --scope machine %WINGET_ARGS%
call :detect_gcc
if defined GCC_DIR exit /b 0
if defined TYPBALL_DRY_RUN set "GCC_DIR=%CODEBLOCKS_MINGW%"
if defined GCC_DIR exit /b 0
echo [Typball] MinGW のインストールを確認できませんでした。%CODEBLOCKS_MINGW%\g++.exe がありません。
exit /b 1

:err_no_make
echo [Typball] mingw32-make.exe が見つかりません。Code::Blocks 同梱の MinGW か、
echo           MSYS2 の mingw-w64-ucrt-x86_64-toolchain を導入してください。
exit /b 0

:install_msvc
call :need_winget || exit /b 1
echo [Typball] Visual Studio 2022 Build Tools (C++ ワークロード) をインストールします。
echo           数 GB のダウンロードと管理者権限の確認 (UAC) が発生します...
%RUN% winget install -e --id Microsoft.VisualStudio.2022.BuildTools %WINGET_ARGS% --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
call :detect_msvc
if defined VSINSTALL exit /b 0
if defined TYPBALL_DRY_RUN exit /b 0
echo [Typball] Build Tools のインストールを確認できませんでした。
exit /b 1

:ensure_cmake
where cmake >nul 2>nul
if not errorlevel 1 exit /b 0
if exist "%ProgramFiles%\CMake\bin\cmake.exe" set "PATH=%ProgramFiles%\CMake\bin;%PATH%" & exit /b 0
call :need_winget || exit /b 1
echo [Typball] cmake が見つかりません。winget で CMake をインストールします...
%RUN% winget install -e --id Kitware.CMake %WINGET_ARGS%
if exist "%ProgramFiles%\CMake\bin\cmake.exe" set "PATH=%ProgramFiles%\CMake\bin;%PATH%" & exit /b 0
if defined TYPBALL_DRY_RUN exit /b 0
echo [Typball] cmake が見つかりません。
exit /b 1

:ensure_llvm
if exist "%LLVM_BIN%\libclang.dll" set "LIBCLANG_PATH=%LLVM_BIN%" & exit /b 0
call :need_winget || exit /b 1
echo [Typball] libclang.dll が見つかりません。winget で LLVM をインストールします...
%RUN% winget install -e --id LLVM.LLVM %WINGET_ARGS%
if exist "%LLVM_BIN%\libclang.dll" set "LIBCLANG_PATH=%LLVM_BIN%" & exit /b 0
if defined TYPBALL_DRY_RUN set "LIBCLANG_PATH=%LLVM_BIN%" & exit /b 0
echo [Typball] libclang.dll が見つかりません。LLVM をインストールして LIBCLANG_PATH を設定してください。
exit /b 1

:need_winget
where winget >nul 2>nul
if not errorlevel 1 exit /b 0
echo [Typball] winget が見つかりません。Microsoft Store から「アプリ インストーラー」を入れてください。
exit /b 1

:detect_gcc
set "GCC_DIR="
if defined TYPBALL_MINGW_DIR if exist "%TYPBALL_MINGW_DIR%\g++.exe" set "GCC_DIR=%TYPBALL_MINGW_DIR%"
rem Code::Blocks (setup.exe / winget --scope machine)
if not defined GCC_DIR if exist "%CODEBLOCKS_MINGW%\g++.exe" set "GCC_DIR=%CODEBLOCKS_MINGW%"
rem Code::Blocks portable zip installed by winget (user scope)
if not defined GCC_DIR for /d %%p in ("%LOCALAPPDATA%\Microsoft\WinGet\Packages\CodeBlocks.CodeBlocks.MinGW_*") do (
    if not defined GCC_DIR if exist "%%~p\MinGW\bin\g++.exe" set "GCC_DIR=%%~p\MinGW\bin"
    if not defined GCC_DIR for /d %%q in ("%%~p\*") do if not defined GCC_DIR if exist "%%~q\MinGW\bin\g++.exe" set "GCC_DIR=%%~q\MinGW\bin"
)
rem WinLibs portable zip installed by winget (BrechtSanders.WinLibs.*): same build as Code::Blocks' MinGW
if not defined GCC_DIR for /d %%p in ("%LOCALAPPDATA%\Microsoft\WinGet\Packages\BrechtSanders.WinLibs*") do (
    if not defined GCC_DIR if exist "%%~p\mingw64\bin\g++.exe" set "GCC_DIR=%%~p\mingw64\bin"
    if not defined GCC_DIR for /d %%q in ("%%~p\*") do if not defined GCC_DIR if exist "%%~q\mingw64\bin\g++.exe" set "GCC_DIR=%%~q\mingw64\bin"
)
rem other common MinGW-w64 locations
for %%d in ("C:\msys64\ucrt64\bin" "C:\msys64\mingw64\bin" "C:\mingw64\bin" "C:\mingw64\mingw64\bin" "C:\winlibs\mingw64\bin") do (
    if not defined GCC_DIR if exist "%%~d\g++.exe" set "GCC_DIR=%%~d"
)
if not defined GCC_DIR for /f "delims=" %%i in ('where g++ 2^>nul') do if not defined GCC_DIR set "GCC_DIR=%%~dpi"
if defined GCC_DIR if "!GCC_DIR:~-1!"=="\" set "GCC_DIR=!GCC_DIR:~0,-1!"
exit /b 0

:detect_msvc
set "VSINSTALL="
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if exist "%VSWHERE%" for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSINSTALL=%%i"
exit /b 0

rem ------------------------------------------------------------------ exit ----
:fail
echo.
echo [Typball] ビルドに失敗しました。
call :pause_if_doubleclicked
exit /b 1

:end
call :pause_if_doubleclicked
exit /b 0

:pause_if_doubleclicked
rem pause before the window closes when started by double-click (cmd /c)
echo %cmdcmdline% | "%SystemRoot%\System32\find.exe" /i "/c" >nul && pause
exit /b 0
