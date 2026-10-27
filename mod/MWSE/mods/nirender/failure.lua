-- The failure path of the renderer (the failure-path design, section 6). When the helper
-- numidium.exe has stopped or lost its graphics device, the game has no picture any more:
-- the game is saved as "numidium-autosave", then the plugin shows its message box, which
-- names the save. The plugin says so through `nirender.hostState()`, which is nil while the
-- helper is well (no table, no garbage). The save is tried again at each frame for 10 s when
-- the engine refuses it (a load, a menu); with no game loaded there is nothing to save. The
-- plugin shows the box by itself when this file does not answer in 3 s.

local nirender = include("nirender")
if not nirender then return end

local FILE = "numidium-autosave"
local RETRY_SECONDS = 10

local failure = nil
local onFrame

local function answer(saveName, why)
    event.unregister("enterFrame", onFrame)
    nirender.failureHandled(saveName, why)
end

onFrame = function()
    if not failure then
        local state = nirender.hostState()
        if not state then return end
        failure = { since = os.clock(), tries = 0 }
        mwse.log("[nirender] the renderer %s: the game is saved before the message", state.word)
    end
    if not tes3.player then
        mwse.log("[nirender] no game is loaded: nothing to save")
        answer(nil, "with no game loaded")
        return
    end
    local name = string.format("MGE XE autosave %s", os.date("%Y-%m-%d %H:%M"))
    failure.tries = failure.tries + 1
    if tes3.saveGame({ file = FILE, name = name }) then
        mwse.log("[nirender] the game is saved as %s (\"%s\"), try %d", FILE, name, failure.tries)
        answer(name, nil)
    elseif os.clock() - failure.since > RETRY_SECONDS then
        mwse.log("[nirender] no save was possible in %d s (%d tries)", RETRY_SECONDS, failure.tries)
        answer(nil, "in this state")
    end
end

event.register("enterFrame", onFrame)
