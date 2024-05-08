build:
	cargo build
	mkdir -p lua
	mv ./target/debug/libwebsocket_nvim.dylib ./lua/websocket_nvim.so