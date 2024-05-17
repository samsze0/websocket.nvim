local WebsocketClient = require("websocket.client").WebsocketClient

local PORT = 12010
local FZF_API_KEY = "test"

local client = WebsocketClient.new({
    connect_addr = ("ws://localhost:%d"):format(PORT),
    on_message = function(message)
        print("Received message: " .. message)
    end,
    on_connect = function()
        print("Connected")
    end,
    on_disconnect = function()
        print("Disconnected")
    end,
    on_error = function(err)
        print("On error", vim.inspect(err))
    end,
    extra_headers = {
        ["Fzf-Api-Key"] = FZF_API_KEY
    }
})

client:connect()

-- Schedule to run in 2 seconds
vim.defer_fn(function()
  client:send_data("pos(3)+websocket-broadcast@Hi from server@")
end, 2000)

-- Schedule to run in 5 seconds
vim.defer_fn(function()
  client:disconnect()
end, 5000)