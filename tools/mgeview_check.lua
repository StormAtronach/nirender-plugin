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

-- 2b. Chunks that a batch loads itself (a batch that runs a base batch, as runs-batch-d410/batch-runs-a.lua
-- does with loadfile) read the view too, also one level further down, and the global loaders stay as they were.
local folder = os.getenv("TEMP") or os.getenv("TMP") or "."
local inner = folder .. "/mgeview_check_inner.lua"
local file = assert(io.open(inner, "w"))
file:write("return mge, loadstring('return mge')()\n")
file:close()
local loaders = { loadstring = loadstring, load = load, loadfile = loadfile, dofile = dofile }
local outer = assert(loadstring(string.format([[
    local path = %q
    local fromString = loadstring("return mge")()
    local fromLoad = load("return mge")()
    local fromFile, deeper = loadfile(path)()
    local fromDo = dofile(path)
    local broken, problem = loadstring("return (")
    return fromString, fromLoad, fromFile, deeper, fromDo, broken, problem
]], inner)))
setfenv(outer, view.environment(spec))
local fromString, fromLoad, fromFile, deeper, fromDo, broken, problem = outer()
os.remove(inner)
assert(fromString == harness, "a chunk of loadstring reads the global mge, not the view")
assert(fromLoad == harness, "a chunk of load reads the global mge, not the view")
assert(fromFile == harness and deeper == harness, "a chunk of loadfile reads the global mge, not the view")
assert(fromDo == harness, "a chunk of dofile reads the global mge, not the view")
assert(broken == nil and type(problem) == "string", "a chunk with an error gives nil and the message")
for name, loader in pairs(loaders) do
    assert(rawget(_G, name) == loader, "the global " .. name .. " changed")
end
assert(mge == plain and plain.shaders == nil, "an inner chunk changed the global mge")

-- 2c. The run's --lua-file stands for a mod's Lua (W494): its chunk reads the global mge, also in an
-- engine-reference run, where the batch beside it reads the view. A copy of mgeview.lua whose
-- luaFileChunk gives the chunk the view's environment fails here.
local fileChunk = assert(view.luaFileChunk("fileGlobal = 9; return mge, mge.shaders"))
local fileSees, fileShaders = fileChunk()
assert(fileSees == plain and fileShaders == nil, "the --lua-file reads the view, not the global mge")
assert(rawget(_G, "fileGlobal") == 9, "a global write of the --lua-file goes to _G")
assert(select(2, view.luaFileChunk("return (")) ~= nil, "a --lua-file with an error gives nil and the message")
setfenv(batch, view.environment(spec))
assert(select(2, batch()) == harness, "the batch beside it reads the view")

-- 3. A game with MGE XE's API: the global has its own shaders, and the view does not replace them
-- in the global table.
local real = { shaders = { find = function() return "real" end, load = function() return "real" end }, render = {} }
view = fresh(real)
harness = view.of(spec)
assert(harness.shaders.find({}) == nil, "the harness keeps its stand-ins")
assert(real.shaders.find({}) == "real" and real.shaders.load({}) == "real", "the global shaders changed")
assert(harness.render == real.render, "other fields read the global")
print("mgeview check: ok")
