---@diagnostic disable
-- The in-game half of tools/run_test.py. Loads a save, runs every port in shadow mode for a
-- while, then holds the game in menu mode and runs the frame comparisons.
-- Progress goes to spec.progressFile; the runner stops the game at the line "DONE".

return function(nirender, spec)
    local function say(fmt, ...)
        local line = "[nirender.test] " .. string.format(fmt, ...)
        mwse.log("%s", line)
        local f = io.open(spec.progressFile, "ab")
        if f then
            f:write(line, "\n")
            f:close()
        end
    end

    -- Steps run from enterFrame on the real clock: game timers stop in menu mode and do not
    -- exist before a save is loaded.
    local queue = {}
    local function after(seconds, fn)
        table.insert(queue, { at = os.clock() + seconds, fn = fn })
    end
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
        local report = nirender.report()
        local f = io.open(spec.reportFile, "wb")
        if f then
            f:write(report)
            f:close()
        end
        say("report written")
        say("DONE")
    end

    -- Runs the frame comparisons one after the other, then `done`.
    local function frame_tests(where, list, done)
        local index = 0
        local function next_test()
            index = index + 1
            local kind = list[index]
            if not kind then return done() end
            if not nirender.frameTest(kind) then
                say("frame test %s %s: could not start", where, kind)
                return next_test()
            end
            local function poll()
                local result = nirender.frameTestResult()
                if result then
                    say("frame test %s %s: %s", where, kind, result)
                    after(0.2, next_test)
                else
                    after(0.05, poll)
                end
            end
            after(0.05, poll)
        end
        next_test()
    end

    local function hold_in_menu_mode()
        local menu = tes3ui.createMenu({ id = "nirender:hold", fixedFrame = true })
        menu:createLabel({ text = "nirender frame test" })
        menu:updateLayout()
        tes3ui.enterMenuMode("nirender:hold")
    end

    local started = false
    event.register("loaded", function()
        if started then return end
        started = true
        say("save loaded, cell '%s', x87 control word 0x%04x", tes3.player.cell.editorName, nirender.fpuControlWord())
        after(spec.settleSeconds or 4, function()
            say("shadow mode on for %d s (%s)", spec.shadowSeconds, spec.ports or "*")
            nirender.setMode(spec.ports or "*", "shadow")
            after(spec.shadowSeconds, function()
                nirender.setMode("*", "off")
                say("shadow mode off")
                frame_tests("gameplay", { "control" }, function()
                    hold_in_menu_mode()
                    say("menu mode on")
                    after(1, function()
                        frame_tests("menu", { "control", "control", "live", "live", "control" }, function()
                            -- Back to the game with the Rust code for a moment: it has to keep running.
                            tes3ui.leaveMenuMode()
                            local menu = tes3ui.findMenu("nirender:hold")
                            if menu then menu:destroy() end
                            nirender.setMode(spec.ports or "*", spec.liveSeconds and "live" or "off")
                            after(spec.liveSeconds or 0, function()
                                nirender.setMode("*", "off")
                                finish()
                            end)
                        end)
                    end)
                end)
            end)
        end)
    end)

    after(spec.menuSeconds or 6, function()
        if spec.save then
            say("loading save '%s'", spec.save)
            tes3.loadGame(spec.save)
        else
            say("starting a new game")
            tes3.newGame()
        end
    end)
    say("scenario armed")
end
