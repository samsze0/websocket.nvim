local current_path = debug.getinfo(1).source:match("@?(.*/)")
vim.opt.runtimepath:append(current_path .. "../rust")

local uuid_utils = require("utils.uuid")
local tbl_utils = require("utils.table")

_G["_WEBSOCKET_NVIM"] = {
  clients = {
    callbacks = {},
  },
  servers = {
    callbacks = {},
  },
}

local current_path = debug.getinfo(1).source:match("@?(.*/)")
vim.opt.runtimepath:append(current_path .. "../../rust")
local websocket_client_ffi = require("websocket_ffi").client
local websocket_server_ffi = require("websocket_ffi").server

T.debug(websocket_client_ffi)
T.debug(websocket_server_ffi)

local server_info = {
  id = uuid_utils.v4(),
  port = 12000,
}

_G["_WEBSOCKET_NVIM"].servers.callbacks[server_info.id] = {
  on_message = function(server_id, server_client_id, message)
    print(
      "Server received message",
      message,
      "from client",
      server_client_id
    )
    if message:match("^Hi from client %d$") then
      -- Do nothing
    elseif message:match("^Please broadcast$") then
      websocket_server_ffi.broadcast_data(
        server_id,
        "Broadcast from server"
      )
    elseif message:match("^Please disconnect$") then
      websocket_server_ffi.disconnect_client(
        server_id,
        server_client_id
      )
    end
  end,
  on_client_disconnect = function(server_id, server_client_id)
    print("Client", server_client_id, "disconnected from", server_id)
  end,
  on_client_connect = function(server_id, server_client_id)
    print("Client", server_client_id, "connected to", server_id, "Server clients:", vim.inspect(websocket_server_ffi.get_clients(server_id)))

    websocket_server_ffi.send_data_to_client(
      server_id,
      server_client_id,
      "Reply from server"
    )
  end,
  on_error = function(server_id, error)
    print("Server encountered error", vim.inspect(error))
  end,
}

websocket_server_ffi.start(server_info.id, "localhost", server_info.port, {
  ["Extra-test-header-from-server"] = "test",
})

T.assert_deep_eq(websocket_server_ffi.get_servers(), {
  [server_info.id] = {
    id = server_info.id,
    host = "localhost",
    port = tostring(server_info.port),
  },
})

local client_1_info = {
  id = uuid_utils.v4(),
  last_message = nil,
  disconnected = false,
  connected = false,
  last_error = nil
}

local client_2_info = {
  id = uuid_utils.v4(),
  last_message = nil,
  disconnected = false,
  connected = false,
  last_error = nil
}

_G["_WEBSOCKET_NVIM"].clients.callbacks[client_1_info.id] = {
  on_message = function(client_id, message)
    print("Client 1 received message", message)
    client_1_info.last_message = message
  end,
  on_disconnect = function(client_id)
    print("Client 1 disconnected")
    client_1_info.disconnected = true
  end,
  on_connect = function(client_id)
    print("Client 1 connected to server")
    client_1_info.connected = true
  end,
  on_error = function(client_id, error)
    print("Client 1 encountered error", vim.inspect(error))
    client_1_info.last_error = error
  end,
}

_G["_WEBSOCKET_NVIM"].clients.callbacks[client_2_info.id] = {
  on_message = function(client_id, message)
    print("Client 2 received message", message)
    client_2_info.last_message = message
  end,
  on_disconnect = function(client_id)
    print("Client 2 disconnected")
    client_2_info.disconnected = true
  end,
  on_connect = function(client_id)
    print("Client 2 connected to server")
    client_2_info.connected = true
  end,
  on_error = function(client_id, error)
    print("Client 2 encountered error", vim.inspect(error))
    client_2_info.last_error = error
  end,
}

websocket_client_ffi.connect(
  client_1_info.id,
  ("ws://localhost:%d"):format(server_info.port),
  {
    ["Extra-test-header"] = "test",
  }
)

vim.cmd("sleep " .. 1)

T.assert(client_1_info.connected)
T.assert_eq(tbl_utils.len(websocket_server_ffi.get_clients(server_info.id)), 1)

websocket_client_ffi.send_data(
  client_1_info.id,
  "Hi from client 1"
)

vim.cmd("sleep " .. 1)

T.assert_eq(client_1_info.last_message, "Reply from server")
T.assert_not(client_1_info.last_error)

websocket_client_ffi.connect(
  client_2_info.id,
  ("ws://localhost:%d"):format(server_info.port),
  {
    ["Extra-test-header"] = "test",
  }
)

vim.cmd("sleep " .. 1)

T.assert(client_2_info.connected)
T.assert_eq(tbl_utils.len(websocket_server_ffi.get_clients(server_info.id)), 2)

websocket_client_ffi.send_data(
  client_2_info.id,
  "Hi from client 2"
)

vim.cmd("sleep " .. 1)

T.assert_eq(client_2_info.last_message, "Reply from server")
T.assert_not(client_2_info.last_error)

websocket_client_ffi.disconnect(client_1_info.id)

vim.cmd("sleep " .. 1)

T.assert_eq(tbl_utils.len(websocket_server_ffi.get_clients(server_info.id)), 1)

websocket_server_ffi.terminate(server_info.id)

vim.cmd("sleep " .. 1)

T.assert_deep_eq(websocket_server_ffi.get_servers(), {})