require("websocket").setup({})

local WebsocketClient = require("websocket.client").WebsocketClient
local WebsocketServer = require("websocket.server").WebsocketServer

local PORT = 12001

local client_1_info = {
    connected = false,
    last_message = nil,
    last_error = nil
}

local client_2_info = {
    connected = false,
    last_message = nil,
    last_error = nil
}

local client_1 = WebsocketClient.new({
  connect_addr = ("ws://localhost:%d"):format(PORT),
  on_message = function(self, message)
    print("Client 1 received message: " .. message)
    client_1_info.last_message = message
  end,
  on_connect = function(self)
    print("Client 1 connected")
    client_1_info.connected = true
  end,
  on_disconnect = function(self)
    print("Client 1 disconnected")
    client_1_info.connected = false
  end,
  on_error = function(self, err)
    print("Client 1 encountered error", vim.inspect(err))
    client_1_info.last_error = err
  end,
  extra_headers = {
    ["Extra-test-header"] = "test-value",
  },
})

local client_2 = WebsocketClient.new({
  connect_addr = ("ws://localhost:%d"):format(PORT),
  on_message = function(self, message)
    print("Client 2 received message: " .. message)
    client_2_info.last_message = message
  end,
  on_connect = function(self)
    print("Client 2 connected")
    client_2_info.connected = true
  end,
  on_disconnect = function(self)
    print("Client 2 disconnected")
    client_2_info.connected = false
  end,
  on_error = function(self, err)
    print("Client 2 encountered error", vim.inspect(err))
    client_2_info.last_error = err
  end,
  extra_headers = {
    ["Extra-test-header"] = "test-value",
  },
})

T.assert(not client_1:is_active())
T.assert(not client_2:is_active())
T.assert_deep_eq(WebsocketClient.get_clients(), {})

local server_info = {
    host = "localhost",
    port = PORT
}

local server = WebsocketServer.new({
    host = server_info.host,
    port = server_info.port,
    extra_response_headers = {
        ["Extra-test-header"] = "test-value",
    },
    on_message = function(self, client_id, message)
        print("Server received message from client " .. client_id .. ": " .. message)
        self:try_send_data_to_client(client_id, "Reply from server")
    end,
    on_client_connect = function(self, client_id)
        print("Client " .. client_id .. " connected")
    end,
    on_client_disconnect = function(self, client_id)
        print("Client " .. client_id .. " disconnected")
    end,
    on_error = function(self, err)
        print("Server encountered error", vim.inspect(err))
    end,
})

T.assert_deep_eq(WebsocketServer.get_servers(), {})

server:try_start()

vim.cmd("sleep 1")

T.assert(server:is_active())

local client_1_connect_error = client_1:connect()
local client_2_connect_error = client_2:connect()
assert(not client_1_connect_error, client_1_connect_error)
assert(not client_2_connect_error, client_2_connect_error)

vim.cmd("sleep 1")

T.assert(client_1:is_active())
T.assert(client_2:is_active())

client_1:send_data("Hi from client 1")
client_2:send_data("Hi from client 2")

vim.cmd("sleep 1")

T.assert_eq(client_1_info.last_message, "Reply from server")
T.assert_eq(client_2_info.last_message, "Reply from server")