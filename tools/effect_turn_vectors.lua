-- The test vectors of `host/src/effect_turn.rs`: the engine's own functions on made objects.
--
-- In a running game, through mwse-mcp:
--     return dofile("<repo>/tools/effect_turn_vectors.lua")("<repo>/host/src/effect_turn_vectors.txt")
--
-- It calls NiBillboardNode::RotateToCamera 0x6D3C00, NiAVObject::UpdateWorldData 0x6FB350 and
-- NiTextureEffect::UpdateWorldData 0x6E3970 (with UpdateProjection 0x6E37C0) through the FFI, on
-- zeroed memory that holds only what they read. These functions call no virtual function here:
-- a billboard has no children, and a zero velocity allocates nothing (SetWorldVelocity 0x6EC5F0).
-- No object of the game is touched. A line of the output is
--     name|coordinate mode|kinds|root|the locals|model|camera|world|projection
-- with each float as the eight hex digits of its word.
local ffi = require("ffi")

local rotateToCamera = ffi.cast("void (__thiscall *)(void*, void*)", 0x6D3C00)
local updateWorldData = ffi.cast("void (__thiscall *)(void*)", 0x6FB350)
local effectUpdateWorldData = ffi.cast("void (__thiscall *)(void*)", 0x6E3970)

-- The layouts of src/gen/layouts.rs.
local FLAGS, PARENT, LOCAL_ROTATE, LOCAL_TRANSLATE, LOCAL_SCALE, WORLD = 0x14, 0x18, 0x2C, 0x30, 0x3C, 0x40
local NODE_SIZE, EFFECT_SIZE, CAMERA_SIZE = 0xB8, 0x148, 0x110
local CAMERA_LOCATION, CAMERA_DIRECTION = WORLD + 0x24, 0xDC
local MODEL_MATRIX, MODEL_TRANSLATION, WORLD_MATRIX, COORDINATES = 0xA8, 0xCC, 0xD8, 0x118

