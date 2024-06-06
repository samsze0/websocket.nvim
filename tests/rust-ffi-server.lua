local current_path = debug.getinfo(1).source:match("@?(.*/)")
vim.opt.runtimepath:append(current_path .. "../rust")

local uuid_utils = require("utils.uuid")

local PORT = 12010
local FZF_API_KEY = "test"

_G["_WEBSOCKET_NVIM"] = {
    servers = {
        callbacks = {}
    }
}

local current_path = debug.getinfo(1).source:match("@?(.*/)")
vim.opt.runtimepath:append(current_path .. "../../rust")
local websocket_server_ffi = require("websocket_ffi").server

print("FFI", vim.inspect(websocket_server_ffi))
local server_id = uuid_utils.v4()
_G["_WEBSOCKET_NVIM"].servers.callbacks[server_id] = {
    on_message = function(server_id, server_client_id, message)
        print("Callback: Server", server_id, "received message", message, "from client", server_client_id)
    end,
    on_client_disconnect = function(server_id, server_client_id)
        print("Callback: Client", server_client_id, "disconnected from", server_id)
    end,
    on_client_connect = function(server_id, server_client_id)
        print("Callback: Client", server_client_id, "connected to", server_id)

        websocket_server_ffi.send_data_to_client(server_id, server_client_id, "pos(3)+websocket-broadcast@Hi from server@")

        vim.fn.system("sleep 1")

        websocket_server_ffi.broadcast_data(server_id, "pos(5)+websocket-broadcast@Hi again from server@")

        local is_active = websocket_server_ffi.is_client_active(server_id, server_client_id)
        print("Is client active", is_active)

        -- Schedule to run in 5 seconds
        -- vim.defer_fn(function()
        --   websocket_server_ffi.terminate_client(server_id, server_client_id)
        -- end, 5000)
    end,
    on_error = function(server_id, error)
        print("Callback: Server", server_id, "encounters error", vim.inspect(error))
    end
}
websocket_server_ffi.start(server_id, "localhost", PORT, {
    ["Fzf-Api-Key"] = FZF_API_KEY
})

local is_active = websocket_server_ffi.is_active(server_id)
print("Is active", is_active)

-- Schedule to run in 5 seconds
-- vim.defer_fn(function()
--   websocket_server_ffi.terminate(server_id)
-- end, 5000)
