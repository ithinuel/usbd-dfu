//! This DFU bootloader demo follows the following map :
//! | Sector |    Start    |     End     | Size (in KiB) | use
//! |--------|-------------|-------------|---------------|----
//! |      0 | 0x0800_0000 | 0x0800_3FFF |            16 | Bootloader
//! |      1 | 0x0800_4000 | 0x0800_7FFF |            16 | Application
//! |      2 | 0x0800_8000 | 0x0800_BFFF |            16 | ...
//! |      3 | 0x0800_C000 | 0x0800_FFFF |            16 |
//! |      4 | 0x0801_0000 | 0x0801_FFFF |            64 |
//! |      5 | 0x0802_0000 | 0x0803_FFFF |           128 |
//! |      6 | 0x0804_0000 | 0x0805_FFFF |           128 | ...
//! |      7 | 0x0806_0000 | 0x0807_FFFF |           128 | Application + manifest
//! |      _ | 0x1FFF_7800 | 0x1FFF_7A0F |         0.528 | OTP Area
//! |      _ | 0x1FFF_C000 | 0x1FFF_C00F |         0.016 | Option bytes

use core::{mem::MaybeUninit, slice::from_raw_parts};
use cortex_m::peripheral::syst::SystClkSource;
use stm32f4xx_hal::otg_fs::{UsbBus, USB};
use stm32f4xx_hal::prelude::*;

#[cfg(any(feature = "debug-uart", feature = "debug-buffer"))]
use cortex_m::interrupt;

#[cfg(feature = "bootloader")]
type DFUImpl = DFUModeImpl;
#[cfg(feature = "application")]
type DFUImpl = DFURuntimeImpl;

const APPLICATION_REGION_START: usize = 0x0800_4000;
const APPLICATION_REGION_LENGTH: usize = (16 * 3 + 64 + 128 * 3) * 1024;
const APPLICATION_MANIFEST_START: usize = 0x0807_0000 - 128;

static mut EP_MEMORY: MaybeUninit<[u32; 256]> = MaybeUninit::uninit();

#[cfg(feature = "debug-buffer")]
static mut DEBUG_BUFFER: MaybeUninit<[u8; 1024]> = MaybeUninit::uninit();
#[cfg(any(feature = "debug-uart", feature = "debug-buffer"))]
pub static mut WRITER: Option<debug::DbgWriter> = None;

fn get_manifest() -> &'static Manifest {
    unsafe { &*(APPLICATION_MANIFEST_START as *const Manifest) }
}
fn get_app_array() -> &'static [u8] {
    unsafe {
        from_raw_parts(
            APPLICATION_REGION_START as *const u8,
            APPLICATION_REGION_LENGTH,
        )
    }
}

pub fn reset() -> ! {
    stm32f4xx_hal::stm32::SCB::sys_reset()
}

pub fn init() -> (
    usb_device::bus::UsbBusAllocator<impl usb_device::class_prelude::UsbBus>,
    stm32f4xx_hal::gpio::gpioa::PA10<stm32f4xx_hal::gpio::Output<stm32f4xx_hal::gpio::PushPull>>,
    cortex_m::Peripherals,
    DFUImpl,
) {
    let dp = stm32f4xx_hal::stm32::Peripherals::take().unwrap();
    let mut cp = cortex_m::Peripherals::take().unwrap();

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
    let led = gpioa.pa10.into_push_pull_output();

    let usb = USB {
        usb_global: dp.OTG_FS_GLOBAL,
        usb_device: dp.OTG_FS_DEVICE,
        usb_pwrclk: dp.OTG_FS_PWRCLK,
        pin_dm: gpioa.pa11.into_alternate_af10(),
        pin_dp: gpioa.pa12.into_alternate_af10(),
        hclk: clocks.hclk(),
    };

    #[cfg(feature = "debug-buffer")]
    interrupt::free(move |_| unsafe {
        WRITER = Some(DbgWriter::using_buffer(DEBUG_BUFFER.assume_init_mut()));
    });
    #[cfg(feature = "debug-uart")]
    {
        let pa2 = gpioa.pa2.into_alternate_af7();
        let usart2 = dp.USART2;
        interrupt::free(move |_| unsafe {
            let config = stm32f4xx_hal::serial::config::Config::default().baudrate(115200.bps());
            let serial = stm32f4xx_hal::serial::Serial::new(
                usart2,
                (pa2, stm32f4xx_hal::serial::NoRx),
                config,
                clocks,
            )
            .unwrap();

            WRITER = Some(serial);
        });
    }

    #[cfg(feature = "application")]
    let dfu = DFURuntimeImpl;
    #[cfg(feature = "bootloader")]
    let dfu = DFUModeImpl {
        upload_ptr: None,
        download_ptr: 0,
    };

    (
        UsbBus::new(usb, unsafe { EP_MEMORY.assume_init_mut() }),
        led,
        cp,
        dfu,
    )
}

