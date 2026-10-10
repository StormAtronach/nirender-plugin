---@diagnostic disable
-- The in-game part of tools/run_link.py. It connects the plugin to the render link of
-- MGE XE.
--
-- In each place it lets the game run, first with the game code and then with the Rust code,
-- and reads the counters of MGE XE: for each fact, how many times the packet of the plugin
-- and the inference of MGE XE were equal or different.
--
-- With spec.facts it also tests the use of the facts. MGE XE takes the facts of that mask
-- from the packets. The scenario holds the game in menu mode and saves three pictures of the
-- frame: facts off, facts on, facts off. The runner compares the pictures.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "link")
    local say, after = scenario.say, scenario.after
    local facts = spec.facts or 0
    local place = 0

    -- Lets the game run for spec.seconds in `mode`, then reports the counters.
    local function measure(where, mode, label, done)
        nirender.setMode("*", mode)
        after(1, function()
            nirender.linkCounters(true)
            after(spec.seconds, function()
                for line in nirender.linkCounters(true):gmatch("[^\n]+") do
                    say("counters|%s|%s|%s", where, label, line)
                end
                done()
            end)
        end)
    end

    -- Saves the pictures `names` in sequence. Each entry is { name, mask }.
    local function pictures(where, list, index, done)
        local entry = list[index]
        if not entry then return done() end
        nirender.linkUseFacts(entry[2])
        -- The frame with the new mask must be on the screen before the capture.
        after(0.6, function()
            local path = string.format("%s/%d-%s.bmp", spec.shotDir, place, entry[1])
            mge.saveScreenshot({ path = path, captureWithUI = true })
            after(0.8, function()
                say("picture|%s|%d|%s|%s", where, place, entry[1], path)
                pictures(where, list, index + 1, done)
            end)
        end)
    end

    local function picture_test(where, done)
        if facts == 0 then return done() end
        local menu = tes3ui.createMenu({ id = "nirender:hold", fixedFrame = true })
        menu:createLabel({ text = "nirender link test" })
        menu:updateLayout()
        tes3ui.enterMenuMode("nirender:hold")
        -- The frame changes for a moment after the start of menu mode.
        after(3, function()
            pictures(where, { { "off1", 0 }, { "on", facts }, { "off2", 0 } }, 1, function()
                tes3ui.leaveMenuMode()
                local held = tes3ui.findMenu("nirender:hold")
                if held then held:destroy() end
                done()
            end)
        end)
    end

    local function test_here(done)
        local where = tes3.player.cell.editorName
        place = place + 1
        nirender.linkUseFacts(0)
        measure(where, "off", "off", function()
            measure(where, "live", "live", function()
                nirender.setMode("*", "off")
                picture_test(where, function()
                    if facts == 0 then return done() end
                    -- The game runs with the facts in use. It must continue to run, and the
                    -- counters must stay equal.
                    say("facts in use: mask %s", tostring(nirender.linkUseFacts(facts)))
                    measure(where, "off", "facts", function()
                        nirender.linkUseFacts(0)
                        done()
                    end)
                end)
            end)
        end)
    end

    local function visit(index)
        local cell = spec.cells and spec.cells[index]
        if not cell then return scenario.finish() end
        scenario.goTo(cell)
        after(spec.settleSeconds or 6, function()
            test_here(function() visit(index + 1) end)
        end)
    end

    scenario.start(function()
        if spec.hour then
            -- The moons and their shadow shapes are in view only at night.
            tes3.worldController.hour.value = spec.hour
            say("game hour set to %s", tostring(spec.hour))
        end
        after(spec.settleSeconds or 6, function()
            local ok, message = nirender.link()
            say("link: %s (%s)", tostring(ok), message)
            if not ok then return scenario.finish() end
            test_here(function() visit(1) end)
        end)
    end)
end
