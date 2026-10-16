---@diagnostic disable
-- The in-game part of tools/run_own_device.py: the game with the d3d8.dll of this repository
-- in its folder. The game has no Direct3D device. main.lua put the Rust code on before the
-- game made its renderer, and the host program draws the only picture.
--
-- The scenario loads a save and asks the host program for a picture after each step.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "owndevice")
    local say, after = scenario.say, scenario.after

    local function inventory(on)
        if on then
            for _, name in ipairs({ "MenuInventory", "MenuMap", "MenuStat", "MenuMagic" }) do
                local shown = tes3ui.findMenu(name)
                if shown then
                    shown.visible = true
                    shown.disabled = false
                    shown:updateLayout()
                end
            end
            tes3ui.enterMenuMode("MenuInventory")
        else
            tes3ui.leaveMenuMode()
        end
    end

    -- Holds every animation of a scene node at one moment, so that the two renderers see the
    -- same moment of an exhibit: each time controller of the node, of its children and of
    -- their properties gets frequency 0 and a fixed phase in its key range. A particle
    -- system starts again instead, 3 seconds before its current time, so that it is in
    -- the middle of its life at the picture. Returns the number of controllers.
    local function holdAnimation(node)
        local count = 0
        local function holdChain(c)
            while c do
                local particles = pcall(function() return c:isInstanceOfType(ni.type.NiParticleSystemController) end)
                    and c:isInstanceOfType(ni.type.NiParticleSystemController)
                if particles then
                    local now = tonumber(c.lastTime) or 0
                    c:stop()
                    c:start(now - 3)
                else
                    local low, high = tonumber(c.lowKeyFrame) or 0, tonumber(c.highKeyFrame) or 0
                    c.frequency = 0
                    c.phase = low + (high - low) * 0.37
                end
                count = count + 1
                c = c.nextController
            end
        end
        local function visit(obj)
            if not obj then return end
            holdChain(obj.controller)
            for _, name in ipairs({ "alphaProperty", "materialProperty", "texturingProperty", "vertexColorProperty", "stencilProperty", "zBufferProperty", "fogProperty" }) do
                local ok, prop = pcall(function() return obj[name] end)
                if ok and prop then holdChain(prop.controller) end
            end
            local ok, children = pcall(function() return obj.children end)
            if ok and children then
                for i = 1, #children do visit(children[i]) end
            end
        end
        pcall(visit, node)
        return count
    end

    -- A console command of the engine that switches a debug overlay or a part of the scene
    -- (engine-debug-toggles-inventory.md): two steps, the command on and the picture, then
    -- the same command again so that the steps after it see the game as it was. The pair is
    -- one entry of the list of steps (a call in the middle of a table constructor gives one
    -- value); the list is flattened below.
    -- The Lua of the run (`--lua-file` of the runner), a little after the player arrives at
    -- a place: for a test that needs a state that a mod makes from Lua, for example the
    -- variables of a post shader.
    local function runLua()
        if not spec.lua then return end
        after(2.5, function()
            local chunk, problem = loadstring(spec.lua)
            if not chunk then
                say("lua: does not compile: %s", tostring(problem))
                return
            end
            local ok, result = pcall(chunk)
            say("lua: %s", ok and ("ran " .. tostring(result)) or ("failed: " .. tostring(result)))
        end)
    end

    local function toggle(name, command, seconds)
        local function run()
            say("%s: %s %s", name, command, tostring(tes3.runLegacyScript({ command = command })))
        end
        local function on()
            run()
            -- A capture of two frames with the toggle in effect, for the replay of the
            -- records against the draws of the device (tools/replay.py): the engine's own
            -- draws are the reference of a debug overlay, not MGE XE on Direct3D 9.
            after(1.0, function()
                if nirender.ownDevice() then
                    local ok, problem = nirender.capture(string.format("%s/%s.nircap", spec.dir, name), 0, 2)
                    say("%s: capture %s %s", name, tostring(ok), tostring(problem))
                end
            end)
        end
        return { pair = true, { name, seconds or 2.5, on }, { name .. "-off", 0.5, run } }
    end

    -- The step tour is complete when its cells are done.
    local tourDone = false
    local steps = {
        { "world", 1.0, function() end },
        -- The zoom of MGE XE, when MGE XE is there and runs its frame logic without a device:
        -- the picture of the step "zoom" is the middle of the picture of the step "world",
        -- three times as large.
        { "zoom", 2.0, function()
            if not (mge and mge.camera) then return say("zoom: no mge.camera") end
            mge.camera.zoomEnable = true
            mge.camera.zoom = 3
            say("zoom: enable %s, zoom %s, screen %s x %s", tostring(mge.camera.zoomEnable), tostring(mge.camera.zoom),
                tostring(mge.getScreenWidth and mge.getScreenWidth()), tostring(mge.getScreenHeight and mge.getScreenHeight()))
        end },
        { "zoom-off", 1.0, function()
            if not (mge and mge.camera) then return end
            mge.camera.zoom = 1
            mge.camera.zoomEnable = false
        end },
        { "inventory", 2.0, function()
            inventory(true)
            after(0.5, function() tes3ui.updateInventoryCharacterImage() end)
        end },
        -- The picture of the character with a blended body: a chameleon effect gives the
        -- materials of the player an alpha under 1. The Code Patch draws such a part into
        -- the picture in a way of its own (patchDrawIndexedPrimitive, see the plan). The
        -- capture has the frame that draws the picture.
        { "ghost", 5.0, function()
            -- The game applies a magic effect when its time runs: not in the menu mode.
            inventory(false)
            tes3.applyMagicSource({ reference = tes3.player, name = "nirender test", bypassResistances = true,
                effects = { { id = tes3.effect.chameleon, min = 75, max = 75, duration = 600 } } })
            after(2.0, function()
                say("ghost: chameleon %s", tostring(tes3.mobilePlayer.chameleon))
                inventory(true)
                after(1.0, function()
                    if nirender.ownDevice() then
                        local ok, problem = nirender.capture(string.format("%s/ghost.nircap", spec.dir), 0, 3)
                        say("ghost: capture %s %s", tostring(ok), tostring(problem))
                    end
                    tes3ui.updateInventoryCharacterImage()
                end)
            end)
        end },
        { "ghost-off", 3.0, function()
            inventory(false)
            tes3.removeEffects({ reference = tes3.player, effect = tes3.effect.chameleon })
            after(1.5, function()
                inventory(true)
                after(0.5, function() tes3ui.updateInventoryCharacterImage() end)
            end)
        end },
        { "interior", spec.settleSeconds or 6, function()
            inventory(false)
            -- The hour and the weather of the run, when the spec asks for them: for an
            -- interior that behaves as an exterior. The save's otherwise.
            if spec.hour then
                local hour = tes3.findGlobal("GameHour")
                if hour then hour.value = spec.hour end
            end
            scenario.goTo(spec.interior)
            runLua()
            if spec.weather then
                after(2, function()
                    local weather = tes3.worldController.weatherController
                    pcall(function() weather:switchImmediate(spec.weather) weather:updateVisuals() end)
                end)
            end
            -- A place in the cell (x,y,z) and a heading, as in the exterior step; or the
            -- heading alone: the player turns after the cell loads.
            local x, y, z = tostring(spec.position or ""):match("([-%d%.]+)%s*,%s*([-%d%.]+)%s*,%s*([-%d%.]+)")
            if x then
                after(1.5, function()
                    local yaw = math.rad(tonumber(spec.yaw) or 0)
                    tes3.positionCell({ reference = tes3.player, cell = spec.interior, position = tes3vector3.new(tonumber(x), tonumber(y), tonumber(z)), orientation = tes3vector3.new(0, 0, yaw) })
                    say("position: %s,%s,%s yaw %s", x, y, z, tostring(spec.yaw))
                end)
            elseif spec.findDry then
                -- Above the water of the cell: the nearest spot around the player where the
                -- floor is above the water level, found with rays onto the objects of the
                -- cell; the player looks back at the spot of the entry, where the water is.
                after(1.5, function()
                    local from = tes3.player.position
                    local level = tes3.player.cell.waterLevel or 0
                    local found
                    for ring = 1, 24 do
                        local r = ring * 128
                        for a = 0, 15 do
                            local x, y = from.x + r * math.cos(a * math.pi / 8), from.y + r * math.sin(a * math.pi / 8)
                            local hit = tes3.rayTest({ position = tes3vector3.new(x, y, level + 1500), direction = tes3vector3.new(0, 0, -1), root = tes3.game.worldObjectRoot })
                            if hit and hit.intersection and hit.intersection.z > level + 10 then
                                found = tes3vector3.new(x, y, hit.intersection.z + 80)
                                break
                            end
                        end
                        if found then break end
                    end
                    if found then
                        local yaw = math.atan2(from.x - found.x, from.y - found.y)
                        tes3.positionCell({ reference = tes3.player, cell = spec.interior, position = found, orientation = tes3vector3.new(0, 0, yaw) })
                        say("find dry: at %.0f,%.0f,%.0f yaw %.0f, water level %.0f", found.x, found.y, found.z, math.deg(yaw), level)
                    else
                        say("find dry: no floor above the water level %.0f within 3072 units", level)
                    end
                end)
            elseif spec.yaw then
                after(1.5, function()
                    local o = tes3.player.orientation
                    tes3.player.orientation = tes3vector3.new(o.x, o.y, math.rad(tonumber(spec.yaw)))
                end)
            end
        end },
        -- The lights of the scene off, in the interior, where they make the picture.
        toggle("lights", "ToggleLights"),
        { "exterior", spec.settleSeconds or 6, function()
            -- The debug overlay of the renderer, when the spec asks for it.
            if spec.overlay and nirender.overlay then nirender.overlay(true) end
            -- Noon, so that the pictures of the exterior have the light of the day.
            local hour = tes3.findGlobal("GameHour")
            if hour then hour.value = spec.hour or 12 end
            scenario.goTo(spec.exterior)
            runLua()
            -- The weather of the run, when the spec asks for one (the game's otherwise: it
            -- is random, so the pictures of two runs can differ). 0 clear to 9 blizzard.
            if spec.weather then
                after(2, function()
                    local weather = tes3.worldController.weatherController
                    pcall(function() weather:switchImmediate(spec.weather) weather:updateVisuals() end)
                end)
            end
            -- Under the water: the nearest spot around the player where the land is 170 units
            -- or more under the water level, found with rays onto the landscape.
            if spec.findWater then
                after(1.5, function()
                    local from = tes3.player.position
                    local found
                    for ring = 1, 24 do
                        local r = ring * 256
                        for a = 0, 15 do
                            local x, y = from.x + r * math.cos(a * math.pi / 8), from.y + r * math.sin(a * math.pi / 8)
                            local hit = tes3.rayTest({ position = tes3vector3.new(x, y, 4000), direction = tes3vector3.new(0, 0, -1), root = tes3.game.worldLandscapeRoot })
                            if hit and hit.intersection and hit.intersection.z < -170 then
                                found = tes3vector3.new(x, y, -130)
                                break
                            end
                        end
                        if found then break end
                    end
                    if found then
                        local yaw = math.rad(tonumber(spec.yaw) or 0)
                        tes3.positionCell({ reference = tes3.player, position = found, orientation = tes3vector3.new(0, 0, yaw) })
                        say("find water: at %.0f,%.0f,%.0f", found.x, found.y, found.z)
                    else
                        say("find water: no spot within 6144 units")
                    end
                end)
            end
            -- A heading alone: the player turns after the cell loads.
            if spec.yaw and not (spec.position and spec.position ~= "") and not spec.findWater then
                after(1.5, function()
                    local o = tes3.player.orientation
                    tes3.player.orientation = tes3vector3.new(o.x, o.y, math.rad(tonumber(spec.yaw)))
                    say("yaw: %s", tostring(spec.yaw))
                end)
            end
            -- A place in the cell, and a heading: for a picture of one thing, the same each time.
            if spec.position and spec.position ~= "" then
                local x, y, z = tostring(spec.position):match("([-%d%.]+)%s*,%s*([-%d%.]+)%s*,%s*([-%d%.]+)")
                if not x then
                    x, y = tostring(spec.position):match("([-%d%.]+)%s*,%s*([-%d%.]+)")
                end
                after(1.5, function()
                    local yaw = math.rad(tonumber(spec.yaw) or 0)
                    if not z then
                        -- Without a height: onto the landscape, found with a ray.
                        local hit = tes3.rayTest({ position = tes3vector3.new(tonumber(x), tonumber(y), 8000), direction = tes3vector3.new(0, 0, -1), root = tes3.game.worldLandscapeRoot })
                        z = hit and hit.intersection and (hit.intersection.z + 80) or 0
                    end
                    tes3.positionCell({ reference = tes3.player, position = tes3vector3.new(tonumber(x), tonumber(y), tonumber(z)), orientation = tes3vector3.new(0, 0, yaw) })
                    say("position: %s,%s,%s yaw %s", x, y, z, tostring(spec.yaw))
                end)
            end
        end },
        -- The frame rate at one place: the frames that the game gives to Lua in 10 seconds,
        -- with the player at rest in the exterior cell. The same step runs with Direct3D 9
        -- (tools/run_own_device.py --reference), for a number to compare with.
        { "rate", 10.5, function()
            local frames, from = 0, os.clock()
            local function count()
                frames = frames + 1
                if os.clock() - from >= 10 then
                    event.unregister("enterFrame", count)
                    say("rate: %.1f frames in a second at '%s'", frames / (os.clock() - from), tes3.player.cell.editorName)
                end
            end
            event.register("enterFrame", count)
        end },
        -- Each weather of the game in turn, with the frame rate of each over 4 seconds: a
        -- cost of one weather (its sky, its fog, its particles) shows against the others.
        -- Two passes, 0 to 9 then 9 to 0: a trend of the run (the game still settles in a
        -- large modlist) shows as a difference between the passes, a weather as a cost in
        -- both.
        { "weathers", 128, function()
            local weather = tes3.worldController.weatherController
            local order = { 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0 }
            local at = 1
            local function measure()
                local index = order[at]
                if not index then
                    say("weathers: done")
                    return
                end
                pcall(function() weather:switchImmediate(index) weather:updateVisuals() end)
                after(2, function()
                    local frames, from = 0, os.clock()
                    local function count()
                        frames = frames + 1
                        if os.clock() - from >= 4 then
                            event.unregister("enterFrame", count)
                            say("rate: %.1f frames in a second at '%s' weather %d pass %d", frames / (os.clock() - from), tes3.player.cell.editorName, index, at <= 10 and 1 or 2)
                            at = at + 1
                            measure()
                        end
                    end
                    event.register("enterFrame", count)
                end)
            end
            measure()
        end },
        -- The tour of a test level: in each cell of spec.tourCells, in front of each object
        -- whose id starts with spec.tour, two frames into the capture file of the cell. The
        -- file holds between the stops. tools/validate_tour.py replays the stops.
        -- With spec.tourDual, each stop gives two frames in mode gpu and two in mode off,
        -- with the time of the game held between them: the two paths at one moment.
        { "tour", 1.0, function()
            local cells = {}
            if spec.tourDual then say("tour: dual (mode gpu, then mode off, at each stop)") end
            for name in tostring(spec.tourCells or ""):gmatch("[^;]+") do cells[#cells + 1] = name end
            local function cell(index)
                local name = cells[index]
                if not name then
                    tourDone = true
                    return say("tour: done, %d cells", #cells)
                end
                scenario.goTo(name)
                -- The Lua of the run (`--lua-file`) at the first cell of the tour too: a run
                -- with the tour step alone never reaches the interior or exterior steps.
                if index == 1 then runLua() end
                after(spec.settleSeconds or 6, function()
                    -- With spec.tourExtra, the test NIFs of the flag support (made by
                    -- tools/make_test_nifs.py) stand in the first cell, in a row east of the
                    -- easternmost exhibit, and become stops of the tour. The player passes
                    -- through walls there: the row may leave the tiled floor.
                    if index == 1 and spec.tourExtra then
                        local east = nil
                        for ref in tes3.player.cell:iterateReferences() do
                            local id = ref.object.id:lower()
                            if id:sub(1, 5) == "@nif_" and not id:find("tile", 1, true) and (not east or ref.position.x > east.position.x) then east = ref end
                        end
                        local base = east and east.position or tes3.player.position
                        -- A steady light of the game for the entries that need light (the
                        -- level lights almost nothing): the first light object with no
                        -- flicker or pulse and a radius of 256 or more.
                        local steady = nil
                        for light in tes3.iterateObjects(tes3.objectType.light) do
                            -- A radius up to 1024: a mod's scripted light of radius 8000 (AB_light@_Lightning) lit
                            -- nothing on 2026-10-16.
                            if light.radius and light.radius >= 256 and light.radius <= 1024 and not light.flickers and not light.flickersSlowly and not light.pulses
                                and not light.pulsesSlowly and not light.isNegative and not light.isOffByDefault and not light.canCarry
                                and light.mesh and light.mesh ~= "" then
                                -- The game's own lights first (a mod's can be blue or scripted), then
                                -- the largest radius.
                                local own = light.sourceMod == "Morrowind.esm"
                                local best = steady and steady.sourceMod == "Morrowind.esm"
                                if not steady or (own and not best) or (own == best and light.radius > steady.radius) then
                                    steady = light
                                end
                            end
                        end
                        local placed = 0
                        for i, entry in ipairs(require("nirender.test_nifs")) do
                            local object = tes3.getObject(entry.id) or tes3.createObject({ objectType = tes3.objectType.static, id = entry.id, mesh = entry.mesh })
                            if object then
                                local at = tes3vector3.new(base.x + 400 * i, base.y, base.z + 80)
                                tes3.createReference({ object = object, cell = tes3.player.cell, position = at, orientation = tes3vector3.new(0, 0, 0) })
                                if entry.light and steady then
                                    -- In front, left and above the object, at half the light's radius (a
                                    -- first try at 291 units was outside a radius of 256).
                                    local r = steady.radius
                                    tes3.createReference({ object = steady, cell = tes3.player.cell,
                                        position = tes3vector3.new(at.x - 0.25 * r, at.y - 0.35 * r, at.z + 0.25 * r), orientation = tes3vector3.new(0, 0, 0) })
                                end
                                placed = placed + 1
                            end
                        end
                        say("tour: the steady light is %s (radius %s)", steady and steady.id or "none", steady and tostring(steady.radius) or "-")
                        tes3.mobilePlayer.movementCollision = false
                        say("tour: %d test NIFs placed east of %s at %s", placed, east and east.object.id or "the player", tostring(base))
                    end
                    -- The capture is of the stream of the own device. The reference run with
                    -- MGE XE on Direct3D 9 has none: it takes a picture of each stop instead.
                    local own = nirender.ownDevice()
                    local path = string.format("%s/tour-%d.nircap", spec.dir, index)
                    say("tour: cell %d '%s' into %s: %s", index, name, path, tostring(own and nirender.capture(path, 0, 0)))
                    scenario.tour(1, function()
                        if own then nirender.captureEnd() end
                        after(0.5, function() cell(index + 1) end)
                    end, function(stop, ref, go)
                        -- The animations of the object at one moment, the same in each run.
                        local held = holdAnimation(ref.sceneNode)
                        after(0.5, function()
                            if own and spec.tourDual then
                                -- The time of the game held: the particles stay where they
                                -- are. Two frames in mode gpu (the helper draws the retained
                                -- shapes), then two in mode off (the engine displays every
                                -- shape), then mode gpu again and the time runs.
                                tes3.worldController.simulationTimeScalar = 0
                                nirender.captureFrames(2)
                                say("tour: cell %d stop %d %s (%d controllers held) dual", index, stop, ref.object.id, held)
                                after(0.4, function()
                                    nirender.retained("off")
                                    after(0.6, function()
                                        nirender.captureFrames(2)
                                        after(0.4, function()
                                            nirender.retained("gpu")
                                            tes3.worldController.simulationTimeScalar = 1
                                            after(0.3, go)
                                        end)
                                    end)
                                end)
                                return
                            end
                            if own then nirender.captureFrames(2) end
                            say("tour: cell %d stop %d %s (%d controllers held)", index, stop, ref.object.id, held)
                            if not own and mge and mge.saveScreenshot then
                                -- The reference with MGE XE on Direct3D 9: a picture of each stop, with
                                -- the name that tools/validate_tour.py gives the replayed picture.
                                local name = string.format("%d-%02d-%s.png", index, stop, ref.object.id:gsub("[^%w_]", "_"))
                                lfs.mkdir(spec.dir .. "/ref")
                                mge.saveScreenshot({ path = spec.dir .. "/ref/" .. name, captureWithUI = true })
                            end
                            after(0.3, go)
                        end)
                    end)
                end)
            end
            cell(1)
        end, function() return tourDone end },
        -- Lines: the cell borders of the game (the console command ToggleBorders makes a
        -- NiLines for each loaded exterior cell, `Land::setDisplayCellBorders`). The game
        -- draws them with `RenderLines`, which no mesh of the game reaches.
        { "lines", 2.0, function()
            say("lines: cell borders %s", tostring(tes3.runLegacyScript({ command = "ToggleBorders" })))
        end },
        -- A capture file of some frames at rest, for the offline replay: the same picture
        -- from the same file each time, for a check of a change of the renderer.
        { "capture", 2.5, function()
            local ok, problem = nirender.capture(spec.dir .. "/exterior.nircap", 0, spec.captureFrames or 30)
            say("capture: %s %s", tostring(ok), tostring(problem))
        end },
        -- The sky off in the exterior (the console command ToggleSky: `WeatherController`
        -- 0x440870 culls the sky root; the same function hides the sky of a plain interior
        -- cell). The world scene then has no sky draw, and the scene packet says so.
        toggle("sky", "ToggleSky"),
        -- The debug overlays of the engine, each a draw path of its own: the collision boxes
        -- of the actors (`DataHandler::showActorCollisionBoxes` 0x48DCC0, the shared
        -- wireframe property), everything in wireframe (the same property), the collision
        -- grid (`updateCollisionGridDebugOverlay` 0x488590, shapes and a text for each cell),
        -- the path grid (NiTriShapes of the `PathGridNode`s).
        toggle("boxes", "ToggleCollisionBoxes"),
        toggle("wireframe", "ToggleWireframe"),
        toggle("grid", "ToggleGrid"),
        toggle("pathgrid", "TogglePathGrid"),
        -- The parts of the scene off: the world object roots (the sky alone), the water.
        toggle("noworld", "ToggleWorld"),
        toggle("nowater", "ToggleWater"),
        -- The local map without the fog of war: the map texture of the HUD is a render to
        -- texture (`ui_MenuMap_toggleFogOfWar` 0x5EB340).
        toggle("fog", "ToggleFogOfWar"),
        -- A film (the film player draws with `DrawPrimitiveUP`): the film holds the main loop
        -- of the game, so the host program takes the picture of its first frame on its own
        -- (`--shot-film`, host-film-frame.png); the picture of the step is the frame after
        -- the film.
        { "film", 4.0, function()
            -- PlayBink takes the file and the interrupt flag (1: Escape ends the film); the
            -- compiler refuses the command without the flag ("You need to enter a flag for
            -- interupt", Warnings.txt). The call holds the main loop while the film plays:
            -- the time of the call says whether it played.
            local from = os.clock()
            local ok = tes3.runLegacyScript({ command = 'PlayBink "mw_logo.bik" 1' })
            say("film: %s in %.2f s", tostring(ok), os.clock() - from)
        end },
        -- The same place with the camera turned to the two sides: a thing in the two
        -- pictures must have the same colour. The fog of a thing does not change with the
        -- direction of the camera.
        { "yaw-a", 2.0, function()
            local o = tes3.player.orientation
            tes3.player.orientation = tes3vector3.new(o.x, o.y, o.z + 0.3)
        end },
        { "yaw-b", 2.0, function()
            local o = tes3.player.orientation
            tes3.player.orientation = tes3vector3.new(o.x, o.y, o.z - 0.6)
        end },
        { "yaw-off", 0.5, function()
            local o = tes3.player.orientation
            tes3.player.orientation = tes3vector3.new(o.x, o.y, o.z + 0.3)
        end },
        -- The player goes north over the borders of exterior cells, a small step in each
        -- frame, as on foot but faster. The game then loads the cells in front of the player
        -- on its second thread while the main thread draws.
        { "walk", spec.walkSeconds or 25, function()
            local last = os.clock()
            local from = tes3.player.cell.editorName
            scenario.eachFrame = function(now)
                local step = math.min(now - last, 0.1) * (spec.walkSpeed or 2500)
                last = now
                local at = tes3.player.position
                tes3.player.position = tes3vector3.new(at.x, at.y + step, at.z + 0)
            end
            say("walk from '%s'", from)
        end },
        { "walk-end", 3.0, function()
            scenario.eachFrame = nil
            say("walk to '%s'; calls from other threads: %s", tes3.player.cell.editorName, (nirender.threadCensus():gsub("\n", "; ")))
        end },
    }

    -- The shader batch of the run (`--shader-batch` of the runner): a Lua file that gives a
    -- list of sets of post shaders, `{ name = "lens", { "Lens Effects", variable = value } }`.
    -- A set can have `run`, a function that the step calls first, and `seconds`, its wait.
    -- One step for each set: the shaders of the set before go off, those of this set are
    -- loaded, get their variables and go on, and the picture is `sh-<name>`. A set without
    -- a shader is the picture without post shaders. One start of the game then gives the
    -- pictures of each shader.
    if spec.shaderBatch then
        local chunk, problem = loadstring(spec.shaderBatch)
        local ok, sets = pcall(chunk or function() error(problem) end)
        if not ok or type(sets) ~= "table" then
            say("shader batch: not read: %s", tostring(sets))
            sets = {}
        end
        local on = {}
        local function allOff()
            for _, shader in ipairs(on) do shader.enabled = false end
            on = {}
        end
        for _, set in ipairs(sets) do
            steps[#steps + 1] = { "sh-" .. tostring(set.name), set.seconds or 1.5, function()
                allOff()
                -- A set can have a function `run`: for a state that a mod makes itself (a
                -- menu of the mod that turns its shaders on). A text that it returns goes
                -- into the line: "click X Y of WxH" or "drag X1 Y1 X2 Y2 of WxH" makes the
                -- runner use the real mouse there (the question box of a load only: it puts
                -- the cursor of Windows there). In menu mode the cursor of the game moves
                -- only with the motion of the mouse: "menu-click X Y" and "menu-drag X1 Y1
                -- X2 Y2" give the place in the coordinates of tes3.getCursorPosition (0, 0
                -- in the middle, y up). The runner moves the mouse until the cursor of the
                -- game, which the lines "cursor X Y" report, is there, and presses.
                if set.run then
                    local ok, result = pcall(set.run)
                    say("shader batch: %s: run %s %s", tostring(set.name), tostring(ok), tostring(result or ""))
                    if ok and type(result) == "string" and result:find("^menu%-") then
                        timer.start({ type = timer.real, duration = 0.1, iterations = 150, callback = function()
                            local cursor = tes3.getCursorPosition()
                            say("cursor %d %d", cursor.x, cursor.y)
                        end })
                    end
                end
                for _, item in ipairs(set) do
                    local shader = mge.shaders.find({ name = item[1] }) or mge.shaders.load({ name = item[1] })
                    if shader then
                        for variable, value in pairs(item) do
                            if type(variable) == "string" then shader[variable] = value end
                        end
                        shader.enabled = true
                        on[#on + 1] = shader
                    else
                        say("shader batch: %s: no shader %s", tostring(set.name), tostring(item[1]))
                    end
                end
            end }
        end
        steps[#steps + 1] = { "sh-off", 0.5, allOff }
    end

    -- The pairs of toggle() become their two steps, in place.
    local flat = {}
    for _, s in ipairs(steps) do
        if s.pair then
            flat[#flat + 1] = s[1]
            flat[#flat + 1] = s[2]
        else
            flat[#flat + 1] = s
        end
    end
    steps = flat

    -- With spec.steps (names with commas), only those steps: a quick run for one thing.
    if spec.steps and spec.steps ~= "" then
        local wanted = {}
        for name in tostring(spec.steps):gmatch("[^,%s]+") do wanted[name] = true end
        local kept = {}
        for _, s in ipairs(steps) do
            -- `shaders` is each step of the shader batch.
            if wanted[s[1]] or (wanted.shaders and s[1]:sub(1, 3) == "sh-") then kept[#kept + 1] = s end
        end
        steps = kept
        say("steps: %s (%d)", tostring(spec.steps), #steps)
    end

    local function step(index)
        local this = steps[index]
        if not this then
            say("steps done: %s", nirender.captureStatus())
            -- The end of the stream is the end of the picture. The host program then says
            -- what it drew.
            nirender.captureStop()
            return after(1, scenario.finish)
        end
        local ok, problem = pcall(this[3])
        say("step %s: %s %s", this[1], tostring(ok), ok and "" or tostring(problem))
        -- A step with a fourth field is complete when that function says so: after its time,
        -- the scenario waits for it, one second at a time.
        local function when_done(continue)
            if this[4] and not this[4]() then return after(1, function() when_done(continue) end) end
            continue()
        end
        after(this[2], function() when_done(function()
            local p = tes3.player and tes3.player.position
            -- The host program prints its counts of this step at the mark.
            if nirender.streamMark then nirender.streamMark("after " .. this[1]) end
            -- Only when asked (--port-times): the first call turns the timing of every live
            -- port call on, which a profile of the run would then carry (2026-10-16).
            if spec.portTimes and nirender.portTimes then say("ports %s: %s", this[1], nirender.portTimes(12)) end
            -- The hour of the game at the picture: the light of the sun follows it, so two
            -- pictures compare only at the same hour.
            local hour = tes3.findGlobal("GameHour")
            say("after %s: %s cell '%s' at %s hour %.4f", this[1], nirender.captureStatus(), tes3.player and tes3.player.cell.editorName or "?",
                p and string.format("%.0f,%.0f,%.0f yaw %.0f", p.x, p.y, p.z, math.deg(tes3.player.orientation.z)) or "?", hour and hour.value or -1)
            -- The fog of the scene graph (MGE XE changes its colour for the scattering) and
            -- the fog colour of the weather.
            local fogOk, fogProblem = pcall(function()
                local fog, weather = tes3.game.fogProperty, tes3.worldController.weatherController.currentFogColor
                -- The bytes of the colour as the game keeps them: blue, green, red, alpha.
                say("fog %s: scene %s %s %s %s density %.4f, weather %s", this[1], tostring(fog.color[1]), tostring(fog.color[2]), tostring(fog.color[3]),
                    tostring(fog.color[4]), fog.density, tostring(weather))
            end)
            if not fogOk then say("fog %s: not read: %s", this[1], tostring(fogProblem)) end
            -- The post shaders that are on, and the lighting mode of MGE XE: two pictures
            -- compare only with the same chain and mode (owner, 2026-10-16).
            if mge and mge.shaders and mge.shaders.list then
                local names = {}
                for _, shader in ipairs(mge.shaders.list) do
                    if shader.enabled then names[#names + 1] = shader.name end
                end
                say("shaders %s: %s; lighting mode %s", this[1], #names > 0 and table.concat(names, ", ") or "none",
                    mge.getLightingMode and tostring(mge.getLightingMode()) or "?")
            end
            if nirender.captureStatus():sub(1, 3) == "off" and mge and mge.saveScreenshot then
                -- No stream: the run is the reference with MGE XE on Direct3D 9. The picture
                -- is that of MGE XE.
                mge.saveScreenshot({ path = string.format("%s/ref-%s.png", spec.dir, this[1]), captureWithUI = true })
            else
                nirender.streamShot(string.format("%s/host-%s.png", spec.dir, this[1]))
            end
            after(0.8, function() step(index + 1) end)
        end) end)
    end

    say("own device: %s", tostring(nirender.ownDevice()))
    -- The game gives no frame to Lua before the main menu is there: it shows its films and
    -- loads its data first. The scenario starts at the first frame of the main menu, takes a
    -- picture of the menu, and then loads the save.
    spec.menuSeconds = 8
    local function at_main_menu()
        event.unregister("enterFrame", at_main_menu)
        say("main menu: %s", nirender.captureStatus())
        after(5, function() nirender.streamShot(string.format("%s/host-menu.png", spec.dir)) end)
        -- A capture file of the load of the save, from the main menu: the loading screen of
        -- the game is a screen polygon (`RenderScreenPoly`), which no place of the game has.
        if spec.captureMenu and spec.captureMenu > 0 then
            after(5.5, function()
                local ok, problem = nirender.capture(spec.dir .. "/menu.nircap", 0, spec.captureMenu)
                say("capture of the menu and the load: %s %s", tostring(ok), tostring(problem))
            end)
        end
        scenario.start(function()
            after(spec.settleSeconds or 6, function() step(1) end)
        end)
    end
    event.register("enterFrame", at_main_menu)
end
