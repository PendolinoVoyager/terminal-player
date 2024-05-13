use std::path::Path;
use terminal_player::Config;
use video_rs::{Decoder, Location};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut config = Config::from_args(&args).unwrap_or_else(|e| {
        println!("Cannot create config: {}", e);
        std::process::exit(1);
    });

    init_ffmpeg();

    let decoder = create_decoder(&config.file_name).unwrap_or_else(|e| {
        println!("Cannot create decoder from {}.\n{}", config.file_name, e);
        std::process::exit(1);
    });
    config.add_decoder_info(&decoder);

    terminal_player::run(decoder, config);
}

pub fn init_ffmpeg() {
    video_rs::init().unwrap();
}
pub fn create_decoder(file_name: &str) -> Result<Decoder, video_rs::Error> {
    let path = Path::new(file_name);
    let source = Location::File(path.to_path_buf());
    Decoder::new(source)
}
