-- Level 4 of the NIF tests: the cases of a mod that draws the world through a camera, as a mirror or a photo does. The tour step
-- (owndevice.lua) calls `wrap` at each stop; a stop of an entry that this file knows turns its case on and turns it off again
-- before the tour goes on. The files are made by tools/make_test_nifs.py (`make_level4`).
--
--   @nif_x_level4_mirror   a pane and a group `subject` (a red box, a green cylinder, a yellow box and a turning billboard with a
--                          magenta triangle). Each frame the world camera, turned 45 degrees round the group, clicks the group,
--                          with the yellow box hidden around the click, into a niRenderedTexture that is the base map of the pane:
--                          the pane shows the red box, the green cylinder and the triangle turned for the mirror camera; the world
--                          shows all four, the triangle turned for the world camera (D82 stage 2b's exhibit).
--   @nif_x_level4_isolate  the group and, beside it in the same file, a blue box `bystander` that is not under the group. What
--                          Isolate Subject of Joy of Painting does (services/PhotoMenu/SubjectFilter.lua, enableOcclusion and
--                          disableOcclusion): the camera's scene is the group, MGE XE's distant land, statics and water are
--                          off, and the water of the game is hidden. The back buffer shows the group on the clear colour and
--                          no blue box: a picture with the blue box is a renderer that draws more than the camera's scene.
local mock = {}

local TEXTURE_SIZE = 256
local PROOF_CLICK = 30
local MIRROR_YAW = math.rad(45)

local function say(format, ...)
    mwse.log("[nirender.mirror_mock] " .. format, ...)
end

-- The mirror: a click into a rendered texture each frame. Returns the function that ends the case.
local function mirror(reference, proofFolder)
    local root = reference.sceneNode
    local pane = root:getObjectByName("pane")
    local subject = root:getObjectByName("subject")
    local yellow = subject:getObjectByName("yellow box")
    local camera = tes3.worldController.worldCamera.cameraData.camera
    local texture = niRenderedTexture.create(TEXTURE_SIZE, TEXTURE_SIZE)
    pane.texturingProperty.maps[1].texture = texture
    pane:updateProperties()
    local world = camera.scene
    local clicks = 0

    -- The mirror camera: the world camera turned MIRROR_YAW round the group (a place and a facing), so that the billboard in
    -- the group turns for a different camera in the click than in the world's walk (D82 stage 2b). The world camera's own
    -- place and facing come back after the click.
    local function click()
        -- The texture as the frame before finished it: this read comes before the click of the proof frame and after a frame
        -- with no read, so no read has flushed the mirror's views for it. It shows what the helper made of the frame without
        -- the flush of a read, and it can fail. The read after the proof click (below) is the check of the flush path.
        if clicks == PROOF_CLICK and proofFolder then
            local pixels = niPixelData.new(TEXTURE_SIZE, TEXTURE_SIZE)
            local read = texture:readback(pixels)
            local path = proofFolder .. "/level4-mirror-texture-late.tga"
            pixels:exportTGA(path)
            say("proof at click %d, before the click (the texture of click %d): readback %s, texture written to %s", clicks, clicks - 1, tostring(read), path)
        end
        local original = camera.scene
        local oldTranslation, oldRotation = camera.translation:copy(), camera.rotation:copy()
        -- The camera hangs under a node that carries its place (the local translation is near zero at the stop), so the
        -- turn is made in world space and written back as a local transform of the parent's space.
        local centre = subject.worldTransform.translation
        local turn = tes3matrix33.new()
        turn:toRotationZ(MIRROR_YAW)
        local current = camera.worldTransform
        local mirrorTranslation = centre + turn * (current.translation - centre)
        local mirrorRotation = turn * current.rotation
        local parent = camera.parent.worldTransform
        local fromParent = parent.rotation:transpose()
        camera.translation = (fromParent * (mirrorTranslation - parent.translation)) * (1 / parent.scale)
        camera.rotation = fromParent * mirrorRotation
        camera:update()
        local mirrorAt = camera.worldTransform.translation:copy()
        yellow.appCulled = true
        subject:update()
        camera.scene = subject
        camera.renderer:setRenderTarget(texture)
        camera:clear()
        camera:click()
        camera:swapBuffers()
        camera.renderer:setRenderTarget(nil)
        local sceneWasSubject = camera.scene == subject
        camera.scene = original
        camera.translation = oldTranslation
        camera.rotation = oldRotation
        camera:update()
        yellow.appCulled = false
        subject:update()
        -- The proof of the click, after the restore so that a fault in it leaves the world camera as it was: the
        -- rendered texture itself, written once as a picture of its own.
        if clicks == PROOF_CLICK and proofFolder then
            local pixels = niPixelData.new(TEXTURE_SIZE, TEXTURE_SIZE)
            local read = texture:readback(pixels)
            local path = proofFolder .. "/level4-mirror-texture.tga"
            pixels:exportTGA(path)
            say("proof at click %d: readback %s, texture written to %s; camera scene was the subject: %s", clicks, tostring(read), path, tostring(sceneWasSubject))
            local worldAt = camera.worldTransform.translation
            say("proof at click %d: the mirror camera stood at %s, %.0f units from the world camera at %s; the group is at %s", clicks, tostring(mirrorAt),
                mirrorAt:distance(worldAt), tostring(worldAt), tostring(subject.worldTransform.translation))
        end
        clicks = clicks + 1
    end
    event.register(tes3.event.enterFrame, click)
    say("mirror on: the pane shows a texture of %d x %d, the yellow box is hidden around each click", TEXTURE_SIZE, TEXTURE_SIZE)

    return function()
        event.unregister(tes3.event.enterFrame, click)
        camera.scene = world
        yellow.appCulled = false
        subject:update()
        say("mirror off: %d clicks, the camera's scene is the world again: %s", clicks, tostring(camera.scene == world))
    end
end

-- The isolation of Joy of Painting, in its order. Returns the function that ends the case.
local function isolate(reference)
    local subject = reference.sceneNode:getObjectByName("subject")
    local oldRoot = tes3.getCamera().scene
    tes3.getCamera().scene = subject
    local oldDistantLand = mge.render.distantLand
    mge.render.distantLand = false
    local oldDistantStatics = mge.render.distantStatics
    mge.render.distantStatics = false
    local oldDistantWater = mge.render.distantWater
    mge.render.distantWater = false
    ---@diagnostic disable-next-line: undefined-field
    if tes3.dataHandler.waterController.waterPlane.appCulled == false then
        tes3.runLegacyScript({ command = "ToggleWater" })
    end
    say("isolate on: the camera's scene is the subject: %s; distant land %s, statics %s and water %s were on",
        tostring(tes3.getCamera().scene == subject), tostring(oldDistantLand), tostring(oldDistantStatics), tostring(oldDistantWater))

    return function()
        if oldDistantLand ~= nil then
            mge.render.distantLand = oldDistantLand
        end
        if oldDistantStatics ~= nil then
            mge.render.distantStatics = oldDistantStatics
        end
        if oldDistantWater ~= nil then
            mge.render.distantWater = oldDistantWater
        end
        if oldRoot ~= nil then
            tes3.getCamera().scene = oldRoot
        end
        ---@diagnostic disable-next-line: undefined-field
        if tes3.dataHandler.waterController.waterPlane.appCulled == true then
            tes3.runLegacyScript({ command = "ToggleWater" })
        end
        say("isolate off: the camera's scene is the world again: %s", tostring(tes3.getCamera().scene == oldRoot))
    end
end

local cases = {
    ["@nif_x_level4_mirror"] = mirror,
    ["@nif_x_level4_isolate"] = isolate,
}

-- Turns on the case of the stop at `reference` (nothing for an entry without one) and returns a `go` that turns it off first.
function mock.wrap(reference, go, proofFolder)
    local case = cases[reference.object.id:lower()]
    if not case then
        return go
    end
    local finish = case(reference, proofFolder)
    return function(...)
        finish()
        return go(...)
    end
end

return mock
