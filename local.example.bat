@echo off
rem The places of this machine for the batch files at the repo root. Copy this file to
rem local.bat (not tracked) and set the values.
rem The Python interpreter of the tools (3.12 with Pillow).
set "PYTHON=C:\Python312\python.exe"
rem A build folder of the MGE XE G7 helper: d3d8.dll, numidium.exe and its Data Files.
set "MGE_BUILD=C:\path\to\mgebuilds\f32"
rem The Mod Organizer instance of the heavy modlist for play-manual-check-outlander.bat.
set "OUTLANDER_INSTANCE=D:\path\to\Outlander-test"
rem The livecoding file of the mark key; without this line, the one in the sibling repo moreFPS.
rem set "LIVECODING=C:\path\to\livecoding-mark.lua"
