-- nirender loads nirender.dll and installs it.
--
-- MWSE/config/nirender.json:
--   "mode":  "off", "shadow" or "live", for every port after the game started.
--            "off" keeps the game code. "shadow" runs the two functions and compares them.
--            "live" runs the Rust code.
--   "early": true installs when this file loads, before the game creates its renderer.
--            It applies "mode" immediately, so the start of the game also runs in that mode.
--            false is the default. It installs at the "initialized" event.
-- A run spec starts the test scenario in nirender/test.lua. The run spec is
-- MWSE/config/nirender_test.json, and tools/run_test.py writes it. Without a run spec this
-- file only installs.

local nirender = include("nirender")
if not nirender then
    mwse.log("[nirender] nirender.dll not loaded. If you use a mod manager, check that it did not leave .dll files out.")
    return
end

local config = mwse.loadConfig("nirender", { mode = "off", early = false, captureKey = "f11", overlayKey = "f10" })

local spec = nil
do
    local ok, result = pcall(mwse.loadConfig, "nirender_test")
    if ok then spec = result end
end

local early = config.early
local startMode = config.mode
if spec then
    early = spec.startMode ~= nil
    startMode = spec.startMode or "off"
end

-- The game has the Direct3D 8 of the plugin, and no Direct3D device. The Rust code is then
-- the only renderer: it must be on before the game makes its renderer, with the link to the
-- host program and with all of its facts and kinds in use.
local ownDevice = nirender.ownDevice()
if ownDevice then
    early = true
    startMode = "live"
end

mwse.log("[nirender] version %s, %d ports, mode %s, early %s, test spec %s, renderer at load 0x%x", nirender.version,
    nirender.ports, startMode, tostring(early), spec and "present" or "absent", nirender.renderer())

local function install()
    local ok, message = nirender.install(spec and spec.logFile or nil)
    mwse.log("[nirender] install: %s", message)
    return ok
end

local installed = false
if early then
    installed = install()
    if installed then nirender.setMode("*", startMode) end
end
if ownDevice and installed then
    local ok, message = nirender.link()
    mwse.log("[nirender] own device, link: %s (%s)", tostring(ok), message)
    mwse.log("[nirender] own device, textures through the link: %s, facts %s, kinds %s", tostring(nirender.linkTextures(true)),
        tostring(nirender.linkUseFacts(4261413630)), tostring(nirender.linkUseKinds(15)))
elseif ownDevice then
    -- The renderer is refused for this session (a port cannot run as Rust): the game runs
    -- MGE XE on Direct3D 9. The player sees why.
    event.register("loaded", function()
        tes3.messageBox("MGE XE: the wgpu renderer is off for this session. See MWSE.log and nirender.log in the Morrowind folder.")
    end, { doOnce = true })
end

-- A capture of the next frames for a bug report: the key of the configuration (a name of
-- tes3.scanCode; "none" for no key) writes nirender-<date>.nircap into the folder of the game.
local captureKey = tostring(config.captureKey or "none"):lower()
if ownDevice and installed and captureKey ~= "none" and tes3.scanCode[captureKey] then
    event.register("keyDown", function(e)
        if e.keyCode ~= tes3.scanCode[captureKey] or e.isAltDown or e.isControlDown or e.isShiftDown then return end
        local path = string.format("nirender-%s.nircap", os.date("%Y%m%d-%H%M%S"))
        local ok, problem = nirender.capture(path, 0, 3)
        tes3.messageBox(ok and ("nirender: 3 frames go into " .. path) or ("nirender: no capture: " .. tostring(problem)))
    end)
end

-- The debug overlay of the renderer (the frame rate, the long frames, the memory of the game
-- and of the renderer): the key of the configuration turns it on and off ("none" for no key).
local overlayKey = tostring(config.overlayKey or "none"):lower()
if ownDevice and installed and overlayKey ~= "none" and tes3.scanCode[overlayKey] then
    event.register("keyDown", function(e)
        if e.keyCode ~= tes3.scanCode[overlayKey] or e.isAltDown or e.isControlDown or e.isShiftDown then return end
        nirender.overlay()
    end)
end

event.register("initialized", function()
    if installed then
        -- It is possible that other mods changed the game code after this file loaded.
        nirender.recheck()
    else
        installed = install()
        if not installed then return end
        if not spec then nirender.setMode("*", startMode) end
    end
    if spec then
        -- The scenario is "test", "perf" or "video".
        require("nirender." .. spec.scenario)(nirender, spec)
    end
end)
