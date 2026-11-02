-- The moving camera of the live check of `NIRENDER_EFFECT_TURN_CHECK` (src/effect_chain.rs).
--
-- In a live session in a cell of the test level, through mwse-mcp:
--     local f = io.open("<repo>/tools/effect_turn_walk.lua"); local s = f:read("*a"); f:close(); return loadstring(s)()(60)
--
-- Places the `turn_` and `ctrl_` exhibits of tools/make_test_nifs.py in a grid north of the player,
-- then flies the player for that many seconds on an ellipse around the grid, turning and pitching
-- towards its centre with a wobble: each frame has another place and other axes. Time runs (the
-- camera follows a player moved by Lua only then). By velocity, from the real position, with
-- collision off for the flight. Returns at once; the end is a line in MWSE.log.
local SPACING = 300
local COLUMNS = 5
local STUCK = 300

return function(seconds)
    local player = tes3.player
    local mobile = tes3.mobilePlayer
    local start = player.position:copy()
    local centre = tes3vector3.new(start.x, start.y + 2000, start.z + 150)

    local placed = 0
    for _, entry in ipairs(require("nirender.test_nifs")) do
        if entry.id:find("turn_", 1, true) or entry.id:find("ctrl_", 1, true) then
            local object = tes3.getObject(entry.id) or tes3.createObject({ objectType = tes3.objectType.static, id = entry.id, mesh = entry.mesh })
            local column, row = placed % COLUMNS, math.floor(placed / COLUMNS)
            local at = tes3vector3.new(centre.x + (column - (COLUMNS - 1) / 2) * SPACING, centre.y + (row - 4.5) * SPACING, centre.z)
            tes3.createReference({ object = object, cell = player.cell, position = at, orientation = tes3vector3.new(0, 0, 0) })
            placed = placed + 1
        end
    end

    tes3.worldController.menuController.godModeEnabled = true
    tes3.worldController.simulationTimeScalar = 1
    tes3.force1stPerson()
    mobile.vanityDisabled = true
    mobile.movementCollision = false
    mobile.isFlying = true

    local began = os.clock()
    local step
    step = function()
        local t = os.clock() - began
        local at = player.position
        if t >= seconds or (t > 2 and (centre - at):length() > 6000) then
            event.unregister(tes3.event.enterFrame, step)
            mobile.velocity = tes3vector3.new(0, 0, 0)
            mobile.isFlying = false
            mobile.movementCollision = true
            mwse.log("[effect turn walk] done after %.1f s at %.0f %.0f %.0f", t, at.x, at.y, at.z)
            return
        end
        local angle = t * 0.7
        local want = tes3vector3.new(centre.x + 1500 * math.cos(angle), centre.y + 2300 * math.sin(angle), centre.z + 500 + 300 * math.sin(angle * 2.3))
        local delta = math.max(tes3.worldController.deltaTime, 0.001)
        if (want - at):length() > STUCK and t > 3 then
            mwse.log("[effect turn walk] %.0f units from the path at %.1f s", (want - at):length(), t)
        end
        mobile.velocity = (want - at) * (1 / delta)
        local yaw = math.atan2(centre.x - at.x, centre.y - at.y) + 0.25 * math.sin(t * 1.7)
        local across = math.sqrt((centre.x - at.x) ^ 2 + (centre.y - at.y) ^ 2)
        local pitch = -math.atan2(centre.z - at.z, across) + 0.2 * math.sin(t * 1.1)
        -- `orientation.x` turns about the world's x axis (mwse-mcp.md).
        player.orientation = tes3vector3.new(pitch * math.cos(yaw), 0, yaw)
    end
    event.register(tes3.event.enterFrame, step)
    return string.format("%d exhibits around %.0f %.0f %.0f, flying for %d s", placed, centre.x, centre.y, centre.z, seconds)
end
