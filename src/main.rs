#![no_main]
#![no_std]

use cortex_m_rt::entry;
use critical_section_lock_mut::LockMut;
use embedded_hal::{delay::DelayNs, digital::OutputPin};
use hsv::*;
use microbit::{
    board::Board,
    hal::timer::Timer,
    hal::{gpio, gpio::Level, gpio::Pin},
    pac, pac::interrupt,
    pac::TIMER0,
};
use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};


const FRAME_TICKS: u32 = 256;
const TICK_US: u32 = 50;

// Source used to help with implementation: https://docs.rust-embedded.org/discovery-mb2/15-interrupts/my-solution.html
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

    fn start(&mut self) {
        self.tick = 0;
        self.timer.enable_interrupt();
        self.timer.start(TICK_US);
    }

    fn stop(&mut self) {
        self.timer.disable_interrupt();
    }

    fn set(&mut self, hsv: &Hsv) {
        let rgb = hsv.to_rgb();
        // Schedule with rgb value
        self.next_schedule = Some([
            (rgb.r * 255.0) as u32,
            (rgb.g * 255.0) as u32,
            (rgb.b * 255.0) as u32,
        ]);
    }

    fn step(&mut self) {
        if self.tick == 0 {
            if let Some(s) = self.next_schedule.take() {
                self.schedule = s;
            }
            for pin in &mut self.rgb_pins {
                let _ = pin.set_high();
            }
        }
        for (i, pin) in self.rgb_pins.iter_mut().enumerate() {
            if self.tick >= self.schedule[i] {
                let _ = pin.set_low();
            }
        }
        self.tick = (self.tick + 1) % FRAME_TICKS;
        self.timer.reset_event();
        self.timer.start(TICK_US);
    }
}

static DISPLAY: LockMut<RgbDisplay> = LockMut::new();

#[interrupt]
fn TIMER0() {
    DISPLAY.with_lock(|d| d.step());
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

    let mut display = microbit::display::blocking::Display::new(board.display_pins);

    let mut led = RgbDisplay::new([r_pin, g_pin, b_pin], timer0);
    led.set(&Hsv {
        h: 0.0,
        s: 1.0,
        v: 0.5,
    });

    DISPLAY.init(led);
    DISPLAY.with_lock(|d| d.start());

    unsafe { pac::NVIC::unmask(pac::Interrupt::TIMER0) };
    pac::NVIC::unpend(pac::Interrupt::TIMER0);

    loop {
        let button_a_pressed = button_a.is_low().unwrap();
        let button_b_pressed = button_b.is_low().unwrap();

        let potentiometer = reader.read_channel(&mut p2).unwrap();
        let hue = (potentiometer as f32 / 16384.0) as f32;

        DISPLAY.with_lock(|d| {
            d.set(&Hsv {
                h: hue,
                s: 0.5,
                v: 0.5,
            })
        });

        rprintln!("potentiometer: {}, hue: {}", potentiometer, hue);
        timer1.delay_ms(20);
    }
}
