--[[
    The front tool of the NIF test level (D176, W225). A livecoding script: the owner sets by hand the camera
    place of each stop of the tour, in the game. scenario.tour reads the result.

    Use: copy this file as Data Files/MWSE/mods/livecoding/livecoding.lua of the Livecoding mod, turn the mod on,
    start the game from Mod Organizer, load a game with nifTest.esp on, press Alt+X (loads the script), then
    Ctrl+Alt+F (starts and stops the tool). Turn the Livecoding mod off after the session: a harness run refuses
    to start while it is on.

    The stops are those of scenario.tour, in its order: in the cells @nifTest_1 to 3, each reference whose id
    starts with "@nif_", without "tile" in it, and not disabled, in the order of iterateReferences. The tool
    names them n/103 and with the gate's names (1-01 to 3-27).

    Keys (numpad; the tool takes them only while it runs and no menu is open):
        Num6 / Num4        next / previous stop
        Num5               accept: the camera place of now is the front of this stop
        Num8 / Num2        nearer / farther (around the centre of the exhibit's bound)
        Num9 / Num7        around the exhibit, to the left / right (the view turns with it)
        Num1 / Num3        down / up
        Num/ / Num*        look up / down
        Shift or Ctrl + one of the above: small steps
        Num0               the default place (300 south of the exhibit, facing north)
        Num.               undo the last accept or skip
        Num-               skip this stop ("no good view"), with a note
        Num+               the next stop without a front or a skip
        Home               this help
        End                hide or show all the UI (the HUD and the tool's text): as the gate's pictures
    Free flight: while the tool runs, the player flies and has no collision; move with the normal keys and
    the mouse, then press Num5. The view is first person while the tool runs.

    The file: Data Files/MWSE/config/nif_fronts.json (in Mod Organizer: overwrite/MWSE/config/), written after
    each accept, skip and undo, and at no other time. At each start the tool copies the file as it is to
    nif_fronts-before.json in the same folder. If the file is there but is not a front file, the tool does not
    start. The tool never saves the game.
]]

local FILE = "Data Files\\MWSE\\config\\nif_fronts.json"
local COPY = "Data Files\\MWSE\\config\\nif_fronts-before.json"
local VERSION = 1
local CELLS = { "@nifTest_1", "@nifTest_2", "@nifTest_3" }
local PREFIX = "@nif_"
local PLUGIN = "nifTest.esp"
-- The horizontal FOV of the gate's tour runs: fov = 91.30851 in the mgeXE.toml of the gate's tour run
-- results/20261103-195324-v-tour and of results/20261109-165731-bob-w225-probe3 (line 26).
local GATE_FOV = 91.30851
-- The default place of a stop: scenario.tourMove (300 units south of the exhibit, orientation 0).
local DEFAULT_DISTANCE = 300
-- The steps: big, and small with Shift or Ctrl.
local STEP = { distance = { 25, 5 }, height = { 10, 2 }, orbit = { 5, 1 }, pitch = { 2, 0.5 } }

-- The gate's stops (results/baseline-records/tour/accepted, W225 w225-stops.txt): the ids in the tour's order.
local EXPECTED = {
    [1] = { -- 42 stops
        "@nif_002", "@nif_004", "@nif_009", "@nif_010", "@nif_011", "@nif_012", "@nif_013", "@nif_014",
        "@nif_015", "@nif_017", "@nif_018", "@nif_019", "@nif_020", "@nif_022", "@nif_023", "@nif_024",
        "@nif_025", "@nif_026", "@nif_028", "@nif_031", "@nif_034", "@nif_021", "@nif_038", "@nif_039",
        "@nif_040", "@nif_041", "@nif_054", "@nif_056", "@nif_057", "@nif_058", "@nif_059", "@nif_060",
        "@nif_061", "@nif_062", "@nif_066", "@nif_067", "@nif_044", "@nif_078", "@nif_092", "@nif_077",
        "@nif_071", "@nif_083",
    },
    [2] = { -- 34 stops
        "@nif_003", "@nif_005", "@nif_006", "@nif_007", "@nif_008", "@nif_016", "@nif_042", "@nif_050",
        "@nif_051", "@nif_069", "@nif_070", "@nif_072", "@nif_073", "@nif_074", "@nif_081", "@nif_082",
        "@nif_086", "@nif_093", "@nif_043", "@nif_045", "@nif_046", "@nif_094", "@nif_096", "@nif_097",
        "@nif_098", "@nif_087", "@nif_099", "@nif_095", "@nif_088", "@nif_085", "@nif_089", "@nif_048",
        "@nif_047", "@nif_049",
    },
    [3] = { -- 27 stops
        "@nif_064", "@nif_065", "@nif_052", "@nif_063", "@nif_076", "@nif_079", "@nif_080", "@nif_101",
        "@nif_102", "@nif_103", "@nif_104", "@nif_084", "@nif_090", "@nif_091", "@nif_105", "@nif_106",
        "@nif_107", "@nif_108", "@nif_109", "@nif_027", "@nif_029", "@nif_030", "@nif_032", "@nif_033",
        "@nif_035", "@nif_036", "@nif_037",
    },
}

-- The stops of all cells: { cell = index, cellName, slot = index in the cell, id, name = "1-02" }.
local STOPS = {}
for cell, ids in ipairs(EXPECTED) do
    for slot, id in ipairs(ids) do
        STOPS[#STOPS + 1] = { cell = cell, cellName = CELLS[cell], slot = slot, id = id, name = string.format("%d-%02d", cell, slot) }
    end
end

local HUD_ID = "NifFronts_HUD"
local NOTE_MENU = "NifFronts_Note"

local tool = {
    on = false,
    index = 1,
    data = nil,      -- the file: { version, fronts = { [key] = entry } }
    undo = {},       -- { key, previous entry or false }
    live = nil,      -- the references of the current cell, in the tour's order
    liveCell = nil,  -- the cell those references are of
    warning = nil,   -- a difference between the live order and EXPECTED
    help = false,
    before = nil,    -- the values the tool changes, set again at its end
    nextHud = 0,
    changes = 0,     -- the writes of the file since the start
    copied = nil,    -- the copy of the file made at the start
}

local function say(fmt, ...)
    local line = string.format(fmt, ...)
    mwse.log("[nif fronts] %s", line)
    return line
end

local function key(stop)
    return stop.name .. " " .. stop.id
end

local function vec(v)
    return { v.x, v.y, v.z }
end

local function v3(t)
    return tes3vector3.new(t[1], t[2], t[3])
end

-- A test outside the owner's session sets _G.nifFrontsTestPath (an absolute path): the tool then reads and
-- writes that file and its copy (the path with ".before"), and nothing in the instance.
local function paths()
    local test = _G.nifFrontsTestPath
    if test then return test, test .. ".before" end
    return FILE, COPY
end

local function readText(path)
    local f = io.open(path, "r")
    if not f then return nil end
    local text = f:read("*a")
    f:close()
    return text
end

local function writeText(path, text)
    local f = assert(io.open(path, "w"))
    f:write(text)
    f:close()
end

-- Reads the file and keeps a copy of it. Returns false and the reason when the file is there but is not a
-- front file: the tool then changes nothing.
local function load()
    local file, copy = paths()
    local text = readText(file)
    local data = { version = VERSION, fronts = {} }
    if text then
        local decoded = json.decode(text)
        if type(decoded) ~= "table" or type(decoded.fronts) ~= "table" then
            return false, string.format("%s is not a front file", file)
        end
        data = decoded
        writeText(copy, text)
    end
    data.version = VERSION
    data.what = "Fronts of the stops of the NIF test level (D176): the camera place for each stop, set by hand. "
        .. "camera: world; local: in the exhibit's frame (offset rotated by the inverse of its rotation, yaw minus its yaw)."
    tool.data = data
    tool.copied = text and copy or nil
    return true
end

local function save()
    local file = paths()
    writeText(file, json.encode(tool.data, { indent = true }))
    tool.changes = tool.changes + 1
end

local function counts()
    local accepted, skipped = 0, 0
    for _, stop in ipairs(STOPS) do
        local entry = tool.data.fronts[key(stop)]
        if entry and entry.status == "accepted" then accepted = accepted + 1 end
        if entry and entry.status == "skip" then skipped = skipped + 1 end
    end
    return accepted, skipped
end

-- Two object ids are the same without case and without spaces at the end (2-04's id is "@nif_007 ").
local function same(a, b)
    return a:lower():gsub("%s+$", "") == b:lower():gsub("%s+$", "")
end

local function cellIndex(name)
    for i, cell in ipairs(CELLS) do
        if cell:lower() == (name or ""):lower() then return i end
    end
end

-- The references of the current cell in the order of scenario.tour.
local function readLive()
    local cell = tes3.player.cell
    local list = {}
    for ref in cell:iterateReferences() do
        local id = ref.object.id:lower()
        if id:sub(1, #PREFIX) == PREFIX and not id:find("tile", 1, true) and not ref.disabled then
            list[#list + 1] = ref
        end
    end
    tool.live, tool.liveCell, tool.warning = list, cell, nil
    local index = cellIndex(cell.id)
    local expected = index and EXPECTED[index]
    if not expected then return end
    local differs = #list ~= #expected
    for i = 1, math.min(#list, #expected) do
        if not same(list[i].object.id, expected[i]) then
            differs = true
            break
        end
    end
    if differs then
        tool.warning = say("the stops of '%s' differ from the gate's list: %d here, %d expected; the numbers may not match the gate",
            cell.id, #list, #expected)
        tes3.messageBox("Nif fronts: %s", tool.warning)
    end
end

local function currentRef()
    local stop = STOPS[tool.index]
    if not tool.live or tool.liveCell ~= tes3.player.cell then return nil end
    local ref = tool.live[stop.slot]
    if ref and same(ref.object.id, stop.id) then return ref end
    -- The order differs: the reference by its id.
    for _, other in ipairs(tool.live) do
        if same(other.object.id, stop.id) then return other end
    end
end

-- The centre of the exhibit for the steps: its bound centre, else its position.
local function centre(ref)
    local node = ref.sceneNode
    return node and node.worldBoundOrigin and node.worldBoundOrigin:copy() or ref.position:copy()
end

-- The camera follows a move of the player one frame later. The height of the eye above the player's place is
-- read when the two agree (no move in this frame); a second move in the same frame starts from the camera place
-- that the first one set (tool.pending, cleared each frame).
local function eyeHeight()
    if not tool.pending then
        local d = tes3.getCameraPosition() - tes3.player.position
        if d.x * d.x + d.y * d.y < 400 then tool.eye = d.z end
    end
    return tool.eye or 125
end

local function cameraNow()
    return tool.pending or tes3.getCameraPosition()
end

local function placeCamera(camera, yaw, pitch)
    local eye = eyeHeight()
    tes3.player.position = tes3vector3.new(camera.x, camera.y, camera.z - eye)
    tes3.player.orientation = tes3vector3.new(pitch, 0, yaw)
    tool.pending = camera:copy()
end

local function placeDefault(ref)
    local eye = eyeHeight()
    local at = ref.position
    tes3.player.position = tes3vector3.new(at.x, at.y - DEFAULT_DISTANCE, at.z)
    tes3.player.orientation = tes3vector3.new(0, 0, 0)
    tool.pending = tes3vector3.new(at.x, at.y - DEFAULT_DISTANCE, at.z + eye)
end

-- The front's camera place in the world, from its values in the exhibit's frame.
local function frontCamera(ref, entry)
    local rotation = ref.sceneNode and ref.sceneNode.worldTransform.rotation
    if entry["local"] and rotation then
        local offset = rotation * v3(entry["local"].offset)
        return ref.position + offset, ref.orientation.z + entry["local"].yaw, entry.camera.pitch
    end
    return v3(entry.camera.position), entry.camera.yaw, entry.camera.pitch
end

local function placeStop()
    local ref = currentRef()
    if not ref then return end
    local entry = tool.data.fronts[key(STOPS[tool.index])]
    if entry and entry.status == "accepted" then
        placeCamera(frontCamera(ref, entry))
    else
        placeDefault(ref)
    end
end

local function goTo(index)
    tool.index = (index - 1) % #STOPS + 1
    local stop = STOPS[tool.index]
    if tes3.player.cell.id:lower() ~= stop.cellName:lower() then
        tes3.positionCell({ cell = stop.cellName, position = tes3.player.position, orientation = tes3vector3.new(0, 0, 0) })
        timer.start({ duration = 1.0, type = timer.real, callback = function()
            if not tool.on then return end
            readLive()
            placeStop()
        end })
        return
    end
    if tool.liveCell ~= tes3.player.cell then readLive() end
    placeStop()
end

-- The camera now, as an entry of the file.
local function entryOfNow(ref, status, note)
    local camera = cameraNow()
    local o = tes3.player.orientation
    local rotation = ref.sceneNode and ref.sceneNode.worldTransform.rotation
    local entry = {
        status = status,
        note = note,
        time = os.date("!%Y-%m-%dT%H:%M:%SZ"),
        id = ref.object.id,
        cell = ref.cell.id,
        exhibit = { position = vec(ref.position), orientation = vec(ref.orientation), scale = ref.scale },
        camera = { position = vec(camera), yaw = o.z, pitch = o.x },
        player = { position = vec(tes3.player.position) },
        fov = mge.camera.fov,
    }
    if rotation then
        entry["local"] = { offset = vec(rotation:transpose() * (camera - ref.position)), yaw = o.z - ref.orientation.z }
    end
    return entry
end

local NOT_LOADED = "Nif fronts: the exhibit of this stop is not loaded here; Num6 or Num4 goes to it"

local function record(status, note)
    local ref = currentRef()
    if not ref then
        tes3.messageBox(NOT_LOADED)
        return
    end
    local k = key(STOPS[tool.index])
    tool.undo[#tool.undo + 1] = { k, tool.data.fronts[k] or false }
    tool.data.fronts[k] = entryOfNow(ref, status, note)
    save()
    local accepted, skipped = counts()
    tes3.messageBox("%s %s (accepted %d/%d, skipped %d)", status == "accepted" and "Accepted" or "Skipped", k, accepted, #STOPS, skipped)
    say("%s %s; accepted %d, skipped %d", status, k, accepted, skipped)
end

local function undo()
    local last = table.remove(tool.undo)
    if not last then
        tes3.messageBox("Nif fronts: nothing to undo")
        return
    end
    tool.data.fronts[last[1]] = last[2] or nil
    save()
    tes3.messageBox("Undone: %s", last[1])
    say("undo %s", last[1])
end

local function askNote(done)
    if not currentRef() then
        tes3.messageBox(NOT_LOADED)
        return
    end
    local menu = tes3ui.createMenu({ id = NOTE_MENU, fixedFrame = true })
    menu:createLabel({ text = "Skip " .. key(STOPS[tool.index]) .. ": a note, then Enter. Cancel: no skip." })
    local input = menu:createTextInput({ id = "NifFronts_NoteInput" })
    input.widget.lengthLimit = 200
    input.minWidth = 400
    local function close()
        menu:destroy()
        tes3ui.leaveMenuMode()
    end
    input:register(tes3.uiEvent.keyEnter, function()
        local text = input.text
        close()
        done(text)
    end)
    local cancel = menu:createButton({ id = "NifFronts_NoteCancel", text = "Cancel" })
    cancel:register(tes3.uiEvent.mouseClick, function()
        close()
        tes3.messageBox("Nif fronts: no skip")
    end)
    menu:updateLayout()
    tes3ui.enterMenuMode(NOTE_MENU)
    tes3ui.acquireTextInput(input)
end

local function nextOpen()
    for step = 1, #STOPS do
        local index = (tool.index + step - 1) % #STOPS + 1
        if not tool.data.fronts[key(STOPS[index])] then return goTo(index) end
    end
    tes3.messageBox("Nif fronts: every stop has a front or a skip")
end

-- The fine steps move the camera around the centre of the exhibit.
local function stepMove(kind, sign, small)
    local ref = currentRef()
    if not ref then return end
    local size = STEP[kind][small and 2 or 1] * sign
    local camera = cameraNow()
    local o = tes3.player.orientation
    local yaw, pitch = o.z, o.x
    local c = centre(ref)
    if kind == "distance" then
        local dx, dy = camera.x - c.x, camera.y - c.y
        local length = math.max(math.sqrt(dx * dx + dy * dy), 1)
        local scale = math.max(length - size, 10) / length
        camera = tes3vector3.new(c.x + dx * scale, c.y + dy * scale, camera.z)
    elseif kind == "height" then
        camera = tes3vector3.new(camera.x, camera.y, camera.z + size)
    elseif kind == "orbit" then
        local a = math.rad(size)
        local dx, dy = camera.x - c.x, camera.y - c.y
        -- Turns the place around the centre clockwise seen from above, as the yaw of the game (0 north,
        -- 90 east) turns.
        local x = dx * math.cos(a) + dy * math.sin(a)
        local y = -dx * math.sin(a) + dy * math.cos(a)
        camera = tes3vector3.new(c.x + x, c.y + y, camera.z)
        yaw = yaw + a
    elseif kind == "pitch" then
        pitch = math.max(math.min(pitch + math.rad(size), math.rad(89)), math.rad(-89))
    end
    placeCamera(camera, yaw, pitch)
end

local HELP = table.concat({
    "Num6/Num4 next/previous   Num5 accept   Num0 default   Num. undo",
    "Num8/Num2 nearer/farther   Num9/Num7 around   Num1/Num3 down/up   Num//Num* look up/down",
    "Shift or Ctrl: small steps   Num- skip with a note   Num+ next stop without a front",
    "Home this help   End hide all UI   Ctrl+Alt+F stop the tool",
    "Free flight: the normal movement keys and the mouse (no collision, flying)",
}, "\n")

local function hudLabel()
    local multi = tes3ui.findMenu("MenuMulti")
    if not multi then return nil end
    local block = multi:findChild(HUD_ID)
    if not block then
        block = multi:createBlock({ id = HUD_ID })
        block.absolutePosAlignX = 0.01
        block.absolutePosAlignY = 0.02
        block.autoWidth = true
        block.autoHeight = true
        block.flowDirection = tes3.flowDirection.topToBottom
        local label = block:createLabel({ id = "NifFronts_Text", text = "" })
        label.color = { 1.0, 1.0, 0.6 }
    end
    return block:findChild("NifFronts_Text"), multi
end

local function hudText()
    local stop = STOPS[tool.index]
    local ref = currentRef()
    local accepted, skipped = counts()
    local lines = { string.format("NIF fronts  %d/%d  %s  %s  in %s   accepted %d/%d  skipped %d", tool.index, #STOPS, stop.name, stop.id,
        stop.cellName, accepted, #STOPS, skipped) }
    if ref then
        local c = centre(ref)
        local camera = tes3.getCameraPosition()
        local to = c - camera
        local distance = to:length()
        local radius = ref.sceneNode and ref.sceneNode.worldBoundRadius or 0
        local seen
        if distance <= radius then
            seen = "camera INSIDE the bound"
        else
            local angle = math.acos(math.max(-1, math.min(1, tes3.getCameraVector():dot(to:normalized()))))
            local margin = math.asin(math.min(1, radius / distance))
            seen = (angle - margin) < math.rad(mge.camera.fov / 2) and "in view" or "OUT OF VIEW"
        end
        local o = tes3.player.orientation
        lines[#lines + 1] = string.format("exhibit at %.0f %.0f %.0f yaw %.0f   bound radius %.0f   distance %.0f   %s",
            ref.position.x, ref.position.y, ref.position.z, math.deg(ref.orientation.z) % 360, radius, distance, seen)
        lines[#lines + 1] = string.format("camera at %.0f %.0f %.0f yaw %.1f pitch %.1f   FOV %.2f", camera.x, camera.y, camera.z,
            math.deg(o.z) % 360, math.deg(o.x), mge.camera.fov)
    else
        lines[#lines + 1] = "the exhibit is not loaded here (Num6/Num4 go to it)"
    end
    local entry = tool.data.fronts[key(stop)]
    if entry then
        lines[#lines + 1] = string.format("saved: %s %s%s", entry.status, entry.time or "", entry.note and entry.note ~= "" and ("  note: " .. entry.note) or "")
    else
        lines[#lines + 1] = "saved: nothing (the default place)"
    end
    if tool.warning then lines[#lines + 1] = "WARNING: " .. tool.warning end
    if tool.help then lines[#lines + 1] = HELP else lines[#lines + 1] = "Home: help" end
    return table.concat(lines, "\n")
end

local function updateHud()
    local label, multi = hudLabel()
    if not label then return end
    label.text = hudText()
    multi:updateLayout()
end

local function onSimulate()
    tool.pending = nil
    if not tool.on then return end
    local now = os.clock()
    if now < tool.nextHud then return end
    tool.nextHud = now + 0.2
    updateHud()
end

local function start()
    if not tes3.isModActive(PLUGIN) then
        tes3.messageBox("Nif fronts: %s is not on in this game. Turn it on in Mod Organizer and load a game.", PLUGIN)
        return
    end
    tool.changes = 0
    local loaded, reason = load()
    if not loaded then
        say("not started: %s", reason)
        tes3.messageBox("Nif fronts: not started: %s. Correct or move the file, then start again.", reason)
        return
    end
    local mobile = tes3.mobilePlayer
    local multi = tes3ui.findMenu("MenuMulti")
    tool.before = { collision = mobile.movementCollision, flying = mobile.isFlying, fov = mge.camera.fov,
        hud = multi and multi.visible, thirdPerson = tes3.is3rdPerson() }
    -- Kept in the real global table too: a reload (Alt+X) while the tool runs makes a new copy, which sets
    -- these values again.
    _G.nifFrontsBefore = tool.before
    tool.on = true
    mobile.movementCollision = false
    mobile.isFlying = true
    mge.camera.fov = GATE_FOV
    tes3.force1stPerson()
    say("start: movement collision %s, flying %s, FOV %.2f, third person %s before; %d fronts in the file; copy %s",
        tostring(tool.before.collision), tostring(tool.before.flying), tool.before.fov, tostring(tool.before.thirdPerson),
        (function() local n = 0 for _ in pairs(tool.data.fronts) do n = n + 1 end return n end)(), tool.copied or "none (no file)")
    local here = cellIndex(tes3.player.cell.id)
    if here then
        readLive()
        -- The stop nearest to the player in this cell.
        local best, bestDistance = nil, math.huge
        for i, stop in ipairs(STOPS) do
            if stop.cell == here then
                local ref = tool.live[stop.slot]
                if ref then
                    local d = ref.position:distance(tes3.player.position)
                    if d < bestDistance then best, bestDistance = i, d end
                end
            end
        end
        tool.index = best or STOPS[1] and 1
        placeStop()
    else
        goTo(1)
    end
    updateHud()
end

local function stop()
    tool.on = false
    _G.nifFrontsBefore = nil
    local mobile = tes3.mobilePlayer
    local b = tool.before or {}
    if mobile then
        mobile.movementCollision = b.collision ~= false
        mobile.isFlying = b.flying == true
    end
    if b.fov then mge.camera.fov = b.fov end
    if b.thirdPerson then tes3.force3rdPerson() end
    local multi = tes3ui.findMenu("MenuMulti")
    if multi then
        local block = multi:findChild(HUD_ID)
        if block then block:destroy() end
        if b.hud ~= nil then multi.visible = b.hud end
        multi:updateLayout()
    end
    say("stop: movement collision %s, flying %s, FOV %.2f, third person %s again; %d writes of the file",
        tostring(mobile and mobile.movementCollision), tostring(mobile and mobile.isFlying), mge.camera.fov,
        tostring(b.thirdPerson == true), tool.changes)
    tes3.messageBox("Nif fronts: stopped; %d writes of the file in this session", tool.changes)
end

local KEYS = {
    [tes3.scanCode.numpad6] = function() goTo(tool.index + 1) end,
    [tes3.scanCode.numpad4] = function() goTo(tool.index - 1) end,
    [tes3.scanCode.numpad5] = function() record("accepted") end,
    [tes3.scanCode.numpad8] = function(small) stepMove("distance", 1, small) end,
    [tes3.scanCode.numpad2] = function(small) stepMove("distance", -1, small) end,
    [tes3.scanCode.numpad9] = function(small) stepMove("orbit", 1, small) end,
    [tes3.scanCode.numpad7] = function(small) stepMove("orbit", -1, small) end,
    [tes3.scanCode.numpad3] = function(small) stepMove("height", 1, small) end,
    [tes3.scanCode.numpad1] = function(small) stepMove("height", -1, small) end,
    [tes3.scanCode.divide] = function(small) stepMove("pitch", -1, small) end,
    [tes3.scanCode.multiply] = function(small) stepMove("pitch", 1, small) end,
    [tes3.scanCode.numpad0] = function() local ref = currentRef() if ref then placeDefault(ref) end end,
    [tes3.scanCode.decimal] = function() undo() end,
    [tes3.scanCode.subtract] = function() askNote(function(text) record("skip", text) end) end,
    [tes3.scanCode.add] = function() nextOpen() end,
    [tes3.scanCode.home] = function() tool.help = not tool.help end,
    [tes3.scanCode["end"]] = function()
        local multi = tes3ui.findMenu("MenuMulti")
        if multi then multi.visible = not multi.visible end
    end,
}

local function onKeyDown(e)
    if e.keyCode == tes3.scanCode.f and e.isControlDown and e.isAltDown then
        if tool.on then stop() else start() end
        return false
    end
    if not tool.on or tes3.menuMode() then return end
    local action = KEYS[e.keyCode]
    if not action then return end
    -- Ctrl too: with NumLock on, Windows can report Shift + a numpad key as the key without Shift.
    action(e.isShiftDown or e.isControlDown)
    updateHud()
    return false
end

event.register(tes3.event.keyDown, onKeyDown)
event.register(tes3.event.simulate, onSimulate)
-- A reload (Alt+X) while the tool ran: the livecoding mod unregistered the old copy's events. This copy starts
-- idle, so it sets the values of before the tool again.
if _G.nifFrontsBefore and tes3.mobilePlayer then
    tool.before = _G.nifFrontsBefore
    stop()
    say("reload: the values of before the tool are set again")
end
say("loaded; Ctrl+Alt+F starts the tool")
tes3.messageBox("Nif fronts loaded: Ctrl+Alt+F starts and stops the tool")

-- For a test outside the livecoding mod: the functions of the tool.
return { tool = tool, start = start, stop = stop, goTo = goTo, record = record, undo = undo, stepMove = stepMove,
    nextOpen = nextOpen, readLive = readLive, hudText = hudText, STOPS = STOPS, placeDefault = placeDefault,
    currentRef = currentRef, frontCamera = frontCamera, key = key }
