#![no_main]
#![no_std]

use cortex_m_rt::entry;
use critical_section_lock_mut::LockMut;
use embedded_hal::delay::DelayNs;
use hsv::*;
use microbit::{
    board::Board,
    hal::timer::Timer,
    hal::{gpio, gpio::Level, gpio::Pin},
    pac,
    pac::TIMER0,
};
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

struct RgbDisplay {
    tick: u32,
    schedule: [u32; 3],
    next_schedule: Option<[u32; 3]>,
    rgb_pins: [Pin<gpio::Output<gpio::PushPull>>; 3],
    timer: Timer<TIMER0>,
}

impl RgbDisplay {
    fn new<T>(pins: [gpio::Pin<T>; 3], timer: Timer<pac::TIMER0>) -> Self {
        let rgb_pins = pins.map(|p| p.into_push_pull_output(Level::Low));
        Self {
            tick: 0,
            schedule: [0; 3],
            next_schedule: None,
            rgb_pins,
            timer,
        }
    }

    fn set(&mut self, hsv: &Hsv) {}
    fn step(&mut self) {}
}

#[entry]
fn main() -> ! {
    rtt_init_print!();
    let board = Board::take().unwrap();

    let r_pin = board.edge.e16.degrade();
    let g_pin = board.edge.e09.degrade();
    let b_pin = board.edge.e08.degrade();
    let timer0 = Timer::new(board.TIMER0);
    let mut timer1 = Timer::new(board.TIMER1);

    let mut p2 = board.edge.e02.into_floating_input();
    let mut reader =
        microbit::hal::Saadc::new(board.ADC, microbit::hal::saadc::SaadcConfig::default());

    loop {
        let potentiometer = reader.read_channel(&mut p2).unwrap();
        let hue = (potentiometer as f32 / 16384.0) as f32;

        rprintln!("potentiometer: {}, hue: {}", potentiometer, hue);
        timer1.delay_ms(20);
    }
}
