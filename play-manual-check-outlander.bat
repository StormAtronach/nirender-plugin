@echo off
rem The manual check of play-manual-check.bat, in the Outlander-test instance (the heavy
rem modlist; never the Outlander instance of the owner's play). The build: the same as the
rem other batch file, E:\nirender-keep\mgebuilds\f32, with the plugin of this repo.
rem The instance: D:\Modlists\Outlander-test, its game folder "Stock Game Folder", its
rem profile Outlander, and its mgeXE.toml in the mod "MGE XE G7". Its first start after a
rem version change regenerates the distant land (about 30 s). The Livecoding mod's main.lua
rem comes from the testing instance: Outlander-test has no Livecoding mod.
rem In the game: load a save, press Alt+X once, then Insert at each issue (a numbered mark:
rem the place and view in MWSE.log, 40 frames in C:\tmp\mark-N.nircap, 24 pictures).
rem Do not press F11 with Insert: one capture at a time. Home switches the renderer between
rem the table and the engine's draws; End lists the lights near the camera in MWSE.log.
rem Leave the game by its menu: the tool then puts the instance back.
cd /d "%~dp0"
"C:\OSGeo4W\apps\Python312\python.exe" tools\play.py --mge-build E:\nirender-keep\mgebuilds\f32 --plugin --livecoding "C:\Users\ivan_\source\repos\moreFPS\docs\Numidium Project\scripts\wgpu-scene-audit\livecoding-mark.lua" --instance D:\Modlists\Outlander-test --game-dir "D:\Modlists\Outlander-test\Stock Game Folder" --profile Outlander --config-mod "MGE XE G7"
echo.
echo The game is over and the instance is restored. Press a key to close this window.
pause >nul
