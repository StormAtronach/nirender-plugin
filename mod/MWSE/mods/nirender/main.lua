-- nirender: loads nirender.dll and installs it once the game has its renderer.
--
-- MWSE/config/nirender.json: { "mode": "off" | "shadow" | "live" } for every ported function.
-- "off" keeps the game code, "shadow" runs both and compares, "live" runs the Rust code.
-- A run spec (MWSE/config/nirender_test.json, written by tools/run_test.py) starts the test
-- scenario in nirender/test.lua; without one this file only installs.

local nirender = include("nirender")
if not nirender then
    mwse.log("[nirender] nirender.dll not loaded. If you use a mod manager, check that it did not leave .dll files out.")
    return
end

local config = mwse.loadConfig("nirender", { mode = "off" })

local spec = nil
do
    local ok, result = pcall(mwse.loadConfig, "nirender_test")
    if ok then spec = result end
end

mwse.log("[nirender] version %s, %d ports, mode %s, test spec %s, renderer at load 0x%x", nirender.version, nirender.ports,
    config.mode, spec and "present" or "absent", nirender.renderer())

event.register("initialized", function()
    local ok, message = nirender.install(spec and spec.logFile or nil)
    mwse.log("[nirender] install: %s", message)
    if not ok then return end
    if spec then
        require("nirender.test")(nirender, spec)
    else
        nirender.setMode("*", config.mode)
    end
end)
