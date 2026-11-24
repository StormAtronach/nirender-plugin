-- The hover of the runner's walk (owndevice.lua, the step "walk"): the player goes north at a height
-- over what is under it, so that no rock, tree or wall stops the walk. The owner, 2026-11-24 (W3):
-- "for automatic walks, it is probably good if the player is made to hover 512 units above the
-- ground". A walk on the ground stopped in a rock at the centre of Balmora (-2, -2), and in the trees
-- north of it. The camera is 512 units higher than on foot, so a walk with the hover is a new series
-- of numbers: do not compare it with a walk on the ground.

local hover = {}

-- The height of the player over the top of what is under it.
hover.HEIGHT = 512

-- The height from which the ray goes down: over every land and object of the game.
local RAY_START = 30000

-- The top of what is under the place x, y (the land, or a rock, a tree or a roof on it), from a ray
-- down that does not see the player; nil when the ray finds nothing (a cell that is not loaded).
function hover.ground(x, y)
    local hit = tes3.rayTest({
        position = tes3vector3.new(x, y, RAY_START),
        direction = tes3vector3.new(0, 0, -1),
        ignore = { tes3.player },
    })
    return hit and hit.intersection.z
end

-- The place of the next frame of the walk: `x, y, z` moved north by `step`, at HEIGHT over the top of
-- what `ground(x, y)` finds there; with nothing found, the player keeps its height.
function hover.next(x, y, z, step, ground)
    local north = y + step
    local top = ground(x, north)
    if top then
        return x, north, top + hover.HEIGHT
    end
    return x, north, z
end

return hover
