---@diagnostic disable
-- The in-game half of tools/run_perf.py: frame times with the game code against frame times
-- with the Rust code, in one cell, from one camera position.
--
-- The two modes take turns in short rounds (off, live, off, live, ...), so that a change of
-- the scene, of the weather or of the machine load falls on both in the same measure.
-- A frame time is the time between two Present calls, measured in the plugin.

return function(nirender, spec)
    local function say(fmt, ...)
        local line = "[nirender.perf] " .. string.format(fmt, ...)
        mwse.log("%s", line)
        local f = io.open(spec.progressFile, "ab")
        if f then
            f:write(line, "\n")
            f:close()
        end
    end

    local queue = {}
    local function after(seconds, fn)
        table.insert(queue, { at = os.clock() + seconds, fn = fn })
    end

    local rounds = {}

    event.register("enterFrame", function()
        local now = os.clock()
        for i = #queue, 1, -1 do
            local step = queue[i]
            if now >= step.at then
                table.remove(queue, i)
                local ok, problem = pcall(step.fn)
                if not ok then
                    say("ERROR %s", tostring(problem))
                    say("DONE")
                end
            end
        end
    end)

    local function finish()
        local f = io.open(spec.resultFile, "wb")
        if f then
            f:write(json.encode({ cell = tes3.player.cell.editorName, rounds = rounds }))
            f:close()
        end
        say("result written")
        say("DONE")
    end

    local function round(index)
        if index > spec.rounds * 2 then
            nirender.setMode("*", "off")
            return finish()
        end
        local mode = (index % 2 == 1) and "off" or "live"
        nirender.setMode("*", mode)
        -- A moment for caches to settle after the switch, outside the measurement.
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

    local started = false
    event.register("loaded", function()
        if started then return end
        started = true
        say("save loaded")
        after(3, function()
            say("going to '%s'", spec.cell)
            tes3.runLegacyScript({ command = string.format('CenterOnCell "%s"', spec.cell) })
            after(spec.settleSeconds or 8, function()
                say("measuring in '%s': %d rounds of %d s for each mode", tes3.player.cell.editorName, spec.rounds, spec.roundSeconds)
                round(1)
            end)
        end)
    end)

    after(6, function()
        say("loading save '%s'", spec.save)
        tes3.loadGame(spec.save)
    end)
    say("scenario armed")
end
