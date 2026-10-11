---@diagnostic disable
-- The in-game part of tools/run_ab.py: for each view of a list, a reference picture of the
-- game and a capture of the render link stream of the same frame.
--
-- The reference is the game drawn by the fixed-function pipeline of Direct3D: MGE XE with its
-- distant land, its water, its shadows, its shaders and its per-pixel light off, and the
-- game code for the renderer. The host program draws the capture, and the runner compares
-- the two pictures.
--
-- Each view holds the game in menu mode. Menu mode stops the animations and the clock, so
-- the two pictures are of the same moment.
--
-- spec.places is a list of { cell = name or "", views = "here" | "actor" | "inventory" | "tour" }.
-- "tour" is each object of the cell whose id starts with spec.tour.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "ab")
    local say, after = scenario.say, scenario.after
    local number = 0

    -- The features of MGE XE that add to the picture of the game, and the values they had.
    local features = { "distantLand", "distantStatics", "distantWater", "grass", "shaders", "shadows", "atmosphericScattering",
        "exponentialFog", "fpsCounter", "displayMessages", "reflectiveWater", "dynamicRipples" }
    local before = {}
    local lighting_before = nil

    local function plain_mge(on)
        if on then
            for _, name in ipairs(features) do
                before[name] = mge.render[name]
                mge.render[name] = false
            end
            lighting_before = mge.getLightingMode()
            mge.setLightingMode(mge.lightingMode.vertex)
        else
            for _, name in ipairs(features) do
                if before[name] ~= nil then mge.render[name] = before[name] end
            end
            if lighting_before then mge.setLightingMode(lighting_before) end
        end
    end

    local function hold(on, inventory)
        if on and inventory then
            -- The inventory with the character image and the map. The game shows these four
            -- menus together, and they must be visible before the start of menu mode.
            for _, name in ipairs({ "MenuInventory", "MenuMap", "MenuStat", "MenuMagic" }) do
                local shown = tes3ui.findMenu(name)
                if shown then
                    shown.visible = true
                    shown.disabled = false
                    shown:updateLayout()
                end
            end
            tes3ui.enterMenuMode("MenuInventory")
        elseif on then
            local menu = tes3ui.createMenu({ id = "nirender:hold", fixedFrame = true })
            menu:createLabel({ text = "nirender A/B" })
            menu:updateLayout()
            tes3ui.enterMenuMode("nirender:hold")
        else
            tes3ui.leaveMenuMode()
            local menu = tes3ui.findMenu("nirender:hold")
            if menu then menu:destroy() end
        end
    end

    -- One view: the reference picture, then the capture.
    local function one_view(label, inventory, done)
        number = number + 1
        local n = number
        hold(true, inventory)
        nirender.linkUseKinds(0)
        nirender.linkUseFacts(0)
        nirender.linkTextures(false)
        nirender.setMode("*", "off")
        if inventory then
            -- The game draws the character image only when it changes. The reference gets a
            -- new one, as the capture does: the two are then of the same state of the game.
            after(0.4, function() tes3ui.updateInventoryCharacterImage() end)
        end
        -- The frame with the game code and the plain MGE XE must be on the screen.
        after(1.2, function()
            mge.saveScreenshot({ path = string.format("%s/ref-%03d.png", spec.dir, n), captureWithUI = true })
            after(0.8, function()
                nirender.setMode("*", "live")
                nirender.linkTextures(true)
                nirender.linkUseFacts(spec.facts)
                nirender.linkUseKinds(spec.kinds)
                if inventory then after(0.2, function() tes3ui.updateInventoryCharacterImage() end) end
                -- A second picture, of MGE XE as it draws from the stream. A difference between
                -- it and the reference is a difference of the Rust code or of the link, not of
                -- the host program.
                after(0.8, function() mge.saveScreenshot({ path = string.format("%s/link-%03d.png", spec.dir, n), captureWithUI = true }) end)
                after(1.6, function()
                    local ok, problem = nirender.capture(string.format("%s/view-%03d.nircap", spec.dir, n), 3, inventory and 4 or 2)
                    if not ok then
                        say("view|%d|%s|capture failed: %s", n, label, tostring(problem))
                        hold(false)
                        return done()
                    end
                    local asked = false
                    local function poll()
                        local status = nirender.captureStatus()
                        if inventory and not asked and status:sub(1, 6) == "record" then
                            -- The capture made the rendered textures again, and they are
                            -- empty. The game draws the character image only when it changes.
                            asked = true
                            tes3ui.updateInventoryCharacterImage()
                        end
                        if status:sub(1, 4) ~= "done" then return after(0.01, poll) end
                        say("view|%d|%s|%s", n, label, status)
                        hold(false)
                        after(0.3, done)
                    end
                    poll()
                end)
            end)
        end)
    end

    local function views_of(place)
        local list = {}
        if place.views == "actor" then
            local actor = scenario.nearestActor()
            if actor then
                list[1] = { label = "actor " .. actor.object.id, go = function() scenario.faceActor(actor, spec.actorDistance or 160, true) end }
            end
        elseif place.views == "inventory" then
            list[1] = { label = "inventory", inventory = true, go = function() end }
        elseif place.views == "tour" then
            local prefix = spec.tour:lower()
            for ref in tes3.player.cell:iterateReferences() do
                local id = ref.object.id:lower()
                if id:sub(1, #prefix) == prefix and not id:find("tile", 1, true) and not ref.disabled then
                    local at = ref.position
                    list[#list + 1] = {
                        label = ref.object.id,
                        go = function()
                            tes3.player.position = tes3vector3.new(at.x, at.y - (spec.tourDistance or 300), at.z)
                            tes3.player.orientation = tes3vector3.new(0, 0, 0)
                        end,
                    }
                end
            end
        else
            list[1] = { label = "here", go = function() end }
        end
        return list
    end

    local function one_place(index)
        local place = spec.places[index]
        if not place then
            plain_mge(false)
            nirender.linkUseKinds(0)
            nirender.linkUseFacts(0)
            nirender.linkTextures(false)
            nirender.setMode("*", "off")
            return after(1, scenario.finish)
        end
        if place.cell ~= "" then scenario.goTo(place.cell) end
        after(place.cell ~= "" and (spec.settleSeconds or 6) or 0.1, function()
            local where = tes3.player.cell.editorName
            local views = views_of(place)
            if spec.maxViews and #views > spec.maxViews then
                for i = #views, spec.maxViews + 1, -1 do views[i] = nil end
            end
            say("place '%s': %d views", where, #views)
            local function next_view(i)
                local view = views[i]
                if not view then return one_place(index + 1) end
                view.go()
                -- The game must draw the new view, and a level-of-detail node must switch.
                after(1.0, function()
                    one_view(where .. " / " .. view.label, view.inventory, function() next_view(i + 1) end)
                end)
            end
            next_view(1)
        end)
    end

    scenario.start(function()
        after(spec.settleSeconds or 6, function()
            local ok, message = nirender.link()
            say("link: %s (%s)", tostring(ok), message)
            if not ok then return scenario.finish() end
            plain_mge(true)
            one_place(1)
        end)
    end)
end
