---@diagnostic disable
-- The in-game half of tools/run_video.py: holds one view in one mode for a recording.
-- The camera turns slowly, the same way in every run, so two clips can be put side by side.

return function(nirender, spec)
    local function say(fmt, ...)
        local line = "[nirender.video] " .. string.format(fmt, ...)
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

    local turning = nil
    event.register("enterFrame", function()
        local now = os.clock()
        if turning then
            -- A function of the clock, not of the frame count: the same turn at any frame rate.
            local orientation = tes3.player.orientation:copy()
            orientation.z = turning.start + (now - turning.at) * (spec.turnRate or 0.12)
            tes3.player.orientation = orientation
        end
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

    local started = false
    event.register("loaded", function()
        if started then return end
        started = true
        after(3, function()
            tes3.runLegacyScript({ command = string.format('CenterOnCell "%s"', spec.cell) })
            after(spec.settleSeconds or 8, function()
                nirender.setMode("*", spec.mode)
                -- No HUD in the clip.
                tes3.runLegacyScript({ command = "ToggleMenus" })
                turning = { start = tes3.player.orientation.z, at = os.clock() }
                say("view ready in '%s', mode %s", tes3.player.cell.editorName, spec.mode)
                after(spec.holdSeconds, function()
                    turning = nil
                    nirender.setMode("*", "off")
                    local f = io.open(spec.reportFile, "wb")
                    if f then
                        f:write(nirender.report())
                        f:close()
                    end
                    say("DONE")
                end)
            end)
        end)
    end)

    after(6, function()
        tes3.loadGame(spec.save)
    end)
    say("scenario armed")
end
