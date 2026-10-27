Test build of MGE XE G7 with the wgpu renderer. Read `MGE XE Renderer Readme.md` in the archive first.

## Before you install

- **Check the download.** The SHA-256 of the archive is `{sha256}`; the file `<archive>.sha256` beside it holds the same (`sha256sum -c`, or `certutil -hashfile <archive> SHA256` on Windows). `MANIFEST.txt` inside the archive lists each file with its SHA-256.
- **Install the whole archive.** The game's plugin (`akulakhan.dll`) and the renderer (`numidium.exe`) must come from the same archive.
- **MWSE.dll is in the archive, and you need it.** (Say whether it changed since the last build.)
- **Do not run `MWSE-Update.exe`** while you test this build. It puts the public MWSE.dll back, and the renderer then turns off.
- **The distant land data**: say whether it must be generated again (the version number).
- **The settings file**: say which settings are new or changed.

## Changes since vX.Y.Z

- **What a player sees first.** One line of what was wrong, one of what changed.

## Known issues

- One list, kept in the readme too; name each issue once.

## Please check by hand

- What a tester can see and report, each as a place and a thing to look at.

## How to report a problem

- Where to post, and what to attach: the archive's name, mgeXE.log, MWSE.log, and a save and mgeXE.toml zipped.

## For developers

### Tested

Windows 11, NVIDIA RTX 5070, 1920 x 1080, Vulkan, started from Mod Organizer, with the files of this archive (its plugin and its MWSE.dll):

- {gate}
  (release.py fills the token; with `--no-gate` write the line yourself as "the capture gate (run by hand): N of M pictures equal to the accepted ones")
- The live checks through the mwse-mcp bridge, in one game session, run folder `YYYYmmdd-HHMMSS-label` under `nirender-plugin/results`: what was checked and what it showed.

### Sources

MGE XE G7 `feature/wgpu-renderer` at the tag `nirender-vX.Y.Z` and nirender-plugin `master` at the tag `vX.Y.Z` of this release; MWSE `<branch>` at `<commit>` for the MWSE.dll; water-volumes-plugin `master` at `<commit>` for True Water. `SOURCES.txt` in the archive has the same.
