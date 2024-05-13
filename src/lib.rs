use std::{
    collections::VecDeque,
    io::{self, Write},
    process::exit,
    sync::{mpsc, Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use video_rs::Decoder;

mod globals {
    pub const ROOT_DIR: &str = "/home/klaudiusz/Desktop/Projects/rust/terminal_player/";
    pub const SAMPLE_DIR: &str = "samples/";
    pub const DEF_WIDTH: usize = 72;
    pub const FRAME_BACKLOG: usize = 30 * 10;
    pub fn get_sample_mp4() -> String {
        format!("{}{}sample.mp4", ROOT_DIR, SAMPLE_DIR)
    }
}
#[derive(Debug, Clone)]
pub struct Config {
    pub file_name: String,
    pub video_size: (usize, usize),
    pub width_chars: usize,
    pub sampling_rate: (usize, usize),
    pub aspect_ratio: f32,
    pub frame_rate: u64,
    pub frame_size: usize,
    pub delta_t_ms: Duration,
}

impl Config {
    pub fn from_args(args: &[String]) -> Result<Config, String> {
        //TODO: Look for -w flag for width
        let mut width = globals::DEF_WIDTH;
        let mut file_name = globals::get_sample_mp4();
        for (i, arg) in args.iter().enumerate() {
            match arg {
                arg if arg.starts_with('-') => match arg {
                    arg if arg.starts_with("--width") || arg.starts_with("-w") => {
                        width = args[i + 1].parse().unwrap_or_else(|_| {
                            panic!("Invalid value {} for flag \"width\".", args[i + 1])
                        });
                    }
                    _ => {
                        eprint!("Unknown flag: {}", arg);
                        exit(1)
                    }
                },
                arg => file_name.clone_from(arg),
            }
        }
        if args.len() > 1 {
            Ok(Config {
                file_name,
                video_size: (0, 0),
                width_chars: width,
                sampling_rate: (0, 0),
                aspect_ratio: 0.0,
                frame_rate: 30,
                frame_size: 0,
                delta_t_ms: Duration::from_millis(0),
            })
        } else {
            Err(String::from("Provide a path to the file."))
        }
    }

    pub fn add_decoder_info(&mut self, decoder: &video_rs::Decoder) {
        self.aspect_ratio = decoder.size().0 as f32 / decoder.size().1 as f32;
        self.video_size = (decoder.size().0 as usize, decoder.size().1 as usize);
        let sample_x = self.video_size.0 / self.width_chars;
        self.sampling_rate = (sample_x, (sample_x as f32 / self.aspect_ratio) as usize);
        self.frame_rate = decoder.frame_rate() as u64;
        self.frame_size = ((self.width_chars ^ 2) as f32 * self.aspect_ratio) as usize;
        self.delta_t_ms = Duration::from_millis((1000.0 / decoder.frame_rate()) as u64);
    }
}
#[derive(PartialEq, Debug, Copy, Clone)]
enum ControlSignal {
    Stop,
    Go,
}

//Player
const NULL_FRAME: &str = "\0";
struct Player {
    queue: VecDeque<String>,
    queue_size: usize,
    is_playing: bool,
    config: Config,
    decoder: Arc<Mutex<Decoder>>,
    tx_data: mpsc::Sender<String>,
    rx_data: mpsc::Receiver<String>,
}
impl Player {
    pub fn new(cfg: Config, decoder: Decoder) -> Player {
        let queue = VecDeque::with_capacity(globals::FRAME_BACKLOG);

        let (tx_data, rx_data) = mpsc::channel();

        Player {
            queue,
            queue_size: globals::FRAME_BACKLOG,
            is_playing: false,
            config: cfg,
            decoder: Arc::new(Mutex::new(decoder)),
            tx_data,
            rx_data,
        }
    }

    pub fn play(&mut self) {
        self.is_playing = true;

        let mut prev = Instant::now();

        let con_mut = Arc::new((Condvar::new(), Mutex::new(ControlSignal::Go)));
        self.spawn_frame_parser(Arc::clone(&con_mut));
        let (condvar, mtx) = &*con_mut;

        let mut stream_exhausted = false;
        let mut reached_max_capacity = false; // Flag to track when queue reaches max capacity

        loop {
            let capacity = self.queue_size - self.queue.len();

            let action = match capacity {
                c if c <= 10 => ControlSignal::Stop,
                _ if reached_max_capacity && capacity >= self.queue_size / 2 => {
                    reached_max_capacity = false; // Reset flag when queue drops to 50%
                    ControlSignal::Go
                }
                _ => ControlSignal::Go,
            };

            let mut signal = mtx.lock().unwrap();
            if action == ControlSignal::Stop || *signal == ControlSignal::Stop {
                *signal = action;
                condvar.notify_one();
            }

            drop(signal);

            if !stream_exhausted && action != ControlSignal::Stop {
                match self.rx_data.recv() {
                    Ok(frame) if &frame == NULL_FRAME => {
                        stream_exhausted = true;
                    }
                    Ok(frame) => {
                        self.queue.push_front(frame);
                    }
                    Err(_) => {
                        stream_exhausted = true;
                    }
                }
            }

            if self.should_skip_rendering(prev) {
                continue;
            }

            let frame = match self.queue.pop_back() {
                None => {
                    if stream_exhausted {
                        break;
                    } else {
                        continue; // wait for the frame
                    }
                }
                Some(f) => f,
            };

            self.render_frame(&frame);
            prev = Instant::now();

            // Set flag when queue reaches maximum capacity
            if !reached_max_capacity && self.queue.len() == self.queue_size {
                reached_max_capacity = true;
            }
        }
    }

    fn spawn_frame_parser(&self, condvar: Arc<(Condvar, Mutex<ControlSignal>)>) {
        let cfg = self.config.clone();
        let decoder = Arc::clone(&self.decoder);
        let tx = self.tx_data.clone();
        std::thread::spawn(move || {
            let mut decoder: std::sync::MutexGuard<Decoder> = decoder.lock().unwrap();
            for frame in decoder.decode_raw_iter() {
                let frame = match frame {
                    Err(video_rs::Error::ReadExhausted) => {
                        println!("Stream exhausted");
                        tx.send(String::from(NULL_FRAME)).unwrap();
                        break; //stream exhausted, thread done
                    }
                    Ok(v) => v,
                    Err(e) => {
                        eprint!("{}", e);
                        exit(2);
                    }
                };

                let (condvar, mutex) = &*condvar;
                let mut signal = mutex.lock().unwrap();
                while *signal == ControlSignal::Stop {
                    signal = condvar.wait(signal).unwrap();
                }
                let frame_str = ascii::rgb_to_ascii(frame.data(0), &cfg);
                tx.send(frame_str).unwrap();
            }
        });
    }

    fn render_frame(&self, chars: &str) {
        ascii::clear_screen();
        io::stdout().flush().expect("Failed to flush stdout");
        print!("{}", chars);
    }

    fn should_skip_rendering(&self, prev: Instant) -> bool {
        let elapsed = prev.elapsed();
        return elapsed < self.config.delta_t_ms;
    }
}
//ASCII
pub mod ascii {
    use super::*;

    const CHAR_MAP: &str = " ,\":;Il!i~+_-?][}{1)(|\\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$";

    pub fn rgb_to_ascii(rgb: &[u8], cfg: &Config) -> String {
        let mut frame_str = String::with_capacity(cfg.frame_size);
        for row in rgb
            .chunks(cfg.video_size.0 * 3)
            .step_by(cfg.sampling_rate.1 * 3)
        {
            for pixel in row.chunks(3).step_by(cfg.sampling_rate.0) {
                let ascii_char = rgb_to_ascii_char(pixel);
                frame_str.push(ascii_char);
            }
            frame_str.push('\n');
        }
        frame_str
    }
    pub fn rgb_to_ascii_buff(rgb: &[u8], cfg: &Config, buff: &mut String) {
        for row in rgb
            .chunks(cfg.video_size.0 * 3)
            .step_by(cfg.sampling_rate.1 * 3)
        {
            for pixel in row.chunks(3).step_by(cfg.sampling_rate.0) {
                let ascii_char = rgb_to_ascii_char(pixel);
                buff.push(ascii_char);
            }
            buff.push('\n');
        }
    }
    fn rgb_to_ascii_char(pixel: &[u8]) -> char {
        let y = 0.21 * pixel[0] as f32 + 0.72 * pixel[1] as f32 + 0.07 * pixel[2] as f32;
        let lum = y * 0.001307 * CHAR_MAP.len() as f32;
        let index = lum as usize;
        CHAR_MAP.chars().nth(index).unwrap_or(' ')
    }

    pub fn clear_screen() {
        print!("\x1B[2J\x1B[1;1H"); // Clear screen and move cursor to top-left corner
        std::io::stdout().flush().expect("Failed to flush stdout");
    }
}

pub fn run(decoder: Decoder, cfg: Config) {
    let mut player: Player = Player::new(cfg, decoder);
    player.play();
}
