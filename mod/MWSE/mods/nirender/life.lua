---@diagnostic disable
-- The in-game part of tools/run_life.py: the life of the game after the plugin became the host
-- of its own render link. No call of the game gets to a D3D device; the host program draws
-- the only picture.
--
-- The scenario does the things that need more than draws: the character image and the map
-- of the inventory, the screenshot of the game, a save with its picture, changes of cell, and
-- a load. Before the change of host it takes a picture of the game for each view that does
-- not change (reference pictures); after each step it asks the host program for a picture.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "life")
    local say, after = scenario.say, scenario.after

    local function picture(name)
        return string.format("%s/%s.png", spec.dir, name)
    end

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

    -- The steps after the change of host. Each one is { name, seconds to wait, function }.
    local steps = {
        { "inventory", 2.0, function()
            inventory(true)
            after(0.5, function() tes3ui.updateInventoryCharacterImage() end)
        end },
        { "screenshot", 2.0, function()
            nirender.gameScreenshot()
        end },
        { "save", 3.0, function()
            inventory(false)
            tes3.saveGame({ file = "nirender_life", name = "nirender life" })
        end },
        { "interior", spec.settleSeconds or 6, function()
            scenario.goTo(spec.interior)
        end },
        { "interior-inventory", 2.0, function()
            inventory(true)
            after(0.5, function() tes3ui.updateInventoryCharacterImage() end)
        end },
        { "exterior", spec.settleSeconds or 6, function()
            inventory(false)
            scenario.goTo(spec.exterior)
        end },
        { "load", (spec.settleSeconds or 6) + 6, function()
            tes3.loadGame("nirender_life")
        end },
        { "load-inventory", 2.0, function()
            inventory(true)
            after(0.5, function() tes3ui.updateInventoryCharacterImage() end)
        end },
    }

    local function step(index)
        local this = steps[index]
        if not this then
            inventory(false)
            say("steps done: %s", nirender.captureStatus())
            nirender.captureStop()
            return after(2, scenario.finish)
        end
        local ok, problem = pcall(this[3])
        say("step %s: %s %s", this[1], tostring(ok), ok and "" or tostring(problem))
        after(this[2], function()
            local status = nirender.captureStatus()
            say("after %s: %s cell '%s'", this[1], status, tes3.player and tes3.player.cell.editorName or "?")
            if status:sub(1, 6) ~= "record" then
                say("ERROR the stream ended in the step %s", this[1])
                return scenario.finish()
            end
            nirender.streamShot(picture("host-" .. this[1]))
            after(0.8, function() step(index + 1) end)
        end)
    end

    scenario.start(function()
        after(spec.settleSeconds or 6, function()
            nirender.dumpDevice(spec.dir .. "/device.txt")
            if spec.dumpOnly then return scenario.finish() end
            local ok, message = nirender.link()
            say("link: %s (%s)", tostring(ok), message)
            if not ok then return scenario.finish() end
            nirender.setMode("*", "live")
            nirender.linkTextures(true)
            nirender.linkUseFacts(spec.facts)
            nirender.linkUseKinds(spec.kinds)
            -- The reference: the inventory as MGE XE draws it from the link.
            inventory(true)
            after(0.5, function() tes3ui.updateInventoryCharacterImage() end)
            after(2, function()
                mge.saveScreenshot({ path = picture("game-inventory"), captureWithUI = true })
                after(1, function()
                    local started, problem = nirender.stream(spec.streamName, spec.megabytes, spec.warmupFrames, true)
                    -- The runner starts the host at this line.
                    say("stream started %s %s %s %s", spec.streamName, spec.backend, tostring(started), tostring(problem or ""))
                    if not started then return scenario.finish() end
                    after(3, function() step(1) end)
                end)
            end)
        end)
    end)
end
