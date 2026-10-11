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

local config = mwse.loadConfig("nirender", { mode = "off", early = false })

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
