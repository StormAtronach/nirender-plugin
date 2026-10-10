---@diagnostic disable
-- What the test scenarios share: the progress lines, timed steps, the load of the save, the
-- report.
--
-- A runner in tools/ writes the run spec, starts the game and reads spec.progressFile. The
-- runner stops the game at the line "DONE".

-- Makes the helpers for one scenario. `name` goes into the tag of each progress line.
return function(nirender, spec, name)
    local scenario = {}
    local tag = "[nirender." .. name .. "] "

    -- Writes a progress line to the MWSE log and to the progress file.
    function scenario.say(fmt, ...)
        local line = tag .. string.format(fmt, ...)
        mwse.log("%s", line)
        local f = io.open(spec.progressFile, "ab")
        if f then
            f:write(line, "\n")
            f:close()
        end
    end

    -- The steps run from enterFrame on the real clock. Game timers stop in menu mode. They do
    -- not exist before the game loads a save.
    local queue = {}
    function scenario.after(seconds, fn)
        table.insert(queue, { at = os.clock() + seconds, fn = fn })
    end

    -- A function that runs on every frame, before the steps. A scenario can set it.
    scenario.eachFrame = nil

    event.register("enterFrame", function()
        local now = os.clock()
        if scenario.eachFrame then scenario.eachFrame(now) end
        for i = #queue, 1, -1 do
            local step = queue[i]
            if now >= step.at then
                table.remove(queue, i)
                local ok, problem = pcall(step.fn)
                if not ok then
                    scenario.say("ERROR %s", tostring(problem))
                    scenario.say("DONE")
                end
            end
        end
    end)

    -- Goes to a cell by its name.
    function scenario.goTo(cell)
        scenario.say("going to '%s'", cell)
        tes3.runLegacyScript({ command = string.format('CenterOnCell "%s"', cell) })
    end

    -- Writes the port table to spec.reportFile and ends the scenario.
    function scenario.finish()
        local f = io.open(spec.reportFile, "wb")
        if f then
            f:write(nirender.report())
            f:close()
        end
        scenario.say("report written")
        scenario.say("DONE")
    end

    -- Loads the save of the spec from the main menu. `atMainMenu` runs just before the load.
    -- `onLoaded` runs one time, after the load.
    function scenario.start(onLoaded, atMainMenu)
        local started = false
        event.register("loaded", function()
            if started then return end
            started = true
            -- The player can arrive in water or near an enemy. A dead player ends the game.
            tes3.worldController.menuController.godModeEnabled = true
            scenario.say("save loaded, cell '%s', god mode on", tes3.player.cell.editorName)
            onLoaded()
        end)
        scenario.after(spec.menuSeconds or 6, function()
            if atMainMenu then atMainMenu() end
            if spec.save then
                scenario.say("loading save '%s'", spec.save)
                tes3.loadGame(spec.save)
            else
                scenario.say("starting a new game")
                tes3.newGame()
            end
        end)
        scenario.say("scenario armed")
    end

    return scenario
end
