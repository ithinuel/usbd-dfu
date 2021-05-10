use stm32l4xx_hal::usb::{Peripheral, UsbBus};
use stm32l4xx_hal::serial::{Serial, self};
use stm32l4xx_hal::prelude::*;

pub fn reset() -> ! {
    panic!("reset");
}

fn enable_crs() {
    let rcc = unsafe { &(*stm32::RCC::ptr()) };
    rcc.apb1enr1.modify(|_, w| w.crsen().set_bit());
    let crs = unsafe { &(*stm32::CRS::ptr()) };
    // Initialize clock recovery
    // Set autotrim enabled.
    crs.cr.modify(|_, w| w.autotrimen().set_bit());
    // Enable CR
    crs.cr.modify(|_, w| w.cen().set_bit());
}

/// Enables VddUSB power supply
fn enable_usb_pwr() {
    // Enable PWR peripheral
    let rcc = unsafe { &(*stm32::RCC::ptr()) };
    rcc.apb1enr1.modify(|_, w| w.pwren().set_bit());

    // Enable VddUSB
    let pwr = unsafe { &*stm32::PWR::ptr() };
    pwr.cr2.modify(|_, w| w.usv().set_bit());
}


pub fn init() -> (impl usb_device::class_prelude::UsbBus,(),(),()) {
    // Get access to the device specific peripherals from the peripheral access crate
    let p = Peripherals::take().unwrap_or_else(|| unreachable!());
    let mut cp = cortex_m::Peripherals::take().unwrap_or_else(|| unreachable!());

    // Take ownership over the raw flash and rcc devices and convert them
    // into the corresponding HAL structs
    let mut flash = p.FLASH.constrain();
    let mut rcc = p.RCC.constrain();
    let mut pwr = p.PWR.constrain(&mut rcc.apb1r1);

    // Freeze the configuration of all the clocks in the system and store
    // the frozen frequencies in `clocks`
    let clocks = rcc.cfgr.sysclk(80.mhz()).freeze(&mut flash.acr, &mut pwr);

    // Acquire the GPIOB peripheral
    let mut gpioa = p.GPIOB.split(&mut rcc.ahb2);

    let tx = gpioa.pb6.into_af7(&mut gpioa.moder, &mut gpioa.afrl);
    let rx = gpioa.pb7.into_af7(&mut gpioa.moder, &mut gpioa.afrl);


    let mut gpiob = dp.GPIOB.split(&mut rcc.ahb2);
    let mut led = gpiob
        .pb3
        .into_push_pull_output(&mut gpiob.moder, &mut gpiob.otyper);
    led.set_low(); // Turn off

    let mut gpioa = dp.GPIOA.split(&mut rcc.ahb2);
    let usb = Peripheral {
        usb: dp.USB,
        pin_dm: gpioa.pa11.into_af10(&mut gpioa.moder, &mut gpioa.afrh),
        pin_dp: gpioa.pa12.into_af10(&mut gpioa.moder, &mut gpioa.afrh),
    };
    let usb_bus = UsbBus::new(usb);

    cp.SYST.set_clock_source(SystClkSource::External);
    cp.SYST.set_reload(clocks.master_clock.0 / (8 * 1_000));
    cp.SYST.clear_current();
    cp.SYST.enable_counter();

        let (tx, rx) = Serial::usart1(
            p.USART1,
            (tx, rx),
            serial::Config::default().baudrate(115_200.bps()),
            clocks,
            &mut rcc.apb2,
        )
        .split();

        Self {
            sin: rx,
            sout: tx,
            name: "disco-l475-iot01a",
        }
    ((),(),(),())
}

pub async fn trigger(_ctx: ()) {

}
pub fn consume_debug(_f: impl FnMut(&[u8])->usize) {

}

#[macro_export]
macro_rules! dbgprint {
    ($($arg:tt)*) => {{}};
}

macro_rules! impl_capabilities {
    ($name:ty) => {
        impl usbd_dfu::Capabilities for $name {
            const CAN_UPLOAD: bool = true;
            const CAN_DOWNLOAD: bool = true;
            const IS_MANIFESTATION_TOLERANT: bool = true;
            const WILL_DETACH: bool = false;
            const DETACH_TIMEOUT: u16 = 5000;
            const TRANSFER_SIZE: u16 = 4096;
        }
    };
}

pub struct DFURuntimeImpl;
impl_capabilities!(DFURuntimeImpl);

