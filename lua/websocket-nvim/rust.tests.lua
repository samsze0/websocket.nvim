local utils = require("samsze0-utils-nvim")

local PORT = 12010
local FZF_API_KEY = "test"

_G["_WEBSOCKET_NVIM"] = {
    callbacks = {}
}

vim.cmd("set rtp+=./rust")
local websocket = require("websocket_nvim")

print(vim.inspect(websocket))
local client_id = utils.uuid()
_G["_WEBSOCKET_NVIM"].callbacks[client_id] = {
    on_message = function(args)
        local client_id = args[1]
        local message = args[2]
        print("Callback: Received message", message)
    end,
    on_disconnect = function(client_id)
        print("Callback: Disconnected from", client_id)
    end,
    on_connect = function(client_id)
        print("Callback: Connected to", client_id)
    end,
}
websocket.new_client(client_id, string.format("ws://localhost:%d", PORT), {
    ["Fzf-Api-Key"] = FZF_API_KEY
})
print(vim.inspect(client_id))

local is_active = websocket.is_active(client_id)
print("Is active", is_active)

websocket.send_data(client_id, "Hello, world!")

local messages = websocket.check_replay_messages(client_id)
print(vim.inspect(messages))

-- Schedule to run in 5 seconds
vim.defer_fn(function()
  websocket.disconnect(client_id)
end, 5000)