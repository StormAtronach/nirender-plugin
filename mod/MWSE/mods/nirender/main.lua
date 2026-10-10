-- nirender: loads nirender.dll and installs it.
--
-- MWSE/config/nirender.json:
--   "mode":  "off" | "shadow" | "live" for every ported function once the game is up.
--            "off" keeps the game code, "shadow" runs both and compares, "live" runs the Rust code.
--   "early": true installs when this file loads, before the game creates its renderer, and
--            applies "mode" at once, so the start of the game runs in that mode too.
--            false (the default) installs at the "initialized" event.
-- A run spec (MWSE/config/nirender_test.json, written by tools/run_test.py) starts the test
-- scenario in nirender/test.lua; without one this file only installs.

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

event.register("initialized", function()
    if installed then
        -- Other mods may have changed the game code since this file loaded.
        nirender.recheck()
    else
        installed = install()
        if not installed then return end
        if not spec then nirender.setMode("*", startMode) end
    end
    if spec then
        require("nirender.test")(nirender, spec)
    end
end)
