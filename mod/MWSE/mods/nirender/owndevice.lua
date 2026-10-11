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
            scenario.goTo(spec.exterior)
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
            say("after %s: %s cell '%s'", this[1], nirender.captureStatus(), tes3.player and tes3.player.cell.editorName or "?")
            nirender.streamShot(string.format("%s/host-%s.png", spec.dir, this[1]))
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
