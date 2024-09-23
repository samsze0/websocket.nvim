if _G["_WEBSOCKET_NVIM"] == nil then
  error("Run setup() first to initialize this plugin")
end

local websocket_server_ffi = require("websocket_ffi").server
local uuid_utils = require("utils.uuid")

local M = {}

---@class WebsocketServer
---@field server_id string
---@field host string
---@field port number
---@field extra_response_headers table<string, string>
---@field on_message fun(server: WebsocketServer, client_id: string, message: string)
---@field on_client_disconnect fun(server: WebsocketServer, client_id: string)
---@field on_client_connect fun(server: WebsocketServer, client_id: string)
---@field on_error fun(server: WebsocketServer, err: WebsocketClientError)
local WebsocketServer = {}
WebsocketServer.__index = WebsocketServer
WebsocketServer.__is_class = true
M.WebsocketServer = WebsocketServer

---@type table<string, WebsocketServer>
local WebsocketServerMap = {}

---@enum WebsocketServerErrorType
local ErrorType = {
  ClientConnectionError = "client_connection_error",
  ClientTerminationError = "client_termination_error",
  ServerTerminationError = "server_termination_error",
  ReceiveMessageError = "receive_message_error",
  SendMessageError = "send_message_error",
  BroadcastMessageError = "broadcast_message_error",
}
M.ErrorType = ErrorType

---@alias WebsocketServerError { type: WebsocketServerErrorType, message: string }

-- Create a new websocket server
--
---@param opts { host: string, port: number, extra_response_headers?: table<string, string>, on_message: fun(server: WebsocketServer, client_id: string, message: string), on_client_disconnect?: fun(server: WebsocketServer, client_id: string), on_client_connect?: fun(server: WebsocketServer, client_id: string), on_error?: fun(server: WebsocketServer, err: WebsocketServerError) }
---@return WebsocketServer
function WebsocketServer.new(opts)
  local server_id = uuid_utils.v4()
  local obj = {
    server_id = server_id,
    host = opts.host,
    port = opts.port,
    extra_response_headers = opts.extra_response_headers or {},
    on_message = opts.on_message,
    on_client_disconnect = opts.on_client_disconnect,
    on_client_connect = opts.on_client_connect,
    on_error = opts.on_error,
  }
  setmetatable(obj, WebsocketServer)
  WebsocketServerMap[server_id] = obj
  return obj
end

-- Connect to the websocket server
function WebsocketServer:try_start()
  _G["_WEBSOCKET_NVIM"].servers.callbacks[self.server_id] = {
    on_message = function(server_id, server_client_id, message)
      local server = WebsocketServerMap[server_id]
      if not server then
        error("Received message but client not found", server_id)
      end

      if server.on_message then server.on_message(self, server_client_id, message) end
    end,
    on_client_disconnect = function(server_id, server_client_id)
      local server = WebsocketServerMap[server_id]
      if not server then
        error("Disconnected but client not found", server_id)
      end

      if server.on_client_disconnect then server.on_client_disconnect(self, server_client_id) end
    end,
    on_client_connect = function(server_id, server_client_id)
      local server = WebsocketServerMap[server_id]
      if not server then error("Connected but client not found", server_id) end

      if server.on_client_connect then server.on_client_connect(self, server_client_id) end
    end,
    on_error = function(server_id, err)
      local server = WebsocketServerMap[server_id]
      if not server then
        error("Encounters error but client not found", server_id)
      end

      if server.on_error then server.on_error(self, err) end
    end,
  }
  websocket_server_ffi.start(
    self.server_id,
    self.host,
    self.port,
    self.extra_response_headers
  )
end

-- Check if the websocket server is active
--
---@return boolean
function WebsocketServer:is_active()
  return WebsocketServer.get_servers()[self.server_id] ~= nil
end

-- Get all active websocket servers
--
---@return table<string, { id: string, host: string, port: number }>
function WebsocketServer.get_servers()
  local servers = websocket_server_ffi.get_servers()
  return servers
end

-- Send data to the websocket server client
--
---@param client_id string
---@param data string
function WebsocketServer:try_send_data_to_client(client_id, data)
  websocket_server_ffi.send_data_to_client(self.server_id, client_id, data)
end

-- Disconnect from the websocket server
--
---@param client_id string
function WebsocketServer:try_disconnect_client(client_id)
  websocket_server_ffi.disconnect_client(self.server_id, client_id)
end

-- Broadcast data to all active clients
--
---@param data string
function WebsocketServer:try_broadcast_data_to_clients(data)
  websocket_server_ffi.broadcast_data(self.server_id, data)
end

return M
