local WebsocketClient = require("websocket.client").WebsocketClient

local client = WebsocketClient.new({
    connect_addr = "ws://localhost:8080",
    on_message = function(message)
        print("Received message: " .. message)
    end,
    on_connect = function()
        print("Connected")
    end,
    on_disconnect = function()
        print("Disconnected")
    end
})

client:connect()

-- Schedule to run in 2 seconds
vim.defer_fn(function()
  client:send("Hello server")
end, 5000)

-- Schedule to run in 5 seconds
vim.defer_fn(function()
  client:disconnect()
end, 5000)