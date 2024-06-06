local M = {}

---@param opts? {}
function M.setup(opts)
  local current_path = debug.getinfo(1).source:match("@?(.*/)")
  vim.opt.runtimepath:append(current_path .. "../../rust")
  _G["_WEBSOCKET_NVIM"] = {
    clients = {
      callbacks = {},
    },
    servers = {
      callbacks = {},
    },
  }
end

return M
