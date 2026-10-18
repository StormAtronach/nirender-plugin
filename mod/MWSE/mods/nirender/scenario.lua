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

    -- Puts the player in front of each object of the cell whose id starts with spec.tour, for
    -- spec.tourSeconds each, then calls `done`. A test level shows some effects only from
    -- near: its meshes have level-of-detail nodes. Without spec.tour, or in a cell that has
    -- no such object, it waits `seconds` and calls `done`. With `each`, it calls
    -- `each(index, ref, next)` after the wait at each stop, and `next()` goes on.
    function scenario.tour(seconds, done, each)
        local stops = {}
        if spec.tour then
            local prefix = spec.tour:lower()
            for ref in tes3.player.cell:iterateReferences() do
                local id = ref.object.id:lower()
                if id:sub(1, #prefix) == prefix and not id:find("tile", 1, true) and not ref.disabled then
                    stops[#stops + 1] = ref
                end
            end
        end
        if #stops == 0 then return scenario.after(seconds, done) end
        scenario.say("tour of %d objects in '%s'", #stops, tes3.player.cell.editorName)
        local function stop(index)
            local ref = stops[index]
            if not ref then return done() end
            -- South of the object, with the view to the north, at it.
            local at = ref.position
            tes3.player.position = tes3vector3.new(at.x, at.y - (spec.tourDistance or 300), at.z)
            tes3.player.orientation = tes3vector3.new(0, 0, 0)
            scenario.after(spec.tourSeconds or 1.5, function()
                if each then
                    each(index, ref, function() stop(index + 1) end)
                else
                    stop(index + 1)
                end
            end)
        end
        stop(1)
    end

    -- The living actor that is nearest to the player, in the cells that the game has loaded.
    -- Nil when there is none.
    function scenario.nearestActor()
        local best, best_distance = nil, math.huge
        local from = tes3.player.position
        for _, cell in ipairs(tes3.getActiveCells()) do
            for ref in cell:iterateReferences({ tes3.objectType.npc, tes3.objectType.creature }, false) do
                if ref ~= tes3.player and ref.sceneNode and ref.mobile and not ref.mobile.isDead then
                    local distance = ref.position:distance(from)
                    if distance < best_distance then best, best_distance = ref, distance end
                end
            end
        end
        return best
    end

    -- Turns the player to an actor. With `move`, or when the actor is more than two times
    -- `distance` away, it also puts the player at `distance` in front of the actor.
    function scenario.faceActor(actor, distance, move)
        local at = actor.position
        local from = tes3.player.position
        if move or at:distance(from) > distance * 2 then
            local facing = actor.facing
            from = tes3vector3.new(at.x + math.sin(facing) * distance, at.y + math.cos(facing) * distance, at.z)
            tes3.positionCell({
                reference = tes3.player,
                -- Without the cell, the game puts the player into the exterior.
                cell = actor.cell,
                position = from,
                orientation = tes3vector3.new(0, 0, math.atan2(at.x - from.x, at.y - from.y)),
                suppressFader = true,
                teleportCompanions = false,
            })
        else
            tes3.player.orientation = tes3vector3.new(0, 0, math.atan2(at.x - from.x, at.y - from.y))
        end
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
            -- The game shows a box for each problem in the data, for example a texture that
            -- a test mesh does not have, and waits for an answer. This is the flag that the
            -- answer "yes to all" sets: the game then takes "yes" and shows no box. The
            -- warnings continue to go to Warnings.txt.
            mwse.memory.writeByte({ address = 0x7C8F72, byte = 1 })
            -- The actors stand still: two pictures of one place then show the same thing. A
            -- measurement keeps the AI (`spec.keepAI`, the runner's --keep-ai): actors with no
            -- AI do not walk, turn or fight, and their cost is not that of a game.
            local ai = "kept on"
            if not spec.keepAI then
                ai = tostring(tes3.runLegacyScript({ command = "ToggleAI" }))
            end
            scenario.say("save loaded, cell '%s', god mode on, AI toggled %s", tes3.player.cell.editorName, ai)
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
