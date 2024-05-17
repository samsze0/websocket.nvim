local M = {}

function M.setup()
	vim.opt.runtimepath:append(vim.fn.expand("%:h") .. "../../rust")
	_G["_WEBSOCKET_NVIM"] = {
		callbacks = {},
	}
end

return M
