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
            if spec.callers then nirender.deviceCallers(true) end
            after(spec.seconds, function()
                if spec.callers then
                    for line in tostring(nirender.deviceCallers(false)):gmatch("[^\n]+") do
                        say("callers|%s|%s|%s", where, label, line)
                    end
                end
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
        nirender.linkUseKinds(entry[2] ~= 0 and (spec.kinds or 0) or 0)
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
        -- The state packets come from the Rust code only.
        if spec.factsNeedRust then nirender.setMode("*", "live") end
        if spec.holdMenu then
            -- A menu of the game, for example "MenuInventory": the inventory, the map, the
            -- character image.
            -- The game shows these four menus together in the inventory mode. They must be
            -- visible before the start of menu mode.
            if spec.holdMenu == "MenuInventory" then
                for _, name in ipairs({ "MenuInventory", "MenuMap", "MenuStat", "MenuMagic" }) do
                    local shown = tes3ui.findMenu(name)
                    if shown then
                        shown.visible = true
                        shown.disabled = false
                        shown:updateLayout()
                    end
                    say("menu %s found: %s", name, tostring(shown ~= nil))
                end
            end
            say("menu %s: %s, menu mode %s", spec.holdMenu, tostring(tes3ui.enterMenuMode(spec.holdMenu)), tostring(tes3ui.menuMode()))
        else
            local menu = tes3ui.createMenu({ id = "nirender:hold", fixedFrame = true })
            menu:createLabel({ text = "nirender link test" })
            menu:updateLayout()
            tes3ui.enterMenuMode("nirender:hold")
        end
        -- The frame changes for a moment after the start of menu mode.
        after(3, function()
            pictures(where, { { "off1", 0 }, { "on", facts }, { "off2", 0 } }, 1, function()
                if spec.holdMenu then
                    mge.saveScreenshot({ path = string.format("%s/%d-menu.png", spec.shotDir, place), captureWithUI = true })
                end
                tes3ui.leaveMenuMode()
                local held = tes3ui.findMenu("nirender:hold")
                if held then held:destroy() end
                nirender.setMode("*", "off")
                done()
            end)
        end)
    end

    local function test_here(done)
        local where = tes3.player.cell.editorName
        place = place + 1
        if spec.travelOn and place > 1 then
            if spec.callers then
                for line in tostring(nirender.deviceCallers(false)):gmatch("[^\n]+") do
                    say("callers|%s|travel|%s", where, line)
                end
            end
            for line in nirender.linkCounters(true):gmatch("[^\n]+") do
                if line:find("^object_calls") or line:find("^frame_calls") or line:find("^fact texture_create") then
                    say("counters|%s|travel|%s", where, line)
                end
            end
        end
        nirender.linkUseFacts(0)
        nirender.linkUseKinds(0)
        if spec.hour or spec.moon then
            -- A clear sky, so that the moons are in view at night.
            local weather = tes3.worldController.weatherController
            pcall(function() weather:switchImmediate(0) weather:updateVisuals() end)
        end
        -- spec.modes limits the test to some of the modes: a quick run.
        local wanted = {}
        for _, mode in ipairs(spec.modes or { "off", "live" }) do wanted[mode] = true end
        local function step(mode, next_step)
            if not wanted[mode] then return next_step() end
            measure(where, mode, mode, next_step)
        end
        step("off", function()
            step("live", function()
                nirender.setMode("*", "off")
                if spec.modes and facts == 0 then return done() end
                picture_test(where, function()
                    if facts == 0 then return done() end
                    -- The game runs with the facts in use. It must continue to run, and the
                    -- counters must stay equal.
                    say("facts in use: mask %s", tostring(nirender.linkUseFacts(facts)))
                    say("kinds in use: %s", tostring(nirender.linkUseKinds(spec.kinds or 0)))
                    measure(where, spec.factsNeedRust and "live" or "off", "facts", function()
                        nirender.linkUseFacts(0)
                        nirender.linkUseKinds(0)
                        done()
                    end)
                end)
            end)
        end)
    end

    if spec.sweep then
        -- The camera turns and looks up, so that each part of the sky comes into view. The
        -- game does not draw a moon that is not in the view.
        scenario.eachFrame = function(now)
            if tes3.player then
                local pitch = spec.sweep * (0.9 + 0.5 * math.sin(now * 0.7))
                tes3.player.orientation = tes3vector3.new(pitch, 0, (now * 1.5) % (2 * math.pi))
            end
        end
    end

    -- Reports the state of a moon and returns its place in the world, or nil.
    local function moon_state(name)
        local moon = tes3.worldController.weatherController[name]
        local root = moon and moon.sceneMoonRoot
        if not root then
            say("moon %s: no scene node", name)
            return nil
        end
        local at = root.worldTransform.translation
        local parent = root.parent
        say("moon %s: game hour %.2f, day %s, fade in %s to %s, fade out %s to %s, weather %s", name,
            tes3.worldController.hour.value, tostring(tes3.worldController.daysPassed.value), tostring(moon.fadeInStart),
            tostring(moon.fadeInFinish), tostring(moon.fadeOutStart), tostring(moon.fadeOutFinish),
            tostring(tes3.worldController.weatherController.currentWeather.index))
        say("moon %s: phase %s, root culled %s, face culled %s, shadow culled %s, parent '%s', at %.0f %.0f %.0f", name,
            tostring(moon.phase), tostring(root.appCulled), tostring(moon.sceneMoonFace.appCulled),
            tostring(moon.sceneMoonShadow.appCulled), parent and tostring(parent.name) or "none", at.x, at.y, at.z)
        return at
    end

    if spec.moon then
        -- The camera looks at a moon. The game does not draw a moon that is not in the view.
        scenario.eachFrame = function(now)
            -- In menu mode the moon does not move. The camera stays, for the picture test.
            if not tes3.player or tes3.menuMode() then return end
            local moon = tes3.worldController.weatherController[spec.moon]
            local face = moon and moon.sceneMoonFace
            if not face then return end
            -- The root of a moon is at the camera. The bound of the face is where the moon is.
            local to = face.worldBoundOrigin - tes3.getPlayerEyePosition()
            local flat = math.sqrt(to.x * to.x + to.y * to.y)
            if flat + math.abs(to.z) < 100 then return end
            tes3.player.orientation = tes3vector3.new(-math.atan2(to.z, flat), 0, math.atan2(to.x, to.y))
        end
    end

    -- Goes through the hours of the night until the game shows the moon of the spec, then
    -- calls `done`. The game hides a moon that is under the horizon or that the weather hides.
    local function find_moon(done)
        if not spec.moon then return done() end
        local hours = { 18, 19, 20, 21, 22, 23, 0, 1, 2, 3, 4, 5, 6 }
        local function try(index)
            local hour = hours[index]
            if not hour then
                say("moon %s: the game did not show it at any hour of the night", spec.moon)
                moon_state(spec.moon)
                return done()
            end
            tes3.worldController.hour.value = hour
            local weather = tes3.worldController.weatherController
            pcall(function() weather:switchImmediate(0) weather:updateVisuals() end)
            after(0.5, function()
                local root = weather[spec.moon].sceneMoonRoot
                if root and not root.appCulled then
                    say("moon %s: in the sky at hour %d", spec.moon, hour)
                    moon_state(spec.moon)
                    return done()
                end
                try(index + 1)
            end)
        end
        try(1)
    end

    local function visit(index)
        local cell = spec.cells and spec.cells[index]
        -- The textures of the next cell come through the link. Only the Rust code does that.
        -- The change of cell, with its load screen, runs with the facts and kinds in use.
        if spec.travelOn and cell then
            nirender.setMode("*", "live")
            say("travel with facts %s kinds %s", tostring(nirender.linkUseFacts(facts)), tostring(nirender.linkUseKinds(spec.kinds or 0)))
            if spec.callers then nirender.deviceCallers(true) end
            nirender.linkCounters(true)
        end
        if spec.cameraCompare then
            say("camera space comparison: %s", tostring(nirender.cameraSpaceCompare(true)))
        end
        if spec.linkTextures then
            nirender.setMode("*", "live")
            say("textures through the link: %s", tostring(nirender.linkTextures(true)))
        end
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
            find_moon(function()
                test_here(function() visit(1) end)
            end)
        end)
    end)
end
