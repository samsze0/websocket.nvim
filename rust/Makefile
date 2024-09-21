build:
	cargo build
	mkdir -p lua
	mv ./target/debug/libwebsocket_ffi.dylib ./lua/websocket_ffi.so

log:
	tail -f /tmp/websocket-nvim.log