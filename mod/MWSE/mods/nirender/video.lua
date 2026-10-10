---@diagnostic disable
-- The in-game part of tools/run_video.py. It holds one view for a recording, one time for
-- each mode of spec.modes. The camera turns slowly, the same way for each mode. Thus a
-- person can put the clips next to each other.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "video")
    local say, after = scenario.say, scenario.after

    local turning = nil
    scenario.eachFrame = function(now)
        if turning then
            -- The angle is a function of the clock, not of the frame count. Thus the turn is
            -- the same at any frame rate.
            local orientation = tes3.player.orientation:copy()
            orientation.z = turning.start + (now - turning.at) * (spec.turnRate or 0.12)
            tes3.player.orientation = orientation
        end
    end

    local function hold(index, startAngle)
        local mode = spec.modes[index]
        if not mode then
            nirender.setMode("*", "off")
            return scenario.finish()
        end
        nirender.setMode("*", mode)
        turning = { start = startAngle, at = os.clock() }
        -- The runner starts a recording at each of these lines.
        say("view ready in '%s', mode %s", tes3.player.cell.editorName, mode)
        after(spec.holdSeconds, function()
            turning = nil
            say("view done, mode %s, %d calls ran as Rust", mode, nirender.liveCalls())
            hold(index + 1, startAngle)
        end)
    end

    scenario.start(function()
        after(3, function()
            scenario.goTo(spec.cell)
            after(spec.settleSeconds or 8, function()
                -- This removes the HUD from the clips.
                tes3.runLegacyScript({ command = "ToggleMenus" })
                hold(1, tes3.player.orientation.z)
            end)
        end)
    end)
end
