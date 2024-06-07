local current_path = debug.getinfo(1).source:match("@?(.*/)")
vim.opt.runtimepath:append(current_path .. "../rust")

local uuid_utils = require("utils.uuid")

local PORT = 12010
local FZF_API_KEY = "test"

local TIMEOUT = 2000

_G["_WEBSOCKET_NVIM"] = {
  clients = {
    callbacks = {},
  },
}

local current_path = debug.getinfo(1).source:match("@?(.*/)")
vim.opt.runtimepath:append(current_path .. "../../rust")
local websocket_client_ffi = require("websocket_ffi").client

T.debug(websocket_client_ffi)

local last_message = nil
local disconnected = false
local connected = false
local last_error = nil

local client_id = uuid_utils.v4()
_G["_WEBSOCKET_NVIM"].clients.callbacks[client_id] = {
  on_message = function(client_id, message)
    print("Callback: Received message", message)
    last_message = message
  end,
  on_disconnect = function(client_id)
    print("Callback: Disconnected from", client_id)
    disconnected = true
  end,
  on_connect = function(client_id)
    print("Callback: Connected to", client_id)
    connected = true
  end,
  on_error = function(client_id, error)
    print("Callback: Error", vim.inspect(error))
    last_error = error
  end,
}

websocket_client_ffi.connect(
  client_id,
  string.format("ws://localhost:%d", PORT),
  {
    ["Fzf-Api-Key"] = FZF_API_KEY,
  }
)

vim.cmd("sleep " .. TIMEOUT / 1000)

T.assert(connected)

local is_active = websocket_client_ffi.is_active(client_id)
T.assert(is_active)

websocket_client_ffi.send_data(
  client_id,
  "pos(3)+websocket-broadcast@Hi from server@"
)

vim.cmd("sleep " .. TIMEOUT / 1000)

T.assert_eq(last_message, "Hi from server")
T.assert_not(last_error)

websocket_client_ffi.disconnect(client_id)

vim.cmd("sleep " .. TIMEOUT / 1000)

T.assert(disconnected)

local is_active = websocket_client_ffi.is_active(client_id)
T.assert(not is_active)