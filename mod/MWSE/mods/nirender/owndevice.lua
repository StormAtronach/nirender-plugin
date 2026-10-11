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
        { "interior", spec.settleSeconds or 6, function()
            inventory(false)
            scenario.goTo(spec.interior)
        end },
        { "exterior", spec.settleSeconds or 6, function()
            -- Noon, so that the pictures of the exterior have the light of the day.
            local hour = tes3.findGlobal("GameHour")
            if hour then hour.value = spec.hour or 12 end
            scenario.goTo(spec.exterior)
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
            -- A place in the cell, and a heading: for a picture of one thing, the same each time.
            if spec.position and spec.position ~= "" then
                local x, y, z = tostring(spec.position):match("([-%d%.]+)%s*,%s*([-%d%.]+)%s*,%s*([-%d%.]+)")
                after(1.5, function()
                    local yaw = math.rad(tonumber(spec.yaw) or 0)
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
        -- A capture file of some frames at rest, for the offline replay: the same picture
        -- from the same file each time, for a check of a change of the renderer.
        { "capture", 2.5, function()
            local ok, problem = nirender.capture(spec.dir .. "/exterior.nircap", 0, spec.captureFrames or 30)
            say("capture: %s %s", tostring(ok), tostring(problem))
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

    -- With spec.steps (names with commas), only those steps: a quick run for one thing.
    if spec.steps and spec.steps ~= "" then
        local wanted = {}
        for name in tostring(spec.steps):gmatch("[^,%s]+") do wanted[name] = true end
        local kept = {}
        for _, s in ipairs(steps) do
            if wanted[s[1]] then kept[#kept + 1] = s end
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
        after(this[2], function()
            local p = tes3.player and tes3.player.position
            say("after %s: %s cell '%s' at %s", this[1], nirender.captureStatus(), tes3.player and tes3.player.cell.editorName or "?",
                p and string.format("%.0f,%.0f,%.0f yaw %.0f", p.x, p.y, p.z, math.deg(tes3.player.orientation.z)) or "?")
            if nirender.captureStatus():sub(1, 3) == "off" and mge and mge.saveScreenshot then
                -- No stream: the run is the reference with MGE XE on Direct3D 9. The picture
                -- is that of MGE XE.
                mge.saveScreenshot({ path = string.format("%s/ref-%s.png", spec.dir, this[1]), captureWithUI = true })
            else
                nirender.streamShot(string.format("%s/host-%s.png", spec.dir, this[1]))
            end
            after(0.8, function() step(index + 1) end)
        end)
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
        scenario.start(function()
            after(spec.settleSeconds or 6, function() step(1) end)
        end)
    end
    event.register("enterFrame", at_main_menu)
end
