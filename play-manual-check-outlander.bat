@echo off
rem The manual check of play-manual-check.bat, in the Outlander-test instance (the heavy
rem modlist; never the Outlander instance of the owner's play). The build: MGE_BUILD of
rem local.bat, with the plugin of this repo. The instance: OUTLANDER_INSTANCE of local.bat,
rem its game folder "Stock Game Folder", its profile Outlander, and its mgeXE.toml in the mod
rem "MGE XE G7". Its first start after a version change regenerates the distant land (about
rem 30 s). The Livecoding mod's main.lua comes from the testing instance: Outlander-test has
rem no Livecoding mod.
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
"%PYTHON%" tools\play.py --mge-build "%MGE_BUILD%" --plugin --livecoding "%LIVECODING%" --instance "%OUTLANDER_INSTANCE%" --game-dir "%OUTLANDER_INSTANCE%\Stock Game Folder" --profile Outlander --config-mod "MGE XE G7"
echo.
echo The game is over and the instance is restored. Press a key to close this window.
pause >nul
