---@diagnostic disable
-- The in-game part of tools/run_perf.py. It compares frame times with the game code and
-- frame times with the Rust code, in one cell, from one camera position.
--
-- The modes alternate in short rounds: off, live, off, live. Thus a change of the scene,
-- of the weather or of the machine load affects the modes equally.
-- With spec.link the plugin connects to MGE XE before the first round, so each mode also
-- sends the packets of the render link, and there is a third mode "link": the Rust code
-- with the facts spec.facts and the kinds spec.kinds in use.
-- A frame time is the time between two Present calls. The plugin measures it.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "perf")
    local say, after = scenario.say, scenario.after

    local rounds = {}

    local function finish()
        local f = io.open(spec.resultFile, "wb")
        if f then
            f:write(json.encode({ cell = tes3.player.cell.editorName, rounds = rounds }))
            f:close()
        end
        say("result written")
        scenario.finish()
    end

    local function round(index)
        local modes = spec.link and { "off", "live", "link" } or { "off", "live" }
        if index > spec.rounds * #modes then
            if spec.link then
                nirender.linkUseKinds(0)
                nirender.linkUseFacts(0)
            end
            nirender.setMode("*", "off")
            return finish()
        end
        local mode = modes[(index - 1) % #modes + 1]
        if spec.link then
            -- Give the state back before the game code draws again.
            if mode ~= "link" then
                nirender.linkUseKinds(0)
                nirender.linkUseFacts(0)
            end
        end
        nirender.setMode("*", mode == "link" and "live" or mode)
        if mode == "link" then
            nirender.linkUseFacts(spec.facts or 0)
            nirender.linkUseKinds(spec.kinds or 0)
        end
        -- The caches settle for a moment after the change of mode. This time is not measured.
        after(spec.warmSeconds or 1, function()
            nirender.timing(true)
            after(spec.roundSeconds, function()
                local times = nirender.timing(false)
                table.insert(rounds, { mode = mode, microseconds = times })
                local _, commas = times:gsub(",", "")
                say("round %d %s: %d frames", index, mode, commas + 1)
                round(index + 1)
            end)
        end)
    end

    scenario.start(function()
        after(3, function()
            if spec.cell and spec.cell ~= "" then scenario.goTo(spec.cell) end
            if spec.link then say("link: %s", tostring(nirender.link())) end
            after(spec.settleSeconds or 8, function()
                say("measuring in '%s': %d rounds of %d s for each mode", tes3.player.cell.editorName, spec.rounds,
                    spec.roundSeconds)
                round(1)
            end)
        end)
    end)
end
