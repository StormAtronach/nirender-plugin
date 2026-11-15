---@diagnostic disable
-- The mge table that the harness's own Lua reads (W486). A run on the engine's own renderer
-- (`spec.engineReference`: `--engine-reference` and `--plain-case` of tools/run_own_device.py)
-- has MGE XE with `runtime.disabled`: it draws nothing, so the scenario, the batches and the cards
-- need stand-ins for what they ask of it. The stand-ins live in this view only. The global `mge`
-- stays as MWSE made it, so the game's mods see what a game without MGE XE shows them: a stand-in in
-- the global table made mods take MGE XE as present and call what the stand-in lacked
-- (run 20261115-185101-w471-plain: 14,993 errors in event callbacks). The run's `--lua-file` reads
-- the global `mge` too, as it stands for a mod's Lua (`mgeview.luaFileChunk`, W494).

local mgeview = {}

-- The picture of an engine-reference run: the engine's draw of the world camera and then of the
-- arm camera into a rendered texture, read back: the world and the first-person arms with no
-- interface, at 2048 x 1024, so wider than the screen's shape. The two draws keep the order of the
-- engine's frame (`renderMainScene` 0x41C400): the world camera's clear of all and its Click, then
-- the arm camera's clear of depth and stencil (0x41C6E4) and its Click (0x41C734). The water and
-- the shadows of that frame are not drawn here. Two others give nothing here:
-- `niRenderer:takeScreenshot` returns nil, and a window capture is black, as the game has exclusive
-- full screen. The runner turns the TGA files of the run folder into PNG files at the end.
local function enginePicture(params)
    local world = tes3.worldController.worldCamera
    local arms = tes3.worldController.armCamera
    local texture = niRenderedTexture.create(2048, 1024)
    world.renderer:setRenderTarget(texture)
    world.camera:clear()
    world.camera:click(false)
    arms.camera:clear(ni.cameraClearFlags.zBuffer + ni.cameraClearFlags.stencil)
    arms.camera:click(false)
    world.renderer:setRenderTarget(nil)
    local pixels = niPixelData.new(2048, 1024)
    texture:readback(pixels)
    pixels:exportTGA((params.path:gsub("%.png$", ".tga")))
end

local engineView = nil

-- The mge table for the harness's Lua in a run of `spec`: the global `mge` itself, or in an
-- engine-reference run one table with the stand-ins, whose other fields read the global `mge`.
function mgeview.of(spec)
    if not (spec and spec.engineReference) then
        return mge
    end
    if not engineView then
        local standIns = {
            saveScreenshot = enginePicture,
            shaders = { list = {}, find = function() return nil end, load = function() return nil end },
            lightingMode = { vertex = 0, perPixel = 1, perPixelInteriorOnly = 2 },
            getLightingMode = function() return "no MGE XE" end,
            setLightingMode = function() end,
        }
        engineView = setmetatable(standIns, { __index = function(_, key) return mge and mge[key] end })
    end
    return engineView
end

-- The environment for the Lua of a batch or a card (`loadstring`) in a run of `spec`, or nil when
-- the chunk keeps the global one: in an engine-reference run, `mge` is the view, every other name is
-- the global one, and a write of a global goes to the global table. Functions that the chunk makes
-- (a set's `run`) get the same environment, and so do the chunks that it loads itself with
-- `loadstring`, `load`, `loadfile` or `dofile` (a batch that runs a base batch).
function mgeview.environment(spec)
    if not (spec and spec.engineReference) then
        return nil
    end
    local environment

    local function inEnvironment(chunk, ...)
        if chunk then
            setfenv(chunk, environment)
        end
        return chunk, ...
    end

    -- The fields are in the table before its metatable: a later write would go to _G.
    environment = {
        mge = mgeview.of(spec),
        loadstring = function(...) return inEnvironment(loadstring(...)) end,
        load = function(...) return inEnvironment(load(...)) end,
        loadfile = function(...) return inEnvironment(loadfile(...)) end,
        dofile = function(path)
            local chunk, problem = inEnvironment(loadfile(path))
            if not chunk then
                error(problem, 2)
            end
            return chunk()
        end,
    }
    return setmetatable(environment, { __index = _G, __newindex = _G })
end

-- The chunk of the run's `--lua-file`: the Lua of "a state that a mod makes from Lua", so it reads
-- what the game's mods read, the global `mge`, in every run (W494). The Lua of a batch and of a card
-- reads the view (`mgeview.environment`).
function mgeview.luaFileChunk(text)
    return loadstring(text)
end

return mgeview
