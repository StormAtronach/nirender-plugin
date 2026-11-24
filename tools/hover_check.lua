-- A check of hover.lua outside the game (W3): each frame of the runner's walk puts the player north by its
-- step, 512 units over the top that the ray down finds there or over the water where the top is under it, and
-- keeps the player's height where the ray finds nothing. Run with the LuaJIT of the MWSE repository: no game, no
-- build. run_own_device.py --self-test runs it.
--     luajit tools/hover_check.lua mod/MWSE/mods/nirender/hover.lua
local hover = assert(loadfile(arg[1]))()

assert(hover.HEIGHT == 512, "the owner's height is 512 units")

-- The tops of a line north: the land at 1000, a rock at y 300 to 400 with its top at 2638, the sea floor at -1167
-- from y 600 to 700, then a cell that is not loaded from y 1000. The water level is 0.
local WATER = 0
local asked = {}
local function ground(x, y)
    asked[#asked + 1] = { x, y }
    if y >= 1000 then
        return nil
    end
    if y >= 300 and y < 400 then
        return 2638
    end
    if y >= 600 and y < 700 then
        return -1167
    end
    return 1000
end

-- 1. Over the land: the player goes north by the step, 512 over the land, and the ray goes down at the new place.
local x, y, z = hover.next(-12288, 0, 2258, 100, ground, WATER)
assert(x == -12288 and y == 100 and z == 1512, string.format("over the land: %s %s %s", x, y, z))
assert(asked[1][1] == -12288 and asked[1][2] == 100, "the ray goes down at the new place, not the old one")

-- 2. Over the rock: 512 over its top, not over the land under it. The null: the walk on the ground, which kept z,
-- stayed in this rock at the centre of Balmora (-2, -2).
x, y, z = hover.next(x, 250, z, 100, ground, WATER)
assert(y == 350 and z == 2638 + 512, string.format("over the rock: %s %s", y, z))

-- 3. Past the rock, down to the land again.
x, y, z = hover.next(x, y, z, 100, ground, WATER)
assert(y == 450 and z == 1512, string.format("past the rock: %s %s", y, z))

-- 4. Over the sea: the ray finds the floor under the water, and the player stays 512 over the water, not 512 over
-- the floor (the Reviewer's note: a view under the water in a walk's numbers).
x, y, z = hover.next(x, 550, z, 100, ground, WATER)
assert(y == 650 and z == WATER + 512, string.format("over the sea: %s %s", y, z))

-- 5. A place where the ray finds nothing: the player keeps its height and still goes north.
x, y, z = hover.next(x, 950, z, 100, ground, WATER)
assert(y == 1050 and z == 512, string.format("no hit: %s %s", y, z))

print("hover check: ok")
