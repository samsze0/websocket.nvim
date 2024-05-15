# websocket.nvim

WIP

Websocket implementation for Neovim

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

## How it works

Because currently there is no decent Lua/libuv implementations of web servers, so I decided to base this plugin on the [Tungstenite](https://github.com/snapview/tungstenite-rs) crate for the majority of the heavy lifting (i.e. the handling of the websocket protocol). The communication between Neovim/Lua and Rust is facilitated by Rust's FFI ([nvim-oxi](https://github.com/noib3/nvim-oxi) in combination with [mlua](https://github.com/mlua-rs/mlua)). Here is an outline of how it works:

1. Neovim/Lua imports the Rust functions (compiled into a dynamic lib) via `require`
2. Neovim/Lua calls the Rust functions to start the websocket server
3. Neovim/Lua stores the **callbacks** (e.g. `on_message`, `on_connected`, `on_disconnected`) as global objects, waiting for the Rust code to pull from using mlua

## License

MIT