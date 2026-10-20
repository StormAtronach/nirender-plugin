@echo off
rem A game to play by hand with the wgpu renderer and the mark key (the owner, 2026-10-20).
rem The build: E:\nirender-keep\mgebuilds\f28 (MGE 048610e) with the plugin of this repo.
rem In the game: load a save, press Alt+X once, then Insert at each issue (a numbered mark:
rem the place and view in MWSE.log, 40 frames in C:\tmp\mark-N.nircap, 24 pictures).
rem Do not press F11 with Insert: one capture at a time. Home switches the renderer between
rem the table and the engine's draws; End lists the lights near the camera in MWSE.log.
rem Leave the game by its menu: the tool then puts the instance back.
cd /d "%~dp0"
"C:\OSGeo4W\apps\Python312\python.exe" tools\play.py --mge-build E:\nirender-keep\mgebuilds\f28 --plugin --livecoding "C:\Users\ivan_\source\repos\moreFPS\docs\Numidium Project\scripts\wgpu-scene-audit\livecoding-mark.lua"
echo.
echo The game is over and the instance is restored. Press a key to close this window.
pause >nul
