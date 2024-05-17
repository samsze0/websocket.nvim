local utils = require("utils")

local PORT = 12010
local FZF_API_KEY = "test"

_G["_WEBSOCKET_NVIM"] = {
    callbacks = {}
}

local current_path = debug.getinfo(1).source:match("@?(.*/)")
vim.opt.runtimepath:append(current_path .. "../../rust")
local websocket_ffi = require("websocket_ffi")

print("FFI", vim.inspect(websocket_ffi))
local client_id = utils.uuid()
_G["_WEBSOCKET_NVIM"].callbacks[client_id] = {
    on_message = function(client_id, message)
        print("Callback: Received message", message)
    end,
    on_disconnect = function(client_id)
        print("Callback: Disconnected from", client_id)
    end,
    on_connect = function(client_id)
        print("Callback: Connected to", client_id)
    end,
    on_error = function(client_id, error)
        print("Callback: Error", vim.inspect(error))
    end
}
websocket_ffi.connect(client_id, string.format("ws://localhost:%d", PORT), {
    ["Fzf-Api-Key"] = FZF_API_KEY
})

local is_active = websocket_ffi.is_active(client_id)
print("Is active", is_active)

websocket_ffi.send_data(client_id, "pos(3)+websocket-broadcast@Hi from server@")

-- Schedule to run in 5 seconds
vim.defer_fn(function()
  websocket_ffi.disconnect(client_id)
end, 5000)
