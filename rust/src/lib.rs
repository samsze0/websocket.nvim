use std::env;

use log::{self};
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use nvim_oxi::{Dictionary, Object};

mod client;

#[nvim_oxi::module]
fn websocket_ffi() -> nvim_oxi::Result<Dictionary> {
    env::set_var("RUST_BACKTRACE", "1");

    let file_appender = FileAppender::builder()
        // Pattern: https://docs.rs/log4rs/*/log4rs/encode/pattern/index.html
        .encoder(Box::new(PatternEncoder::new(
            "[{l}] {d(%Y-%m-%d %H:%M:%S)} {m}\n",
        )))
        .build("/tmp/websocket-nvim.log")
        .expect("Failed to create file appender");

    let log_config = Config::builder()
        .appender(Appender::builder().build("file", Box::new(file_appender)))
        .build(
            Root::builder()
                .appender("file")
                .build(log::LevelFilter::Debug),
        )
        .expect("Failed to create log config");
    let _ = log4rs::init_config(log_config).expect("Failed to initialize logger");

    log_panics::init();

    let api = Dictionary::from_iter([("client", Object::from(client::websocket_client_ffi()))]);

    Ok(api)
}
