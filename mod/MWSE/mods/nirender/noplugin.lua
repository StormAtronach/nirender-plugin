-- The stand-in for the Lua table of akulakhan.dll in a run without the plugin: a reference run
-- of tools/run_own_device.py (--reference, --engine-reference) leaves the DLL out, so that the
-- game runs no code of the plugin. The scenario and the batches call the table; here no call
-- makes a capture, a stream or a port. A call of a name that the stand-in does not have stops
-- with an error that names it, so that a step that needs the plugin fails in the log.

local standIn = {
    version = "none",
    ports = 0,
}

function standIn.ownDevice() return false end

-- The scenario reads "off" at the start of the status: the picture is then that of MGE XE.
function standIn.captureStatus() return "off: no plugin in this run" end

function standIn.capture() return false, "no plugin in this run" end
function standIn.captureFrames() return false end
function standIn.captureEnd() end
function standIn.captureStop() end
function standIn.streamEnd() end
function standIn.streamMark() end
function standIn.streamShot() end
function standIn.retained() end
function standIn.threadCensus() return "no plugin in this run" end
function standIn.report() return "no plugin in this run\n" end
function standIn.tune() error("nirender.tune: no plugin in this run") end

-- The names that the scenario tests for presence before a call (`if nirender.overlay then`):
-- the stand-in lacks them, as a plugin without them would.
local optional = { overlay = true, portTimes = true }

return setmetatable(standIn, {
    __index = function(_, name)
        if optional[name] then
            return nil
        end
        error(string.format("nirender.%s: no plugin in this run", tostring(name)), 2)
    end,
})
