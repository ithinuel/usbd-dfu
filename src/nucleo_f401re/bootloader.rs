use core::convert::TryFrom;
use core::task::Poll;
use stm32f4xx_hal::rcc::RccExt;
use usbd_dfu::Result;

use super::MANIFEST_REGION_START;

pub const APPLICATION_REGION_START: usize = 0x0800_8000;
pub const APPLICATION_LENGTH: usize = MANIFEST_REGION_START - APPLICATION_REGION_START;

const SECTORS: [(usize, usize); 8] = [
    (0x0800_0000, 16 * 1024),
    (0x0800_4000, 16 * 1024),
    (0x0800_8000, 16 * 1024),
    (0x0800_C000, 16 * 1024),
    (0x0801_0000, 64 * 1024),
    (0x0802_0000, 128 * 1024),
    (0x0804_0000, 128 * 1024),
    (0x0806_0000, 128 * 1024),
];
#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Sector(usize);
impl Sector {
    fn is_erased(&self) -> bool {
        let arr = self.region();
        arr.iter().cloned().all(|b| b == 0xFFFF_FFFF)
    }
    fn region(&self) -> &'static [u32] {
        let (addr, length) = match self.0 {
            0..=7 => SECTORS[self.0],
            _ => unreachable!(),
        };
        unsafe { core::slice::from_raw_parts(addr as *const u32, length / 4) }
    }
}
impl TryFrom<usize> for Sector {
    type Error = usbd_dfu::Error;

    fn try_from(address: usize) -> Result<Self> {
        if address < 0x0800_0000 {
            Err(usbd_dfu::Error::Address)
        } else if address < 0x0801_0000 {
            let id = (address >> 14) & 0x0F;
            Ok(Sector(id))
        } else if address < 0x0802_0000 {
            Ok(Sector(4))
        } else if address < 0x0808_0000 {
            let id = ((address >> 17) & 0x0F) + 4;
            Ok(Sector(id))
        } else {
            Err(usbd_dfu::Error::Address)
        }
    }
}
impl Iterator for Sector {
    type Item = Sector;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0 < 7 {
            self.0 += 1;
            Some(Sector(self.0))
        } else {
            None
        }
    }
}

#[derive(Debug)]
enum MemoryState {
    Erasing(Sector),
    Programming {
        addr: usize,
        to_write: usize,
        src: [u8; 4],
    },
    Idle,
}

pub struct Memory {
    flash: stm32f4xx_hal::pac::FLASH,
    state: MemoryState,
}
impl Memory {
    pub fn new(flash: stm32f4xx_hal::pac::FLASH) -> Self {
        Self {
            flash,
            state: MemoryState::Idle,
        }
    }
    pub fn unlock(&mut self) -> Result<()> {
        if self.flash.cr.read().lock().bit_is_set() {
            self.flash.keyr.write(|w| unsafe { w.bits(0x45670123) });
            self.flash.keyr.write(|w| unsafe { w.bits(0xCDEF89AB) });

            if self.flash.cr.read().lock().bit_is_set() {
                return Err(usbd_dfu::Error::Write);
            }
        }
        Ok(())
    }

    pub fn poll(&mut self) -> Poll<Result<usize>> {
        //crate::dbgprint!(".");
        match self.state {
            MemoryState::Idle => Poll::Ready(Ok(0)), /* unused */
            MemoryState::Erasing(sector) => {
                //crate::dbgprint!("{:x?}\r\n", self.state);
                let sr = self.flash.sr.read();
                if sr.bsy().bit_is_set() {
                    Poll::Pending
                } else {
                    let res = if sr.wrperr().bit_is_set() {
                        Err(usbd_dfu::Error::Write)
                    } else if sr.operr().bit_is_set() {
                        Err(usbd_dfu::Error::Erase)
                    } else if !sector.is_erased() {
                        Err(usbd_dfu::Error::CheckErased)
                    } else {
                        Ok(0) // unused
                    };
                    self.state = MemoryState::Idle;
                    Poll::Ready(res)
                }
            }
            MemoryState::Programming {
                addr,
                to_write,
                src,
            } => {
                let sr = self.flash.sr.read();
                if sr.bsy().bit_is_set() {
                    Poll::Pending
                } else {
                    let dst = unsafe { core::slice::from_raw_parts(addr as *const u8, to_write) };

                    let res = if sr.wrperr().bit_is_set() {
                        Err(usbd_dfu::Error::Write)
                    } else if sr.operr().bit_is_set() {
                        Err(usbd_dfu::Error::Programming)
                    } else if dst != &src[..to_write] {
                        Err(usbd_dfu::Error::Verify)
                    } else {
                        Ok(to_write)
                    };
                    self.state = MemoryState::Idle;
                    Poll::Ready(res)
                }
            }
        }
    }