#[cfg(feature = "bootloader")]
pub fn jump_to_application() -> ! {
    unsafe {
        let mut cp = cortex_m::Peripherals::steal();
        cp.SYST.disable_interrupt(); // it wasn't enabled but better safe than sorry
        cp.SYST.disable_counter();

        let dp = stm32f4xx_hal::stm32::Peripherals::steal();
        dp.RCC.ahb1rstr.write_with_zero(|w| w.gpioarst().set_bit());
        dp.RCC.ahb2rstr.write_with_zero(|w| w.otgfsrst().set_bit());
        #[cfg(feature = "debug-uart")]
        {
            dp.RCC.apb1rstr.write_with_zero(|w| w.uart2rst().set_bit());
        }
        //dp.RCC.constrain().cfgr.freeze();

        dp.FLASH.acr.modify(|_, w| {
            w.latency()
                .ws0()
                .icen()
                .clear_bit()
                .dcen()
                .clear_bit()
                .prften()
                .clear_bit()
        });
        cp.SCB.vtor.write(APPLICATION_REGION_START as u32);
        //cp.SCB.disable_dcache(&mut cp.CPUID);
        //cp.SCB.clean_invalidate_dcache(&mut cp.CPUID);
        //cp.SCB.disable_icache();
        //cp.SCB.invalidate_icache();

        cortex_m::asm::bootload(APPLICATION_REGION_START as *const u32);
    }
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
mod debug {
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
        interrupt::free(|_| unsafe {
            WRITER.as_mut().map(|w| {
                let len = w.len;
                w.len -= reader(&w.buffer[..len]);
                w.buffer.copy_within(w.len..len, 0);
            });
        });
    }
}

#[macro_export]
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

macro_rules! impl_capabilities {
    ($name:ty) => {
        impl ::usbd_dfu::Capabilities for $name {
            const CAN_UPLOAD: bool = true;
            const CAN_DOWNLOAD: bool = true;
            const IS_MANIFESTATION_TOLERANT: bool = true;
            const WILL_DETACH: bool = false;
            const DETACH_TIMEOUT: u16 = 5000;
            const TRANSFER_SIZE: u16 = 128;
        }
    };
}

pub struct DFURuntimeImpl;
impl DFURuntimeImpl {
    pub async fn read_manifest(&self) -> &'static Manifest {
        get_manifest()
    }
}
impl_capabilities!(DFURuntimeImpl);
impl usbd_dfu::runtime::DeviceFirmwareUpgrade for DFURuntimeImpl {
    fn on_reset(&mut self) {
        reset();
    }

    fn on_detach_request(&mut self, _timeout_ms: u16) {}
}

#[cfg(not(feature = "use-sha256"))]
const HASH_LENGTH: usize = 20;
#[cfg(feature = "use-sha256")]
const HASH_LENGTH: usize = 32;

#[repr(C)]
#[derive(Debug)]
pub struct Manifest {
    hash: [u8; HASH_LENGTH],
}

pub struct DFUModeImpl {
    upload_ptr: Option<&'static [u8]>,
    download_ptr: usize,
}

impl_capabilities!(DFUModeImpl);
impl usbd_dfu::mode::DeviceFirmwareUpgrade for DFUModeImpl {
    const POLL_TIMEOUT: u32 = 1;

    fn is_firmware_valid(&mut self) -> bool {
        let manifest = get_manifest();

        let app_region = get_app_array();
        let app_slice = &app_region[..app_region.len() - core::mem::size_of::<Manifest>()];
        //
        #[cfg(not(feature = "use-sha256"))]
        {
            let mut sha = sha1::Sha1::new();
            sha.update(app_slice);
            manifest.hash == sha.digest().bytes()
        }
        #[cfg(feature = "use-sha256")]
        {
            manifest.hash == hmac_sha256::Hash::hash(app_slice)
        }
    }
    fn is_transfer_complete(&mut self) -> bool {
        // has write operation completed?
        true
    }
    fn is_manifestation_in_progress(&mut self) -> bool {
        //dbgprint!("is_manifestation_in_progress\r\n");
        let manifestation = self.download_ptr == APPLICATION_REGION_LENGTH;
        self.download_ptr = 0;
        manifestation
    }

    fn upload(
        &mut self,
        _block_number: u16,
        buf: &mut [u8],
    ) -> core::result::Result<usize, usbd_dfu::Error> {
        //dbgprint!(
        //    "{:?} {} {}\r\n",
        //    self.upload_ptr.map(|slice| slice.len()),
        //    block_number,
        //    buf.len()
        //);

        let app_slice = self.upload_ptr.take().unwrap_or_else(get_app_array);

        let size = usize::min(buf.len(), app_slice.len());
        buf[..size].copy_from_slice(&app_slice[..size]);
        if size == 0 {
            self.upload_ptr = None;
        } else {
            self.upload_ptr = Some(&app_slice[size..]);
        }

        Ok(size)
    }
    fn download(
        &mut self,
        _block_number: u16,
        _buf: &[u8],
    ) -> core::result::Result<(), usbd_dfu::Error> {
        //dbgprint!("download {} {}\r\n", _block_number, _buf.len());
        let new_ptr = self.download_ptr + _buf.len();

        if new_ptr > APPLICATION_REGION_LENGTH {
            Err(usbd_dfu::Error::Address)
        } else {
            self.download_ptr = new_ptr;
            Ok(())
        }
    }
}
