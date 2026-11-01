-- A check of notifier.lua outside the game: its reports, and no new function for a second read of
-- a wrapped method. Run by hand with the LuaJIT of the MWSE repository: no game, no build.
--     luajit tools/notifier_check.lua mod/MWSE/mods/nirender/notifier.lua
local reports = {}
package.loaded["nirender"] = { changed = function(address) reports[#reports + 1] = address end }
mwse = { memory = { convertFrom = { niObject = function(object) return object.address end } }, log = function() end }
local handlers = {}
tes3 = { event = { enterFrame = "enterFrame", loaded = "loaded" }, game = {} }
event = { register = function(name, f) handlers[name] = f end }
ni = { type = {} }
local path = arg[1]
local chunk = assert(loadfile(path))

-- A stand-in of a sol type: object tables with a metatable that has the two hooks. As sol does
-- (`index_call_with_`, deps/sol of MWSE), a read of a method gives a new function each time, which
-- takes its object from the call.
local methods = {
    prependController = function(self, c) self.last = c; return "prepended" end,
    removeController = function(self) return "removed" end,
    setTarget = function(self, target) self.target = target; return "set" end,
    isInstanceOfType = function(self, t) return self.kind == t end,
}
local mt = {}
mt.__index = function(o, k)
    local method = methods[k]
    if method then
        return function(...) return method(...) end
    end
    return rawget(o, "_" .. k)
end
mt.__newindex = function(o, k, v) rawset(o, "_" .. k, v) end
local function object(kind, address)
    return setmetatable({ address = address, kind = kind }, mt)
end
ni.type = { NiTexturingProperty = "tex", NiProperty = "prop", NiAVObject = "av", NiTimeController = "ctl" }
_G.niAlphaProperty = nil
_G.niNode = { new = function() return object("av", 100) end }
_G.niMaterialProperty, _G.niZBufferProperty, _G.niVertexColorProperty, _G.niStencilProperty = nil, nil, nil, nil
_G.niTexturingPropertyMap = nil

chunk()

local node = object("av", 7)
-- A plain read and a write of a key that is not reported.
assert(node.flags == nil)
node.other = 1
assert(#reports == 0, "a write of another key reports nothing")
-- A reported key.
node.flags = 3
assert(#reports == 1 and reports[1] == 7, "flags reports the node")
-- A controller method: its answer, and the report of its object. The name keeps its first function.
local first, second = node.prependController, node.prependController
assert(first == second, "the second read makes no new function")
assert(first(node, "c") == "prepended" and node.last == "c")
assert(#reports == 2 and reports[2] == 7, "the method reports its object")
local other = object("av", 8)
assert(other.prependController == first)
assert(other:prependController("d") == "prepended" and other.last == "d")
assert(reports[3] == 8, "the shared wrapper reports the object it is called on")
-- setTarget reports the target.
local target = object("av", 9)
assert(node:setTarget(target) == "set" and node.target == target)
assert(reports[4] == 9, "setTarget reports the target")
-- A method that is not wrapped comes as sol gives it: a new function at each read, and it works.
assert(node.isInstanceOfType ~= node.isInstanceOfType, "the stand-in gives a new function at each read")
assert(node:isInstanceOfType("av"))
-- No new garbage for many reads of a wrapped method.
-- The JIT is off here: its traces count as memory too.
jit.off()
collectgarbage(); collectgarbage("stop")
local before = collectgarbage("count")
for _ = 1, 10000 do local _ = node.prependController end
local grown = (collectgarbage("count") - before) * 1024
collectgarbage("restart")
assert(grown < 1024, "10000 reads of a wrapped method made " .. grown .. " bytes")
handlers.enterFrame()
print("notifier check: ok; 10000 reads of a wrapped method made " .. grown .. " bytes")
