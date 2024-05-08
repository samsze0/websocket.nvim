use nvim_oxi as nvim;

#[nvim::module]
fn websocket_nvim() -> nvim::Result<i32> {
    Ok(10)
}
