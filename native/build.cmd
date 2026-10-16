@echo off
setlocal
cd /d "%~dp0"
set "OUT_DIR=build\bin"
if not "%~1"=="" set "OUT_DIR=%~1"

set "VCVARS="
for %%P in (
  "%ProgramFiles(x86)%\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
  "%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
  "%ProgramFiles%\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
) do if exist %%~P set "VCVARS=%%~P"

if not defined VCVARS (
  echo [native] vcvars64.bat not found
  exit /b 1
)

call "%VCVARS%"
if errorlevel 1 exit /b 1

if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"
cl /nologo /EHsc /O2 /std:c++17 /DUNICODE /D_UNICODE /I src /Fo"%OUT_DIR%\\" ^
  /Fe"%OUT_DIR%\preview_host.exe" ^
  src\preview_host_main.cpp src\decoder.cpp src\presenter.cpp src\audio.cpp src\video.cpp ^
  user32.lib gdi32.lib dxgi.lib d3d11.lib d3dcompiler.lib mfplat.lib mfreadwrite.lib mfuuid.lib ole32.lib avrt.lib windowscodecs.lib
if errorlevel 1 exit /b 1

echo [native] built %OUT_DIR%\preview_host.exe
cl /nologo /EHsc /O2 /std:c++17 /I src /Fo"%OUT_DIR%\visual_transform_test.obj" ^
  /Fe"%OUT_DIR%\visual_transform_tests.exe" tests\visual_transform_test.cpp
if errorlevel 1 exit /b 1
cl /nologo /EHsc /O2 /std:c++17 /I src /Fo"%OUT_DIR%\presenter_shader_test.obj" ^
  /Fe"%OUT_DIR%\presenter_shader_tests.exe" tests\presenter_shader_test.cpp d3d11.lib d3dcompiler.lib
if errorlevel 1 exit /b 1
cl /nologo /EHsc /O2 /std:c++17 /DUNICODE /D_UNICODE /I src /Fo"%OUT_DIR%\video_stack_test.obj" ^
  /Fe"%OUT_DIR%\video_stack_tests.exe" tests\video_stack_test.cpp ^
  "%OUT_DIR%\decoder.obj" "%OUT_DIR%\presenter.obj" "%OUT_DIR%\audio.obj" "%OUT_DIR%\video.obj" ^
  user32.lib gdi32.lib dxgi.lib d3d11.lib d3dcompiler.lib mfplat.lib mfreadwrite.lib mfuuid.lib ole32.lib avrt.lib windowscodecs.lib
if errorlevel 1 exit /b 1
exit /b 0