local kept = {}
local function memory(size)
    local at = ffi.new("uint8_t[?]", size)
    kept[#kept + 1] = at
    return at
end

local function putFloats(at, offset, values)
    local floats = ffi.cast("float*", at + offset)
    for i, value in ipairs(values) do floats[i - 1] = value end
end

local function hexAt(at, offset, count)
    local words = ffi.cast("uint32_t*", at + offset)
    local out = {}
    for i = 0, count - 1 do out[#out + 1] = bit.tohex(words[i]) end
    return table.concat(out, " ")
end

local function hexOf(values)
    local at = memory(#values * 4)
    putFloats(at, 0, values)
    return hexAt(at, 0, #values)
end

-- Rotations by rows, in the engine's order: a product is `a b`.
local function multiply(a, b)
    local out = {}
    for row = 0, 2 do
        for column = 0, 2 do
            local sum = 0
            for k = 0, 2 do sum = sum + a[row * 3 + k + 1] * b[k * 3 + column + 1] end
            out[row * 3 + column + 1] = sum
        end
    end
    return out
end

local function about(axis, angle)
    local c, s = math.cos(angle), math.sin(angle)
    if axis == "x" then return { 1, 0, 0, 0, c, -s, 0, s, c } end
    if axis == "y" then return { c, 0, s, 0, 1, 0, -s, 0, c } end
    return { c, -s, 0, s, c, 0, 0, 0, 1 }
end

local IDENTITY = { 1, 0, 0, 0, 1, 0, 0, 0, 1 }

local function transform(rotation, translation, scale)
    local out = {}
    for _, value in ipairs(rotation) do out[#out + 1] = value end
    for _, value in ipairs(translation) do out[#out + 1] = value end
    out[#out + 1] = scale
    return out
end

-- A camera by its axes (the columns of the rotation: right, up, direction) and its place.
local function camera(rotation, location)
    local r = rotation
    return { r[3], r[6], r[9], r[2], r[5], r[8], r[1], r[4], r[7], location[1], location[2], location[3] }
end

local function setLocal(at, value)
    local rotation = memory(36)
    putFloats(rotation, 0, { unpack(value, 1, 9) })
    ffi.cast("void**", at + LOCAL_ROTATE)[0] = rotation
    putFloats(at, LOCAL_TRANSLATE, { value[10], value[11], value[12] })
    putFloats(at, LOCAL_SCALE, { value[13] })
end

-- `links` are { kind = 0 to 3 (3 a plain node unless `flagged`, then a billboard of mode 3), local = transform }.
-- The last link is the effect.
local function case(file, name, coordinates, root, links, model, view)
    local rootNode = memory(NODE_SIZE)
    putFloats(rootNode, WORLD, root)
    local cameraAt = memory(CAMERA_SIZE)
    putFloats(cameraAt, CAMERA_DIRECTION, { unpack(view, 1, 9) })
    putFloats(cameraAt, CAMERA_LOCATION, { view[10], view[11], view[12] })

    local parent = rootNode
    local kinds, locals = {}, {}
    for i, link in ipairs(links) do
        kinds[i] = tostring(link.flagged and 3 or link.kind)
        locals[i] = hexOf(link.value)
        if i < #links then
            local node = memory(NODE_SIZE)
            ffi.cast("void**", node + PARENT)[0] = parent
            setLocal(node, link.value)
            if link.flagged or link.kind ~= 3 then
                ffi.cast("uint16_t*", node + FLAGS)[0] = 8 + (link.flagged and 3 or link.kind) * 32
                rotateToCamera(node, cameraAt)
            else
                updateWorldData(node)
            end
            parent = node
        end
    end

    local effect = memory(EFFECT_SIZE)
    ffi.cast("void**", effect + PARENT)[0] = parent
    setLocal(effect, links[#links].value)
    putFloats(effect, MODEL_MATRIX, { unpack(model, 1, 9) })
    putFloats(effect, MODEL_TRANSLATION, { model[10], model[11], model[12] })
    ffi.cast("uint32_t*", effect + COORDINATES)[0] = coordinates
    effectUpdateWorldData(effect)

    file:write(table.concat({ name, tostring(coordinates), table.concat(kinds, " "), hexOf(root), table.concat(locals, " "), hexOf(model), hexOf(view),
        hexAt(effect, WORLD, 13), hexAt(effect, WORLD_MATRIX, 12) }, "|"), "\n")
end

return function(path)
    local file = assert(io.open(path, "w"))
    file:write("# Made by tools/effect_turn_vectors.lua in the running game: the engine's own functions on made objects.\n")

    local plainRoot = transform(IDENTITY, { 0, 0, 0 }, 1)
    local roots = {
        plainRoot,
        transform(multiply(about("z", 0.7), about("x", -0.4)), { 41234.5, -60321.25, 812.75 }, 1),
        transform(multiply(about("y", 1.9), about("z", -2.2)), { -1500.5, 3200.25, -77.5 }, 2.5),
    }
    local views = {
        camera(about("z", 0.3), { 0, -500, 100 }),
        camera(multiply(about("z", 2.1), about("x", 0.5)), { 41300, -60000, 900 }),
        camera(multiply(multiply(about("z", -1.2), about("x", -0.9)), about("y", 0.2)), { -1200, 3000, 50 }),
        camera(multiply(about("x", 1.5), about("z", 0.01)), { 300, 200, -4000 }),
    }
    local model = { 1 / 240, 0.001, 0, 0, 1 / 240, -0.002, 0.0005, 0, 1 / 400, 0.5, 0.5, 1 }
    local billboardLocal = transform(about("z", 0.25), { 30, -20, 10 }, 1)
    local scaledLocal = transform(about("x", -0.6), { 5, 15, -25 }, 0.5)
    local effectLocal = transform(about("y", 0.4), { 8, 0, -4 }, 1)
    local scaledEffectLocal = transform(about("z", 1.1), { -3, 6, 2 }, 1.5)

    -- Each pair of the modes, for each camera and each root.
    for mode = 0, 2 do
        for coordinates = 0, 4 do
            for v, view in ipairs(views) do
                for r, root in ipairs(roots) do
                    local odd = (v + r) % 2 == 0
                    case(file, string.format("pair b%d c%d v%d r%d", mode, coordinates, v, r), coordinates, root,
                        { { kind = mode, value = odd and scaledLocal or billboardLocal }, { kind = 3, value = odd and scaledEffectLocal or effectLocal } }, model, view)
                end
            end
        end
    end

    -- A plain node between, two turning billboards, and a billboard of mode 3 in the chain.
    local between = transform(about("x", 0.8), { 12, 0, 40 }, 1.25)
    for first = 0, 2 do
        for second = 0, 2 do
            for _, coordinates in ipairs({ 0, 2, 4 }) do
                for v, view in ipairs(views) do
                    case(file, string.format("nested b%d b%d c%d v%d", first, second, coordinates, v), coordinates, roots[1 + v % 3],
                        { { kind = first, value = billboardLocal }, { kind = 3, value = between }, { kind = second, value = scaledLocal }, { kind = 3, value = effectLocal } }, model, view)
                end
            end
        end
    end
    for mode = 0, 2 do
        case(file, string.format("under mode 3 b%d", mode), 0, roots[2], { { kind = mode, value = billboardLocal }, { kind = 3, flagged = true, value = between }, { kind = 3, value = effectLocal } }, model, views[2])
    end

    -- The identity branches. Mode 0: the camera's up and right both across the Y axis of the
    -- node. Mode 1: the camera on the Y axis of the node. Then just past each limit.
    local plainLink = transform(IDENTITY, { 0, 0, 0 }, 1)
    local alongY = { 0, 1, 0, 0, 0, 1, 1, 0, 0, 10, 20, 30 }
    local nearlyAlongY = { 0, 1, 0, 0, 0.0000005, 1, 1, 0, 0, 10, 20, 30 }
    local pastAlongY = { 0, 1, 0, 0, 0.000002, 1, 1, 0, 0, 10, 20, 30 }
    local onAxis = { 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 100, 0 }
    local nearlyOnAxis = { 0, 0, 1, 0, 1, 0, 1, 0, 0, 1e-13, 100, 0 }
    local pastOnAxis = { 0, 0, 1, 0, 1, 0, 1, 0, 0, 2e-12, 100, 0 }
    for _, coordinates in ipairs({ 0, 1, 2, 3, 4 }) do
        local links = function(mode) return { { kind = mode, value = plainLink }, { kind = 3, value = effectLocal } } end
        case(file, "identity facing c" .. coordinates, coordinates, plainRoot, links(0), model, alongY)
        case(file, "identity facing nearly c" .. coordinates, coordinates, plainRoot, links(0), model, nearlyAlongY)
        case(file, "past facing c" .. coordinates, coordinates, plainRoot, links(0), model, pastAlongY)
        case(file, "identity about y c" .. coordinates, coordinates, plainRoot, links(1), model, onAxis)
        case(file, "identity about y nearly c" .. coordinates, coordinates, plainRoot, links(1), model, nearlyOnAxis)
        case(file, "past about y c" .. coordinates, coordinates, plainRoot, links(1), model, pastOnAxis)
    end
    file:close()
    kept = {}
    return "written " .. path
end
