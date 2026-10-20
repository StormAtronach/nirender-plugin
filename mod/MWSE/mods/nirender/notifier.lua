--[[
    TEMPORARY, plan C17: a stand-in for a change notifier in MWSE's Lua bindings. Remove this
    file (and `nirender.changed`, `instance_stream::note_change`) when MWSE notifies the
    changes itself.

    The plugin keeps each retained shape's material and geometry and notes them again only
    when it learns of a change: the engine's own changes it sees (controllers make a shape
    live; a new property or effect state passes through two ports), but a Lua mod that writes
    into a property, a texture map or a shape, or that attaches a controller, changes memory
    the engine reads at its next draw with no call the plugin sees. Without this file such a
    change reaches the helper at the check of every 64th frame.

    This file wraps the write hooks (`__newindex`) and some methods (`__index`) of the sol
    metatables of MWSE's NI types and reports the changed object's address to the plugin
    (`nirender.changed`); once a frame the plugin checks again the retained shapes that are
    such an object or hold it in their property state. The metatables are found on the
    objects of the scene at each game load and on a few new objects; a type pushed in a form
    not seen here (sol has one metatable for each form of a type) is not wrapped, and its
    changes keep the 64th-frame delay.
]]

local nirender = package.loaded["nirender"]
local convert = mwse.memory and mwse.memory.convertFrom and mwse.memory.convertFrom.niObject
if not nirender or not nirender.changed or not convert then
    mwse.log("[nirender] notifier: not available (nirender.changed %s, mwse.memory.convertFrom.niObject %s)",
        tostring(nirender and nirender.changed ~= nil), tostring(convert ~= nil))
    return
end

local wrapped = {}
local wrappedCount = 0
-- The texturing properties whose maps Lua reached in this frame: a map has no address the
-- plugin can use, so a write into a map reports the texturing properties it came from.
local touched = {}

local function notify(object)
    if object == nil then return end
    local ok, address = pcall(convert, object)
    if ok and address and address ~= 0 then
        nirender.changed(address)
    end
end

-- The writes of a shape or a node that the plugin does not read at every display. `flags`
-- holds the app-cull bit: MWSE writes the field with no call. A change of a shape makes the
-- plugin check its retained instance again at its next display.
local AV_KEYS = {
    data = true, skinInstance = true, flags = true,
    alphaProperty = true, fogProperty = true, materialProperty = true, stencilProperty = true,
    texturingProperty = true, vertexColorProperty = true, zBufferProperty = true,
}
-- The methods that attach or detach a controller: the object (or the target) changed.
local CONTROLLER_METHODS = { prependController = true, removeController = true, removeAllControllers = true }
local MAP_KEYS = {
    maps = true, baseMap = true, darkMap = true, detailMap = true, glossMap = true, glowMap = true, bumpMap = true,
}

local function wrapIndex(mt, role)
    local index = rawget(mt, "__index")
    if type(index) ~= "function" then
        return
    end
    rawset(mt, "__index", function(object, key)
        local value = index(object, key)
        if role == "texturing" and MAP_KEYS[key] then
            touched[object] = true
        end
        if type(value) == "function" then
            if CONTROLLER_METHODS[key] then
                return function(self, ...)
                    local result = value(self, ...)
                    notify(self)
                    return result
                end
            elseif key == "setTarget" then
                return function(self, target, ...)
                    local result = value(self, target, ...)
                    notify(target)
                    return result
                end
            end
        end
        return value
    end)
end

local function wrap(object, role)
    local mt = object and debug.getmetatable(object)
    if not mt or wrapped[mt] then
        return
    end
    local newindex = rawget(mt, "__newindex")
    if type(newindex) ~= "function" then
        return
    end
    wrapped[mt] = role
    wrappedCount = wrappedCount + 1
    if role == "property" or role == "texturing" then
        rawset(mt, "__newindex", function(o, k, v)
            newindex(o, k, v)
            notify(o)
        end)
    elseif role == "map" then
        rawset(mt, "__newindex", function(o, k, v)
            newindex(o, k, v)
            for property in pairs(touched) do
                notify(property)
            end
        end)
    elseif role == "av" then
        rawset(mt, "__newindex", function(o, k, v)
            newindex(o, k, v)
            if AV_KEYS[k] then
                notify(o)
            end
        end)
    end
    if role ~= "map" then
        wrapIndex(mt, role)
    end
end

local function roleOf(object)
    local ok, result = pcall(function()
        if object:isInstanceOfType(ni.type.NiTexturingProperty) then return "texturing" end
        if object:isInstanceOfType(ni.type.NiProperty) then return "property" end
        if object:isInstanceOfType(ni.type.NiAVObject) then return "av" end
        if object:isInstanceOfType(ni.type.NiTimeController) then return "controller" end
        return nil
    end)
    return ok and result or nil
end

local PROPERTY_FIELDS = { "alphaProperty", "fogProperty", "materialProperty", "stencilProperty", "texturingProperty", "vertexColorProperty", "zBufferProperty" }

local function visit(object)
    local role = roleOf(object)
    if role then
        wrap(object, role)
    end
    if role == "texturing" then
        local ok, maps = pcall(function() return object.maps end)
        if ok and maps then
            for i = 1, #maps do
                if maps[i] then
                    wrap(maps[i], "map")
                end
            end
        end
    end
end

-- The scene at a game load: every node and shape, its properties and its maps, and its
-- controllers, until each kind's metatable is wrapped (the walk stops early when nothing
-- new turns up for a while).
local function scan()
    local before = wrappedCount
    local stack = { tes3.game.worldRoot }
    local seen, sinceNew = 0, 0
    while #stack > 0 and seen < 200000 and sinceNew < 20000 do
        local node = table.remove(stack)
        seen = seen + 1
        local count = wrappedCount
        visit(node)
        for _, field in ipairs(PROPERTY_FIELDS) do
            local ok, property = pcall(function() return node[field] end)
            if ok and property then
                visit(property)
            end
        end
        local okc, controller = pcall(function() return node.controller end)
        if okc and controller then
            visit(controller)
        end
        local okk, children = pcall(function() return node.children end)
        if okk and children then
            for i = 1, #children do
                if children[i] then
                    stack[#stack + 1] = children[i]
                end
            end
        end
        sinceNew = (wrappedCount > count) and 0 or (sinceNew + 1)
    end
    mwse.log("[nirender] notifier: %d metatables wrapped (%d new) from %d objects of the scene", wrappedCount, wrappedCount - before, seen)
end

-- New objects of the types Lua can make, so that their forms are wrapped too.
local function seed()
    for _, make in ipairs({
        function() return niAlphaProperty.new() end,
        function() return niMaterialProperty.new() end,
        function() return niZBufferProperty.new() end,
        function() return niVertexColorProperty.new() end,
        function() return niStencilProperty.new() end,
        function() return niNode.new() end,
    }) do
        local ok, object = pcall(make)
        if ok and object then
            visit(object)
        end
    end
    local ok, map = pcall(function() return niTexturingPropertyMap.new() end)
    if ok and map then
        wrap(map, "map")
    end
end

event.register(tes3.event.enterFrame, function()
    if next(touched) then
        touched = {}
    end
end)
event.register(tes3.event.loaded, scan)
seed()
mwse.log("[nirender] notifier: on (temporary, plan C17); %d metatables wrapped from new objects", wrappedCount)
