---@diagnostic disable
-- The in-game part of tools/run_link.py. It connects the plugin to the render link of
-- MGE XE. In each place it lets the game run, first with the game code and then with the
-- Rust code, and reads the counters of MGE XE: for each fact, how many times the packet of
-- the plugin and the inference of MGE XE were equal or different.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "link")
    local say, after = scenario.say, scenario.after

    -- Lets the game run for spec.seconds in `mode`, then reports the counters.
    local function measure(where, mode, done)
        nirender.setMode("*", mode)
        after(1, function()
            nirender.linkCounters(true)
            after(spec.seconds, function()
                for line in nirender.linkCounters(true):gmatch("[^\n]+") do
                    say("counters|%s|%s|%s", where, mode, line)
                end
                done()
            end)
        end)
    end

    local function test_here(done)
        local where = tes3.player.cell.editorName
        measure(where, "off", function()
            measure(where, "live", function()
                nirender.setMode("*", "off")
                done()
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
        after(spec.settleSeconds or 6, function()
            local ok, message = nirender.link()
            say("link: %s (%s)", tostring(ok), message)
            if not ok then return scenario.finish() end
            test_here(function() visit(1) end)
        end)
    end)
end
