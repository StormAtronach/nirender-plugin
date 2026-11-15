-- A check of mgeview.lua outside the game (W486): the harness's view of mge in an engine-reference
-- run has its stand-ins, and the global mge of the game's mods stays as MWSE made it. Run with the
-- LuaJIT of the MWSE repository: no game, no build. run_own_device.py --self-test runs it too.
--     luajit tools/mgeview_check.lua mod/MWSE/mods/nirender/mgeview.lua
local path = arg[1]
local loaded = assert(loadfile(path))

local function fresh(global)
    mge = global
    return loaded()
end

-- 1. A game without MGE XE's API: the global has no shaders.
local plain = { enabled = function() return false end, lightingMode = { vertex = 0 } }
local view = fresh(plain)
assert(view.of(nil) == plain and view.of({}) == plain, "a run that is no engine reference reads the global")
assert(view.environment({}) == nil, "a run that is no engine reference keeps the global environment")
local spec = { engineReference = true }
local harness = view.of(spec)
assert(harness ~= plain, "an engine-reference run has its own view")
assert(view.of(spec) == harness, "one view for the run")
assert(harness.shaders.find({ name = "x" }) == nil and harness.shaders.load({ name = "x" }) == nil, "the stand-in shaders find and load nothing")
assert(type(harness.saveScreenshot) == "function" and harness.getLightingMode() == "no MGE XE")
assert(harness.enabled == plain.enabled, "other fields read the global")
-- The global stays as MWSE made it: what a mod reads there. A view that also wrote its stand-ins into
-- the global, as owndevice.lua did before W486, fails here.
assert(plain.shaders == nil and plain.saveScreenshot == nil and plain.getLightingMode == nil, "the global changed")
assert(rawget(plain, "setLightingMode") == nil, "the global changed")

-- 2. A batch chunk in its environment: mge is the view, a write of a global goes to _G, and a
-- function that the chunk makes (a set's run) reads the view too.
local batch = assert(loadstring([[
    batchGlobal = 7
    local sets = { { name = "a", run = function() return mge.shaders.find({ name = "ashfall" }) == nil and mge.getLightingMode() end } }
    return sets, mge
]]))
setfenv(batch, view.environment(spec))
local sets, seen = batch()
assert(seen == harness, "the batch reads the view")
assert(sets[1].run() == "no MGE XE", "a set's run reads the view")
assert(rawget(_G, "batchGlobal") == 7, "a global write of the batch goes to _G")
assert(mge == plain and plain.shaders == nil, "the batch changed the global mge")

-- 3. A game with MGE XE's API: the global has its own shaders, and the view does not replace them
-- in the global table.
local real = { shaders = { find = function() return "real" end, load = function() return "real" end }, render = {} }
view = fresh(real)
harness = view.of(spec)
assert(harness.shaders.find({}) == nil, "the harness keeps its stand-ins")
assert(real.shaders.find({}) == "real" and real.shaders.load({}) == "real", "the global shaders changed")
assert(harness.render == real.render, "other fields read the global")
print("mgeview check: ok")
