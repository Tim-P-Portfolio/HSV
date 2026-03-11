#![no_main]
#![no_std]

use cortex_m_rt::entry;
use critical_section_lock_mut::LockMut;
use embedded_hal::{
    delay::DelayNs,
    digital::{InputPin, OutputPin},
};
use hsv::*;
use microbit::{
    board::Board,
    hal::timer::Timer,
    hal::{gpio, gpio::Level, gpio::Pin},
    pac,
    pac::interrupt,
    pac::TIMER0,
};

use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

const FRAME_TICKS: u32 = 100;
const TICK_US: u32 = 100;

const H: [[u8; 5]; 5] = [
    [0, 1, 0, 1, 0],
    [0, 1, 0, 1, 0],
    [0, 1, 1, 1, 0],
    [0, 1, 0, 1, 0],
    [0, 1, 0, 1, 0],
];
const S: [[u8; 5]; 5] = [
    [0, 1, 1, 1, 0],
    [0, 1, 0, 0, 0],
    [0, 1, 1, 1, 0],
    [0, 0, 0, 1, 0],
    [0, 1, 1, 1, 0],
];
const V: [[u8; 5]; 5] = [
    [0, 1, 0, 1, 0],
    [0, 1, 0, 1, 0],
    [0, 1, 0, 1, 0],
    [0, 1, 0, 1, 0],
    [0, 0, 1, 0, 0],
];

#[derive(Clone, Copy, Debug)]
enum Component {
    H,
    S,
    V,
}
impl Component {
    fn prev(self) -> Self {
        match self {
            Self::H => Self::V,
            Self::V => Self::S,
            Self::S => Self::H,
        }
    }
    fn next(self) -> Self {
        match self {
            Self::H => Self::S,
            Self::S => Self::V,
            Self::V => Self::H,
        }
    }
}

fn clamp_input(input: i16) -> f32 {
    const LO: i16 = 200;
    const HI: i16 = 16200;
    (input.clamp(LO, HI) - LO) as f32 / (HI - LO) as f32
}


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
        self.step();
    }

    fn stop(&mut self) {
        self.timer.disable_interrupt();
    }

    fn set(&mut self, hsv: &Hsv) {
        let rgb = hsv.to_rgb();
        // Schedule with rgb value
        self.next_schedule = Some([
            (rgb.r * FRAME_TICKS as f32) as u32,
            (rgb.g * FRAME_TICKS as f32) as u32,
            (rgb.b * FRAME_TICKS as f32) as u32,
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

        let next = self.schedule.iter()
            .filter(|&&t| t > self.tick)
            .copied()
            .min()
            .unwrap_or(FRAME_TICKS);
        let delay = (next - self.tick) * TICK_US;
        self.tick = if next >= FRAME_TICKS { 0 } else { next };
        self.timer.reset_event();
        self.timer.start(delay);
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

    let mut button_a = board.buttons.button_a.into_pullup_input();
    let mut button_b = board.buttons.button_b.into_pullup_input();

    let mut component = Component::H;
    let mut hsv = Hsv { h: 0.0, s: 1.0, v: 0.5 };
    let mut a_state = false;
    let mut b_state = false;
    
    loop {
        let a_pressed = button_a.is_low().unwrap();
        let b_pressed = button_b.is_low().unwrap();

        if a_pressed && !a_state { component = component.prev(); rprintln!("PREV"); }
        if b_pressed && !b_state { component = component.next(); rprintln!("NEXT"); }
        a_state = a_pressed;
        b_state = b_pressed;


        let potentiometer = reader.read_channel(&mut p2).unwrap();
        let input = clamp_input(potentiometer);
        match component {
            Component::H => hsv.h = input,
            Component::S => hsv.s = input,
            Component::V => hsv.v = input,
        }

        DISPLAY.with_lock(|d| {
            d.set(&hsv)
        });
        
        let letter = match component {
            Component::H => H,
            Component::S => S,
            Component::V => V,
        };
        rprintln!("potentiometer: {}, input: {}, letter: {:?}", potentiometer, input, component);
        display.show(&mut timer1, letter, 100);

    }
}
