@echo off
REM Forge launcher — sets up MSVC environment and runs Forge.
REM Run from any terminal, or double-click from Explorer.
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsall.bat" x64 >nul 2>&1
cd /d "%~dp0"
cargo run --release
