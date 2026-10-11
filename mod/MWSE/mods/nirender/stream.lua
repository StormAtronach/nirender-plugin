---@diagnostic disable
-- The in-game part of tools/run_stream.py. It loads a save, puts the player in front of an
-- actor, and makes the plugin send the render link stream into shared memory. The runner
-- starts the host program, which draws the stream in its own window while the game runs.
--
-- One stream for each backend of spec.backends, one after the other.

return function(nirender, spec)
    local scenario = require("nirender.scenario")(nirender, spec, "stream")
    local say, after = scenario.say, scenario.after
    local actor = nil

    local function one_stream(index)
        local backend = spec.backends[index]
        if not backend then
            scenario.eachFrame = nil
            if spec.ownHost then
                -- The plugin is the host of the link, and there is no way back in this
                -- version. The runner ends the game.
                return scenario.finish()
            end
            nirender.linkUseKinds(0)
            nirender.linkUseFacts(0)
            nirender.linkTextures(false)
            nirender.setMode("*", "off")
            return after(2, scenario.finish)
        end
        local name = string.format("%s-%d", spec.streamName, index)
        local ok, problem = nirender.stream(name, spec.megabytes, spec.warmupFrames, spec.ownHost == true)
        -- The runner starts the host at this line.
        say("stream started %s %s %s %s", name, backend, tostring(ok), tostring(problem or ""))
        if not ok then return scenario.finish() end
        local finish_at = os.clock() + spec.seconds
        local function poll()
            local status = nirender.captureStatus()
            if status:sub(1, 4) ~= "done" and os.clock() < finish_at then return after(0.25, poll) end
            nirender.captureStop()
            say("stream ended %s %s: %s", name, backend, nirender.captureStatus())
            after(3, function() one_stream(index + 1) end)
        end
        poll()
    end

    scenario.start(function()
        after(spec.settleSeconds or 6, function()
            local ok, message = nirender.link()
            say("link: %s (%s)", tostring(ok), message)
            if not ok then return scenario.finish() end
            if spec.cell and spec.cell ~= "" then scenario.goTo(spec.cell) end
            after(spec.cell and spec.cell ~= "" and 6 or 0.1, function()
                actor = spec.actor and scenario.nearestActor() or nil
                if actor then
                    say("actor: %s", actor.object.id)
                    scenario.faceActor(actor, spec.actorDistance or 160, true)
                    -- An actor plays its idle animations, and it can walk away. The player
                    -- looks at it again two times in a second, and goes back in front of it
                    -- when it is far.
                    local next_look = 0
                    scenario.eachFrame = function(now)
                        if now < next_look then return end
                        next_look = now + 0.5
                        scenario.faceActor(actor, spec.actorDistance or 160, false)
                    end
                end
                nirender.setMode("*", "live")
                say("textures through the link: %s", tostring(nirender.linkTextures(true)))
                say("facts in use: %s", tostring(nirender.linkUseFacts(spec.facts)))
                say("kinds in use: %s", tostring(nirender.linkUseKinds(spec.kinds)))
                after(2, function()
                    mge.saveScreenshot({ path = spec.shot, captureWithUI = true })
                    after(1, function() one_stream(1) end)
                end)
            end)
        end)
    end)
end
