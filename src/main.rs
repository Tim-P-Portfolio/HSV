#![no_main]
#![no_std]

use cortex_m_rt::entry;
use critical_section_lock_mut::LockMut;
use embedded_hal::digital::OutputPin;
use hsv::*;
use microbit::{
    board::Board,
    display::nonblocking::{Display, GreyscaleImage},
    hal::{
        gpio::{self, Level, Pin},
        gpiote::Gpiote,
        timer::Timer,
    },
    pac::{self, interrupt, TIMER0, TIMER1, TIMER2},
};

use panic_rtt_target as _;
use rtt_target::{rprintln, rtt_init_print};

const FRAME_TICKS: u32 = 100;
const TICK_US: u32 = 100;

// Greyscale bitmaps for MB2 display
const H: GreyscaleImage = GreyscaleImage::new(&[
    [0, 9, 0, 9, 0],
    [0, 9, 0, 9, 0],
    [0, 9, 9, 9, 0],
    [0, 9, 0, 9, 0],
    [0, 9, 0, 9, 0],
]);
const S: GreyscaleImage = GreyscaleImage::new(&[
    [0, 9, 9, 9, 0],
    [0, 9, 0, 0, 0],
    [0, 9, 9, 9, 0],
    [0, 0, 0, 9, 0],
    [0, 9, 9, 9, 0],
]);
const V: GreyscaleImage = GreyscaleImage::new(&[
    [0, 9, 0, 9, 0],
    [0, 9, 0, 9, 0],
    [0, 9, 0, 9, 0],
    [0, 9, 0, 9, 0],
    [0, 0, 9, 0, 0],
]);

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

// Source used to help with implementation:
//  https://docs.rust-embedded.org/discovery-mb2/15-interrupts/my-solution.html
//  https://github.com/pdx-cs-rust-embedded/mb2-grayscale
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

        let next = self
            .schedule
            .iter()
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

static RGB_DISPLAY: LockMut<RgbDisplay> = LockMut::new();
static DISPLAY: LockMut<Display<TIMER1>> = LockMut::new();
static P_GPIOTE: LockMut<Gpiote> = LockMut::new();
static COMPONENT: LockMut<Component> = LockMut::new();
static DEBOUNCE_TIMER: LockMut<Timer<TIMER2>> = LockMut::new();

fn on_button_press(step: fn(Component) -> Component) {
    pac::NVIC::mask(pac::Interrupt::GPIOTE);
    COMPONENT.with_lock(|c| *c = step(*c));
    DEBOUNCE_TIMER.with_lock(|t| {
        t.reset_event();
        t.start(100_000u32); // 100 ms
    });
}
// gpiote for A/B button, interrupt
#[interrupt]
fn GPIOTE() {
    P_GPIOTE.with_lock(|gpiote| {
        if gpiote.channel0().is_event_triggered() {
            gpiote.channel0().reset_events();
            on_button_press(Component::prev);
        } else if gpiote.channel1().is_event_triggered() {
            gpiote.channel1().reset_events();
            on_button_press(Component::next);
        }
    });
}

// Debounce interrupt
#[interrupt]
fn TIMER2() {
    DEBOUNCE_TIMER.with_lock(|t| t.reset_event());
    unsafe { pac::NVIC::unmask(pac::Interrupt::GPIOTE) };
    pac::NVIC::unpend(pac::Interrupt::GPIOTE);
}

// RGB LED interrupt
#[interrupt]
fn TIMER0() {
    RGB_DISPLAY.with_lock(|d| d.step());
}

// MB2 led display interrupt
#[interrupt]
fn TIMER1() {
    DISPLAY.with_lock(|d| d.handle_display_event());
}

#[entry]
fn main() -> ! {
    rtt_init_print!();
    let board = Board::take().unwrap();

    let r_pin = board.edge.e16.degrade();
    let g_pin = board.edge.e09.degrade();
    let b_pin = board.edge.e08.degrade();
    let timer0 = Timer::new(board.TIMER0);

    let mut p2 = board.edge.e02.into_floating_input();
    let mut reader =
        microbit::hal::Saadc::new(board.ADC, microbit::hal::saadc::SaadcConfig::default());

    let display = Display::new(board.TIMER1, board.display_pins);
    DISPLAY.init(display);

    let mut led = RgbDisplay::new([r_pin, g_pin, b_pin], timer0);
    led.set(&Hsv {
        h: 0.0,
        s: 1.0,
        v: 0.5,
    });

    let button_a = board.buttons.button_a.into_pullup_input();
    let button_b = board.buttons.button_b.into_pullup_input();

    // Reference for gpiote implementation https://github.com/pdx-cs-rust-embedded/mb2-lsm-gpio/
    let gpiote = Gpiote::new(board.GPIOTE);
    {
        let channel0 = gpiote.channel0();
        channel0
            .input_pin(&button_a.degrade())
            .hi_to_lo()
            .enable_interrupt();
        channel0.reset_events();
    }
    {
        let channel1 = gpiote.channel1();
        channel1
            .input_pin(&button_b.degrade())
            .hi_to_lo()
            .enable_interrupt();
        channel1.reset_events();
    }

    P_GPIOTE.init(gpiote);

    RGB_DISPLAY.init(led);
    RGB_DISPLAY.with_lock(|d| d.start());

    unsafe { pac::NVIC::unmask(pac::Interrupt::TIMER0) };
    pac::NVIC::unpend(pac::Interrupt::TIMER0);

    unsafe { pac::NVIC::unmask(pac::Interrupt::TIMER1) };
    pac::NVIC::unpend(pac::Interrupt::TIMER1);

    unsafe { pac::NVIC::unmask(pac::Interrupt::GPIOTE) };
    pac::NVIC::unpend(pac::Interrupt::GPIOTE);

    let mut timer2 = Timer::new(board.TIMER2);
    timer2.enable_interrupt();
    DEBOUNCE_TIMER.init(timer2);

    unsafe { pac::NVIC::unmask(pac::Interrupt::TIMER2) };
    pac::NVIC::unpend(pac::Interrupt::TIMER2);

    // Initialize HSV values
    let mut hsv = Hsv {
        h: 0.0,
        s: 1.0,
        v: 0.5,
    };

    COMPONENT.init(Component::H);

    loop {
        // Read from ADC on edge pin 2
        let potentiometer = reader.read_channel(&mut p2).unwrap();
        let input = clamp_input(potentiometer);

        let mut component = Component::H;
        COMPONENT.with_lock(|c| component = *c);

        match component {
            Component::H => hsv.h = input,
            Component::S => hsv.s = input,
            Component::V => hsv.v = input,
        }

        RGB_DISPLAY.with_lock(|d| d.set(&hsv));

        let letter = match component {
            Component::H => H,
            Component::S => S,
            Component::V => V,
        };

        COMPONENT.with_lock(|c| match c {
            Component::H => hsv.h = input,
            Component::S => hsv.s = input,
            Component::V => hsv.v = input,
        });
        rprintln!(
            "letter: {:?}, potentiometer: {}, input: {}",
            component,
            potentiometer,
            input,
        );

        DISPLAY.with_lock(|d| d.show(&letter));
    }
}
