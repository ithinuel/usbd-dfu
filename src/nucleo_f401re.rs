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

#[cfg(feature = "application")]
use cortex_m::interrupt;

#[cfg(feature = "bootloader")]
type DFUImpl = DFUModeImpl;
#[cfg(feature = "application")]
type DFUImpl = DFURuntimeImpl;

const APPLICATION_REGION_START: usize = 0x0800_4000;
const APPLICATION_REGION_LENGTH: usize = (16 * 3 + 64 + 128 * 3) * 1024;
const APPLICATION_MANIFEST_START: usize = 0x0807_0000 - 128;

static mut EP_MEMORY: MaybeUninit<[u32; 256]> = MaybeUninit::uninit();
#[cfg(feature = "application")]
static mut DEBUG_BUFFER: MaybeUninit<[u8; 1024]> = MaybeUninit::uninit();
pub static mut WRITER: Option<DbgWriter> = None;

fn get_manifest() -> &'static Manifest {
    unsafe { &*(APPLICATION_MANIFEST_START as *const Manifest) }
}

pub fn reset() -> ! {
    stm32f4xx_hal::stm32::SCB::sys_reset()
}

pub fn init() -> (
    usb_device::bus::UsbBusAllocator<impl usb_device::class_prelude::UsbBus>,
    impl embedded_hal::digital::v2::OutputPin,
    cortex_m::Peripherals,
    DFUImpl,
) {
    let dp = stm32f4xx_hal::stm32::Peripherals::take().unwrap();
    let mut cp = cortex_m::Peripherals::take().unwrap();

    let rcc = dp.RCC.constrain();

    let clocks = rcc.cfgr.sysclk(48.mhz()).require_pll48clk().freeze();

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

    #[cfg(feature = "application")]
    interrupt::free(move |_| unsafe {
        WRITER = Some(DbgWriter::using_buffer(DEBUG_BUFFER.assume_init_mut()));
    });

    #[cfg(feature = "application")]
    let dfu = DFURuntimeImpl;
    #[cfg(feature = "bootloader")]
    let dfu = DFUModeImpl {
        _upload_ptr: None,
        _download_ptr: None,
    };

    (
        UsbBus::new(usb, unsafe { EP_MEMORY.assume_init_mut() }),
        led,
        cp,
        dfu,
    )
}

#[cfg(feature = "application")]
pub async fn trigger<T>(_: &mut T) {
    alloc::vec::Vec::<u8>::with_capacity(256);
}

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
#[cfg(feature = "application")]
pub fn consume_debug(mut reader: impl FnMut(&[u8]) -> usize) {
    interrupt::free(|_| unsafe {
        WRITER.as_mut().map(|w| {
            let len = w.len;
            w.len -= reader(&w.buffer[..len]);
            w.buffer.copy_within(w.len..len, 0);
        });
    });
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
            const TRANSFER_SIZE: u16 = 4096;
        }
    };
}

pub struct DFURuntimeImpl;
impl DFURuntimeImpl {
    pub async fn read(&self) -> &'static Manifest {
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

#[repr(C)]
#[derive(Debug)]
pub struct Manifest {
    hash: [u8; 32],
}

pub struct DFUModeImpl {
    _upload_ptr: Option<&'static [u8]>,
    _download_ptr: Option<usize>,
}
impl_capabilities!(DFUModeImpl);
impl usbd_dfu::mode::DeviceFirmwareUpgrade for DFUModeImpl {
    const POLL_TIMEOUT: u32 = 1;

    fn is_firmware_valid(&mut self) -> bool {
        let app_region = unsafe {
            from_raw_parts(
                APPLICATION_REGION_START as *const u8,
                APPLICATION_REGION_LENGTH,
            )
        };
        // compute sha256
        let computed = hmac_sha256::Hash::hash(app_region);
        // check against manifest
        let manifest = get_manifest();
        manifest.hash == computed
    }
    fn is_transfer_complete(&mut self) -> bool {
        todo!()
    }
    fn is_manifestation_in_progress(&mut self) -> bool {
        todo!()
    }

    fn upload(
        &mut self,
        _block_number: u16,
        _buf: &mut [u8],
    ) -> core::result::Result<usize, usbd_dfu::Error> {
        Err(usbd_dfu::Error::Unknown)
    }
    fn download(
        &mut self,
        _block_number: u16,
        _buf: &[u8],
    ) -> core::result::Result<(), usbd_dfu::Error> {
        todo!()
    }
}
