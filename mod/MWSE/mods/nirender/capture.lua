---@diagnostic disable
-- The in-game part of tools/run_capture.py. It loads a save, goes to a cell, and makes the
-- plugin write a capture of the render link stream: each resource with its bytes and each
-- command of some frames. A program without the game can draw the capture again.
--
-- The capture needs the Rust code, the textures of the link, and all facts and kinds of the
-- link in use.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "capture")
    local say, after = scenario.say, scenario.after

    local function capture()
        if spec.actor then
            local actor = scenario.nearestActor()
            if actor then
                say("actor: %s", actor.object.id)
                scenario.faceActor(actor, spec.actorDistance or 160, true)
                -- Menu mode stops the animations: the picture of the game and the frames of
                -- the capture have the same pose.
                local menu = tes3ui.createMenu({ id = "nirender:hold", fixedFrame = true })
                menu:createLabel({ text = "nirender capture" })
                menu:updateLayout()
                tes3ui.enterMenuMode("nirender:hold")
            end
        end
        if spec.yaw then
            -- The direction of the view, in degrees from north.
            tes3.player.orientation = tes3vector3.new(0, 0, math.rad(spec.yaw))
        end
        nirender.setMode("*", "live")
        say("textures through the link: %s", tostring(nirender.linkTextures(true)))
        say("facts in use: %s", tostring(nirender.linkUseFacts(spec.facts)))
        say("kinds in use: %s", tostring(nirender.linkUseKinds(spec.kinds)))
        after(2, function()
            -- The picture of the game for the same view, with the additions of MGE XE.
            mge.saveScreenshot({ path = spec.shot, captureWithUI = true })
            after(1, function()
                local ok, problem = nirender.capture(spec.captureFile, spec.warmupFrames, spec.frames)
                say("capture started: %s %s", tostring(ok), tostring(problem or ""))
                if not ok then return scenario.finish() end
                local function poll()
                    local status = nirender.captureStatus()
                    if status:sub(1, 4) ~= "done" then return after(0.2, poll) end
                    say("capture: %s", status)
                    if tes3ui.findMenu("nirender:hold") then
                        tes3ui.leaveMenuMode()
                        tes3ui.findMenu("nirender:hold"):destroy()
                    end
                    nirender.linkUseKinds(0)
                    nirender.linkUseFacts(0)
                    nirender.linkTextures(false)
                    nirender.setMode("*", "off")
                    after(2, scenario.finish)
                end
                poll()
            end)
        end)
    end

    scenario.start(function()
        after(spec.settleSeconds or 6, function()
            local ok, message = nirender.link()
            say("link: %s (%s)", tostring(ok), message)
            if not ok then return scenario.finish() end
            if spec.cell and spec.cell ~= "" then
                scenario.goTo(spec.cell)
                return after(spec.settleSeconds or 6, capture)
            end
            capture()
        end)
    end)
end
