@echo off
rem A game to play by hand with the wgpu renderer and the mark key (the owner, 2026-10-20).
rem The build: MGE_BUILD of local.bat (a helper build folder with d3d8.dll and numidium.exe)
rem with the plugin of this repo. local.bat also names the Python interpreter and, if it is
rem not next to this repo in moreFPS, the livecoding file of the mark key.
rem In the game: load a save, press Alt+X once, then Insert at each issue (a numbered mark:
rem the place and view in MWSE.log, 40 frames in C:\tmp\mark-N.nircap, 24 pictures).
rem Do not press F11 with Insert: one capture at a time. Home switches the renderer between
rem the table and the engine's draws; End lists the lights near the camera in MWSE.log.
rem Leave the game by its menu: the tool then puts the instance back.
cd /d "%~dp0"
if not exist local.bat (
    echo local.bat is missing: copy local.example.bat to local.bat and set its places.
    pause
    exit /b 1
)
call "%~dp0local.bat"
if not defined LIVECODING set "LIVECODING=%~dp0..\moreFPS\docs\Numidium Project\scripts\wgpu-scene-audit\livecoding-mark.lua"
"%PYTHON%" tools\play.py --mge-build "%MGE_BUILD%" --plugin --livecoding "%LIVECODING%"
echo.
echo The game is over and the instance is restored. This window closes in 60 s, or at a key.
timeout /t 60 >nul
