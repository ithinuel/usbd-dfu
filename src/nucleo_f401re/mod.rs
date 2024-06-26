//! This DFU bootloader demo follows the following map :
//! | Sector |    Start    |     End     | Size (in KiB) | use
//! |--------|-------------|-------------|---------------|----
//! |      0 | 0x0800_0000 | 0x0800_3FFF |            16 | Bootloader
//! |      1 | 0x0800_4000 | 0x0800_7FFF |            16 |
//! |      2 | 0x0800_8000 | 0x0800_BFFF |            16 | Application
//! |      3 | 0x0800_C000 | 0x0800_FFFF |            16 |
//! |      4 | 0x0801_0000 | 0x0801_FFFF |            64 |
//! |      5 | 0x0802_0000 | 0x0803_FFFF |           128 |
//! |      6 | 0x0804_0000 | 0x0805_FFFF |           128 | ...
//! |      7 | 0x0806_0000 | 0x0807_FFFF |           128 | Application + manifest
//! |      _ | 0x1FFF_7800 | 0x1FFF_7A0F |         0.528 | OTP Area
//! |      _ | 0x1FFF_C000 | 0x1FFF_C00F |         0.016 | Option bytes

use core::mem::MaybeUninit;
use cortex_m::peripheral::syst::SystClkSource;
use stm32f4xx_hal::otg_fs::{UsbBus, USB};
use stm32f4xx_hal::prelude::*;

#[cfg(feature = "bootloader")]
use dfu::mode::DFUModeImpl as DFUImpl;
#[cfg(feature = "application")]
use dfu::runtime::DFURuntimeImpl as DFUImpl;
use dfu::Manifest;

const FLASH_END: usize = 0x0808_0000;
const MANIFEST_SIZE_ALIGNED: usize = ((core::mem::size_of::<Manifest>() + 127) / 128) * 128;
const MANIFEST_REGION_START: usize = FLASH_END - MANIFEST_SIZE_ALIGNED;

static mut EP_MEMORY: MaybeUninit<[u32; 256]> = MaybeUninit::uninit();

#[cfg(feature = "debug-buffer")]
static mut DEBUG_BUFFER: MaybeUninit<[u8; 1024]> = MaybeUninit::uninit();
#[cfg(any(feature = "debug-uart", feature = "debug-buffer"))]
pub static mut WRITER: Option<debug::DbgWriter> = None;

pub fn reset() -> ! {
    stm32f4xx_hal::stm32::SCB::sys_reset()
}

