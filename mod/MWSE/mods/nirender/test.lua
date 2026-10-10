---@diagnostic disable
-- The in-game half of tools/run_test.py. Loads a save, runs the ports in shadow mode at the
-- place of the save and in each cell of spec.cells, then holds the game in menu mode and
-- runs the frame comparisons.
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

    local ports = spec.ports or "*"

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

    -- Runs the frame comparisons one after the other, then `done`. A comparison that reports
    -- a change of the scene runs again, up to 8 times.
    local function frame_tests(where, list, done)
        local index = 0
        local tries = 0
        local function next_test(again)
            if again then
                tries = tries + 1
            else
                index = index + 1
                tries = 0
            end
            local kind = list[index]
            if not kind then return done() end
            if not nirender.frameTest(kind) then
                say("frame test %s %s: could not start", where, kind)
                return next_test()
            end
            local function poll()
                local result = nirender.frameTestResult()
                if result then
                    local retry = result:sub(1, 5) == "RETRY" and tries < 8
                    if retry then
                        mwse.log("[nirender.test] frame test %s %s try %d: %s", where, kind, tries + 1, result:sub(1, 200))
                    else
                        say("frame test %s %s: %s", where, kind, result)
                    end
                    after(0.2, function() next_test(retry) end)
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

    local function release_menu_mode()
        tes3ui.leaveMenuMode()
        local menu = tes3ui.findMenu("nirender:hold")
        if menu then menu:destroy() end
    end

    -- Shadow mode for spec.shadowSeconds, then the frame tests in menu mode, then `done`.
    local function test_here(done)
        local where = tes3.player.cell.editorName
        say("shadow mode on for %d s in '%s' (%s)", spec.shadowSeconds, where, ports)
        nirender.setMode(ports, "shadow")
        after(spec.shadowSeconds, function()
            nirender.setMode("*", "off")
            hold_in_menu_mode()
            after(0.5, function()
                frame_tests("menu", { "control", "live", "live", "control" }, function()
                    release_menu_mode()
                    done()
                end)
            end)
        end)
    end

    local function visit(index)
        local cell = spec.cells and spec.cells[index]
        if not cell then
            -- Back in the game with the Rust code for a moment: it has to keep running.
            nirender.setMode(ports, (spec.liveSeconds or 0) > 0 and "live" or "off")
            say("live mode for %d s", spec.liveSeconds or 0)
            return after(spec.liveSeconds or 0, function()
                nirender.setMode("*", "off")
                finish()
            end)
        end
        say("going to '%s'", cell)
        tes3.runLegacyScript({ command = string.format('CenterOnCell "%s"', cell) })
        after(spec.settleSeconds or 4, function()
            test_here(function() visit(index + 1) end)
        end)
    end

    local started = false
    event.register("loaded", function()
        if started then return end
        started = true
        say("save loaded, cell '%s', x87 control word 0x%04x", tes3.player.cell.editorName, nirender.fpuControlWord())
        after(spec.settleSeconds or 4, function()
            test_here(function() visit(1) end)
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
