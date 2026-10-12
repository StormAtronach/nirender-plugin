---@diagnostic disable
-- The in-game part of tools/run_test.py. It loads a save. It runs the ports in shadow mode at
-- the place of the save and in each cell of spec.cells. After each place it holds the game in
-- menu mode and runs the frame comparisons.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "test")
    local say, after = scenario.say, scenario.after

    local ports = spec.ports or "*"
    nirender.recordBuffers(spec.recordBuffers ~= false)
    if (spec.fault or 0) ~= 0 then
        nirender.fault(spec.fault)
        say("FAULT %d is in the Rust code on purpose: this run has to fail", spec.fault)
    end

    -- Calls `done` with the result of the frame test or frame dump that is in progress.
    local function when_frame_result(done)
        local function poll()
            local result = nirender.frameTestResult()
            if result then
                done(result)
            else
                after(0.05, poll)
            end
        end
        after(0.05, poll)
    end

    -- Runs the frame comparisons in sequence, then `done`. A comparison that reports a change
    -- of the scene runs again, 8 times at most.
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
            when_frame_result(function(result)
                local retry = result:sub(1, 5) == "RETRY" and tries < 8
                if retry then
                    mwse.log("[nirender.test] frame test %s %s try %d: %s", where, kind, tries + 1, result:sub(1, 200))
                else
                    say("frame test %s %s: %s", where, kind, result)
                end
                after(0.2, function() next_test(retry) end)
            end)
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

    -- With spec.extras, makes the game use the parts of the renderer that a normal frame does
    -- not use, while the comparison is on: the inventory with its character image and its
    -- map, a screenshot, and a save with its picture. Then `done`.
    local extras_done = false
    local function extras(done)
        if not spec.extras or extras_done then return done() end
        extras_done = true
        for _, name in ipairs({ "MenuInventory", "MenuMap", "MenuStat", "MenuMagic" }) do
            local shown = tes3ui.findMenu(name)
            if shown then
                shown.visible = true
                shown.disabled = false
                shown:updateLayout()
            end
        end
        say("extras: inventory, menu mode %s", tostring(tes3ui.enterMenuMode("MenuInventory")))
        after(3, function()
            tes3ui.leaveMenuMode()
            after(1, function()
                local ok, result = pcall(function()
                    return tes3.worldController.worldCamera.renderer:takeScreenshot(nil)
                end)
                say("extras: screenshot of the renderer: %s %s", tostring(ok), tostring(result))
                after(1, function()
                    local saved = tes3.saveGame({ file = "nirender_test", name = "nirender test" })
                    say("extras: save with its picture: %s", tostring(saved))
                    os.remove("saves/nirender_test.ess")
                    after(1, done)
                end)
            end)
        end)
    end

    -- With spec.eachSeconds, the comparison of each port alone, one after the other, then
    -- `done`. A port that another port calls has no comparison of its own while the caller
    -- is in a comparison: the caller runs it as Rust. Alone, each call of it is compared.
    local function each_port(done)
        local names = {}
        for line in nirender.report():gmatch("[^\n]+") do
            local name = line:match("^([^	]+)	0x")
            if name and (ports == "*" or name:find(ports, 1, true)) then names[#names + 1] = name end
        end
        say("comparison of %d ports, each one alone, %s s each", #names, tostring(spec.eachSeconds))
        local function step(index)
            nirender.setMode("*", "off")
            local name = names[index]
            if not name then return done() end
            nirender.setMode(name, "shadow")
            scenario.tour(spec.eachSeconds, function() step(index + 1) end)
        end
        step(1)
    end

    -- Runs shadow mode for spec.shadowSeconds, then the frame tests in menu mode, then `done`.
    local function test_here(done)
        local where = tes3.player.cell.editorName
        say("shadow mode on for %d s in '%s' (%s)", spec.shadowSeconds, where, ports)
        if spec.eachSeconds then
            return each_port(done)
        end
        nirender.setMode(ports, "shadow")
        scenario.tour(spec.shadowSeconds, function() extras(function()
            nirender.setMode("*", "off")
            hold_in_menu_mode()
            after(0.5, function()
                frame_tests("menu", { "control", "live", "live", "control" }, function()
                    release_menu_mode()
                    done()
                end)
            end)
        end) end)
    end

    local function visit(index)
        local cell = spec.cells and spec.cells[index]
        if not cell then
            -- The game runs with the Rust code for a short time. The game must continue to run.
            nirender.setMode(ports, (spec.liveSeconds or 0) > 0 and "live" or "off")
            say("live mode for %d s", spec.liveSeconds or 0)
            return after(spec.liveSeconds or 0, function()
                -- The extras again, on the Rust code: a comparison does not include the
                -- calls that it skips.
                extras_done = (spec.liveSeconds or 0) == 0
                extras(function()
                    nirender.setMode("*", "off")
                    scenario.finish()
                end)
            end)
        end
        scenario.goTo(cell)
        after(spec.settleSeconds or 4, function()
            test_here(function() visit(index + 1) end)
        end)
    end

    -- Writes the renderer objects and one frame, for the comparison of two runs. Each run
    -- writes them at the same point. At that point the save is loaded, the game is in menu
    -- mode and every port is off.
    local function dumps(done)
        if not spec.dumpDir then return done() end
        nirender.setMode("*", "off")
        hold_in_menu_mode()
        after(1, function()
            say("state dump: %s", tostring(nirender.dumpState(spec.dumpDir .. "/state.txt")))
            nirender.dumpFrame(spec.dumpDir .. "/frame.txt")
            when_frame_result(function(result)
                say("frame dump: %s", result)
                release_menu_mode()
                done()
            end)
        end)
    end

    -- With spec.reset, makes the game reset its device, with the game code ("off") or the
    -- Rust code ("live"), then `done`. The frame tests after it show that the renderer is
    -- as before.
    local function reset(done)
        if not spec.reset then return done() end
        nirender.setMode(ports, spec.reset)
        after(1, function()
            say("reset of the device asked, mode %s", spec.reset)
            nirender.forceReset()
            after(4, function()
                say("after the reset: the game runs")
                nirender.setMode("*", "off")
                done()
            end)
        end)
    end

    scenario.start(function()
        say("x87 control word 0x%04x", nirender.fpuControlWord())
        if spec.startMode then
            -- The mode that the start of the game ran in. The report has the counts.
            say("the game started in mode '%s'", spec.startMode)
        end
        after(spec.settleSeconds or 4, function()
            dumps(function() reset(function()
                if (spec.shadowSeconds or 0) > 0 then
                    test_here(function() visit(1) end)
                else
                    scenario.finish()
                end
            end) end)
        end)
    end, function()
        if spec.dumpDir then
            -- The renderer as the start of the game left it, at the main menu.
            say("state dump at the main menu: %s", tostring(nirender.dumpState(spec.dumpDir .. "/state_start.txt")))
        end
    end)
end