pub fn init() -> (
    usb_device::bus::UsbBusAllocator<impl usb_device::class_prelude::UsbBus>,
    stm32f4xx_hal::gpio::gpioa::PA5<stm32f4xx_hal::gpio::Output<stm32f4xx_hal::gpio::PushPull>>,
    cortex_m::Peripherals,
    DFUImpl,
) {
    let dp = stm32f4xx_hal::stm32::Peripherals::take().unwrap_or_else(|| unreachable!());
    let mut cp = cortex_m::Peripherals::take().unwrap_or_else(|| unreachable!());

    let rcc = dp.RCC.constrain();

    dp.FLASH.acr.modify(|_, w| {
        w.latency()
            .ws3()
            .icen()
            .set_bit()
            .dcen()
            .set_bit()
            .prften()
            .set_bit()
    });
    let clocks = rcc.cfgr.sysclk(84.mhz()).require_pll48clk().freeze();

    cp.SYST.set_clock_source(SystClkSource::External);
    cp.SYST.set_reload(clocks.sysclk().0 / (8 * 1_000));
    cp.SYST.clear_current();
    cp.SYST.enable_counter();

    let gpioa = dp.GPIOA.split();
    let led = gpioa.pa5.into_push_pull_output();

    let boot_mode = dp.GPIOC.split().pc13.into_floating_input();

    let usb = USB {
        usb_global: dp.OTG_FS_GLOBAL,
        usb_device: dp.OTG_FS_DEVICE,
        usb_pwrclk: dp.OTG_FS_PWRCLK,
        pin_dm: gpioa.pa11.into_alternate_af10(),
        pin_dp: gpioa.pa12.into_alternate_af10(),
        hclk: clocks.hclk(),
    };

    #[cfg(feature = "debug-buffer")]
    cortex_m::interrupt::free(move |_| unsafe {
        WRITER = Some(debug::DbgWriter::using_buffer(
            DEBUG_BUFFER.assume_init_mut(),
        ));
    });
    #[cfg(feature = "debug-uart")]
    {
        let pa2 = gpioa.pa2.into_alternate_af7();
        let usart2 = dp.USART2;
        cortex_m::interrupt::free(move |_| unsafe {
            let config = stm32f4xx_hal::serial::config::Config::default().baudrate(115200.bps());
            let serial = stm32f4xx_hal::serial::Serial::new(
                usart2,
                (pa2, stm32f4xx_hal::serial::NoRx),
                config,
                clocks,
            )
            .unwrap_or_else(|_| unreachable!());

            WRITER = Some(serial);
        });
    }

    #[cfg(feature = "application")]
    let dfu = DFUImpl;
    #[cfg(feature = "bootloader")]
    let dfu = DFUImpl::new(bootloader::Memory::new(dp.FLASH), boot_mode);

    (
        UsbBus::new(usb, unsafe { EP_MEMORY.assume_init_mut() }),
        led,
        cp,
        dfu,
    )
}

pub async fn trigger<T>(_: &mut T) {}
#[cfg(feature = "debug-uart")]
mod debug {
    use stm32f4xx_hal::gpio::{gpioa::PA2, Alternate, AF7};
    use stm32f4xx_hal::pac;
    use stm32f4xx_hal::serial;

    pub type DbgWriter = serial::Serial<pac::USART2, (PA2<Alternate<AF7>>, serial::NoRx)>;
}

#[cfg(feature = "debug-buffer")]
pub mod debug {
    pub struct DbgWriter {
        buffer: &'static mut [u8],
        len: usize,
    }
    impl DbgWriter {
        pub fn using_buffer(buffer: &'static mut [u8]) -> Self {
            Self { buffer, len: 0 }
        }
    }

    impl ::core::fmt::Write for DbgWriter {
        fn write_str(&mut self, s: &str) -> ::core::fmt::Result {
            if self.len > self.buffer.len() {
                // invalid state, that sucks
                self.len = 0;
            }
            let len = core::cmp::min(self.buffer.len() - self.len, s.len());
            let from = self.len;
            self.len += len;
            self.buffer[from..self.len].copy_from_slice(&s.as_bytes()[..len]);
            Ok(())
        }
    }
    pub fn consume_debug(mut reader: impl FnMut(&[u8]) -> usize) {
        cortex_m::interrupt::free(|_| unsafe {
            if let Some(w) = super::WRITER.as_mut() {
                let len = w.len;
                w.len -= reader(&w.buffer[..len]);
                w.buffer.copy_within(w.len..len, 0);
            };
        });
    }
}

#[macro_export]
#[cfg(any(feature = "debug-uart", feature = "debug-buffer"))]
macro_rules! dbgprint {
    ($($arg:tt)*) => {
        {
            use cortex_m::interrupt::free as interrupt_free;
            #[allow(unused_unsafe)]
            interrupt_free(|_| unsafe {
                use ::core::fmt::Write;
                use $crate::nucleo_f401re::WRITER;
                WRITER.as_mut().map(|w| w.write_fmt(format_args!($($arg)*)));
            });
        }
    };
}

#[macro_export]
#[cfg(not(any(feature = "debug-uart", feature = "debug-buffer")))]
macro_rules! dbgprint {
    ($($arg:tt)*) => {};
}

mod dfu;

#[cfg(feature = "bootloader")]
mod bootloader;
#[cfg(feature = "bootloader")]
pub use bootloader::jump_to_application;
