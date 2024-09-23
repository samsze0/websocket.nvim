# websocket.nvim

## Installation

[lazy.nvim](https://github.com/folke/lazy.nvim)

```lua
{
    "samsze0/websocket.nvim",
    dependencies = {
        "samsze0/utils.nvim"
    }
}
```

Client:
```lua
local WebsocketClient = require("websocket.client").WebsocketClient

local client = WebsocketClient.new({
    connect_addr = "ws://localhost:8080",
    on_message = function(self, message)
        print("Received message: " .. message)
    end,
    on_connect = function(self)
        print("Connected")
    end,
    on_disconnect = function(self)
        print("Disconnected")
    end
})

client:try_connect()

-- Schedule to run in 2 seconds
vim.defer_fn(function()
  client:try_send_data("Hello server")
end, 2000)

-- Schedule to run in 5 seconds
vim.defer_fn(function()
  client:try_disconnect()
end, 5000)
```

Server:
```lua
local WebsocketServer = require("websocket.server").WebsocketServer

local server = WebsocketServer.new({
    host = "localhost",
    port = 12001,
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

server:try_start()
```

## How it works

Because currently there is no decent Lua/libuv implementations of web servers, so I decided to base this plugin on the [Tungstenite](https://github.com/snapview/tungstenite-rs) crate for the majority of the heavy lifting (i.e. the handling of the websocket protocol). The communication between Neovim/Lua and Rust is facilitated by Rust's FFI ([nvim-oxi](https://github.com/noib3/nvim-oxi) in combination with [mlua](https://github.com/mlua-rs/mlua)). Here is an outline of how it works:

1. Neovim/Lua imports the Rust functions (compiled into a dynamic lib) via `require`
2. Neovim/Lua calls the Rust functions to start the websocket server
3. Neovim/Lua stores the **callbacks** (e.g. `on_message`, `on_connected`, `on_disconnected`) as global objects, waiting for the Rust code to pull from using mlua

## License

MIT