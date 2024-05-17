local websocket_ffi = require("websocket_ffi")
local utils = require("utils")

local M = {}

---@class WebsocketClient
---@field client_id string
---@field connect_addr string
---@field extra_headers table<string, string>
---@field on_message fun(message: string)
---@field on_disconnect fun()
---@field on_connect fun()
local WebsocketClient = {}
WebsocketClient.__index = WebsocketClient
WebsocketClient.__is_class = true
M.WebsocketClient = WebsocketClient

---@type table<string, WebsocketClient>
local WebsocketClientMap = {}

-- Create a new websocket client
--
---@param opts { connect_addr: string, extra_headers?: table<string, string>, on_message: fun(message: string), on_disconnect?: fun(), on_connect?: fun() }
---@return WebsocketClient
function WebsocketClient.new(opts)
	local client_id = utils.uuid()
	local obj = {
		client_id = client_id,
		connect_addr = opts.connect_addr,
		extra_headers = opts.extra_headers,
		on_message = opts.on_message,
		on_disconnect = opts.on_disconnect,
		on_connect = opts.on_connect,
	}
	setmetatable(obj, WebsocketClient)
	WebsocketClientMap[client_id] = obj
	return obj
end

-- Connect to the websocket server
function WebsocketClient:connect()
	_G["_WEBSOCKET_NVIM"].callbacks[self.client_id] = {
		on_message = function(args)
			local client_id = args[1]
			local client = WebsocketClientMap[client_id]
			if not client then
				error("Received message but client not found", client_id)
			end
			local message = args[2]

			client.on_message(message)
		end,
		on_disconnect = function(client_id)
			local client = WebsocketClientMap[client_id]
			if not client then
				error("Disconnected but client not found", client_id)
			end

      WebsocketClientMap[client_id] = nil

			if client.on_disconnect then
				client.on_disconnect()
			end
		end,
		on_connect = function(client_id)
			local client = WebsocketClientMap[client_id]
			if not client then
				error("Connected but client not found", client_id)
			end

			if client.on_connect then
				client.on_connect()
			end
		end,
	}
	websocket_ffi.connect(self.client_id, self.connect_addr, self.extra_headers)
end

-- Check if the websocket client is active
--
---@return boolean
function WebsocketClient:is_active()
	return websocket_ffi.is_active(self.client_id)
end

-- Send data to the websocket server
--
---@param data string
function WebsocketClient:send_data(data)
	websocket_ffi.send_data(self.client_id, data)
end

-- Disconnect from the websocket server
function WebsocketClient:disconnect()
	websocket_ffi.disconnect(self.client_id)
end

return M