    pub fn erase(&mut self, sector: Sector) -> Poll<usbd_dfu::Error> {
        match self.unlock() {
            Ok(()) => {}
            Err(e) => return Poll::Ready(e),
        }

        self.flash
            .cr
            .modify(|_, w| unsafe { w.ser().set_bit().snb().bits(sector.0 as u8) });
        self.flash.cr.modify(|_, w| w.strt().set_bit());

        self.state = MemoryState::Erasing(sector);
        Poll::Pending
    }

    pub fn program(&mut self, addr: usize, src: &[u8]) -> Poll<usbd_dfu::Error> {
        match self.unlock() {
            Ok(()) => {}
            Err(e) => return Poll::Ready(e),
        }
        let mut to_write = if addr & 1 == 1 {
            1
        } else if addr & 2 == 2 {
            2
        } else {
            usize::min(src.len(), 4)
        };
        if to_write == 3 {
            to_write = 2;
        }

        use stm32f4xx_hal::pac::flash::cr::PSIZE_A;
        let psize = match to_write {
            1 => PSIZE_A::PSIZE8,
            2 => PSIZE_A::PSIZE16,
            4 => PSIZE_A::PSIZE32,
            _ => unreachable!(),
        };

        self.flash
            .cr
            .modify(|_, w| w.pg().set_bit().psize().variant(psize));

        unsafe {
            match psize {
                PSIZE_A::PSIZE8 => {
                    let ptr = addr as *mut u8;
                    core::ptr::write_volatile(ptr, src[0]);
                }
                PSIZE_A::PSIZE16 => {
                    let ptr = addr as *mut u16;
                    let src = core::ptr::read(src.as_ptr() as *const u16);
                    core::ptr::write_volatile(ptr, src);
                }
                PSIZE_A::PSIZE32 => {
                    let ptr = addr as *mut u32;
                    let src = core::ptr::read(src.as_ptr() as *const u32);
                    core::ptr::write_volatile(ptr, src);
                }
                PSIZE_A::PSIZE64 => unreachable!(),
            }
        }

        let mut src_owned = [0; 4];
        src_owned[..to_write].copy_from_slice(&src[..to_write]);
        self.state = MemoryState::Programming {
            addr,
            to_write,
            src: src_owned,
        };
        Poll::Pending
    }
}

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
        let clocks = dp.RCC.constrain().cfgr.freeze();
        #[cfg(feature = "debug-buffer")]
        dbgprint!("clocks {:?}\r\n", clocks.hclk()); //
        dbgprint!("clocks {:?}\r\n", clocks.sysclk()); //
        dbgprint!("clocks {:?}\r\n", clocks.pll48clk()); //

        // This for some reason breaks the system.
        //dp.FLASH.acr.modify(|_, w| {
        //    w.latency()
        //        .ws0()
        //        .icen()
        //        .clear_bit()
        //        .dcen()
        //        .clear_bit()
        //        .prften()
        //        .clear_bit()
        //});
        cp.SCB.vtor.write(APPLICATION_REGION_START as u32);
        //cp.SCB.disable_dcache(&mut cp.CPUID);
        //cp.SCB.clean_invalidate_dcache(&mut cp.CPUID);
        //cp.SCB.disable_icache();
        //cp.SCB.invalidate_icache();

        cortex_m::asm::bootload(APPLICATION_REGION_START as *const u32);
    }
}
