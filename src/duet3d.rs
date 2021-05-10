use cortex_m::peripheral::syst::SystClkSource;
use usbd_dfu::Error;
use atsam4e_hal::pac::{efc, EFC};
pub use atsam4e_hal::dbgprint;
use atsam4e_hal::pmc::{MainClock, PmcExt};
use atsam4e_hal::time::U32Ext;
use atsam4e_hal::usb::*;
use atsam4e_hal::gpio::GpioExt;

use embedded_hal::digital::v2::OutputPin;

pub use atsam4e_hal::debug_on_buffer::consume as consume_debug;

pub fn init() -> (
    usb_device::bus::UsbBusAllocator<impl usb_device::class_prelude::UsbBus>,
    impl OutputPin,
    cortex_m::Peripherals,
    atsam4e_hal::pac::EFC
) {
    // Get access to the device specific peripherals from the peripheral access crate
    let p = atsam4e_hal::pac::Peripherals::take().unwrap_or_else(|| unreachable!());
    let mut cp = cortex_m::Peripherals::take().unwrap_or_else(|| unreachable!());

    p.WDT.mr.write(|w| w.wddis().set_bit());

    // configure the clocks
    let pmc = p.PMC.constrain(); // constrain comes form a trait in the sam4e hal

    // Freeze the configuration of all the clocks in the system and store
    // the frozen frequencies in `clocks`
    let clocks = pmc
        .main_clock(MainClock::External(12.mhz().into()))
        .master_clock(120.mhz())
        .use_usb()
        .freeze();

    let led = p.PIOC.split().pc16.into_push_pull_output(false);

    cp.SYST.set_clock_source(SystClkSource::External);
    cp.SYST.set_reload(clocks.master_clock.0 / (8 * 1_000));
    cp.SYST.clear_current();
    cp.SYST.enable_counter();

    static mut DEBUG_BUFFER: [u8; 1024] = [0; 1024];
    unsafe { atsam4e_hal::debug_on_buffer::setup_with_buffer(&mut DEBUG_BUFFER) };
    (usb_device::bus::UsbBusAllocator::new(UsbBus::new(p.UDP, (DDP, DDM), clocks)), led, cp, p.EFC)
}

pub async fn trigger(efc: &mut atsam4e_hal::pac::EFC) {
    let _info = FlashInfo::read(efc).await;
    //dbgprint!("{:#?}", _info);
}

pub fn reset() -> ! {
    let rstc = unsafe { &*atsam4e_hal::pac::RSTC::ptr() };
    rstc.cr.write_with_zero(|w| {
        w.procrst()
            .set_bit()
            .perrst()
            .set_bit()
            .key()
            .variant(atsam4e_hal::pac::rstc::cr::KEY_AW::PASSWD)
    });
    loop {
        cortex_m::asm::nop();
    }
}

#[derive(Debug)]
pub struct FlashInfo {
    pub id: u32,
    pub size: u32,
    pub page_size: u32,
    pub bytes_per_plane: alloc::vec::Vec<u32>,
    pub bytes_per_lock_region: alloc::vec::Vec<u32>,
}
impl FlashInfo {
    pub async fn read<'efc>(efc: &'efc mut EFC) -> FlashInfo {
        efc.fcr.write_with_zero(|w| w.fcmd().getd().fkey().passwd());

        use core::task::Poll;
        futures::future::poll_fn(|_| {
            if efc.fsr.read().frdy().bit_is_set() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;

        let id = efc.frr.read().bits();
        let size = efc.frr.read().bits();
        let page_size = efc.frr.read().bits();
        let nb_planes = efc.frr.read().bits() as usize;
        let bytes_per_plane = (0..nb_planes).map(|_| efc.frr.read().bits()).collect();
        let nb_lock_region = efc.frr.read().bits();
        let bytes_per_lock_region = (0..nb_lock_region).map(|_| efc.frr.read().bits()).collect();
        FlashInfo {
            id,
            size,
            page_size,
            bytes_per_plane,
            bytes_per_lock_region,
        }
    }
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
impl usbd_dfu::runtime::DeviceFirmwareUpgrade for DFURuntimeImpl {
    fn on_reset(&mut self) {
        // trigger mcu reset to enter bootloader!
        let rstc = unsafe { &*atsam4e_hal::pac::RSTC::ptr() };

        rstc.cr.write_with_zero(|w| {
            w.procrst()
                .set_bit()
                .perrst()
                .set_bit()
                .key()
                .variant(atsam4e_hal::pac::rstc::cr::KEY_AW::PASSWD)
        });
        loop {
            cortex_m::asm::nop();
        }
    }

    fn on_detach_request(&mut self, _timeout_ms: u16) {}
}

pub struct DFUModeImpl {
    /// Embedded Flash Controller
    efc: EFC,
}
impl DFUModeImpl {
    pub fn new(efc: EFC) -> Self {
        Self { efc }
    }
}
impl_capabilities!(DFUModeImpl);
impl usbd_dfu::mode::DeviceFirmwareUpgrade for DFUModeImpl {
    const POLL_TIMEOUT: u32 = 1000;

    fn is_firmware_valid(&mut self) -> bool {
        //TODO: actually validate the flash !
        true
    }

    fn is_transfer_complete(&mut self) -> bool {
        true
    }

    fn is_manifestation_in_progress(&mut self) -> bool {
        false
    }

    fn upload(&mut self, _block_number: u16, _buf: &mut [u8]) -> core::result::Result<usize, Error> {
        Ok(25)
    }
    fn download(&mut self, _block_number: u16, _buf: &[u8]) -> core::result::Result<(), Error> {
        let _fc = &mut self.efc;

        // start address:
        const _PAGE_SIZE: u32 = 512 / 8; //

        // write data to the latch buffer with 32bit operations aligned to 32bits addresses.

        use efc::fcr::*;
        let cmd: u32 = u8::from(FCMD_AW::EWP).into();
        let page_number = 0;
        let key: u32 = u8::from(FKEY_AW::PASSWD).into();

        let iap_arg = (key << 24) | (page_number << 8) | cmd;
        let iap_fsr = unsafe {
            let iap: fn(u32) -> u32 = *(0x0080_0008 as *const usize as *const _);
            cortex_m::interrupt::free(|_| iap(iap_arg))
        };
        assert_eq!(
            iap_fsr & 1,
            1,
            "IAP method only returns once the EFC is ready."
        );
        if iap_fsr & 2 == 2 {
            Err(Error::Programming)
        } else if iap_fsr & 8 == 8 {
            Err(Error::Verify)
        } else if iap_fsr != 0 {
            Err(Error::Unknown)
        } else {
            Ok(())
        }
    }
}
