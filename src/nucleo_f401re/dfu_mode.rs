use usbd_dfu::Capabilities;

use super::{ApplicationRef, APPLICATION_REGION_START, FLASH_END, MANIFEST_REGION_START};

#[derive(Debug, PartialEq, Copy, Clone)]
struct Sector(u8);
fn get_sector(address: usize) -> Option<Sector> {
    if address < 0x0800_0000 {
        None
    } else if address < 0x0801_0000 {
        let id = (address >> 14) & 0x0F;
        Some(Sector(id as u8))
    } else if address < 0x0802_0000 {
        Some(Sector(4))
    } else if address < 0x0808_0000 {
        let id = ((address >> 17) & 0x0F) + 4;
        Some(Sector(id as u8))
    } else {
        None
    }
}

#[derive(Debug)]
struct DownloadState {
    array: [u8; DFUModeImpl::TRANSFER_SIZE as usize],
    used: usize,
    ptr: usize,
    program_ptr: *const u8,
    current_sector: Option<Sector>,
}
impl DownloadState {
    fn update(&mut self, flash: &mut stm32f4xx_hal::pac::FLASH) -> Result<bool, usbd_dfu::Error> {
        let sr = flash.sr.read();
        let is_idle = sr.bsy().bit_is_clear();
        if is_idle {
            //Err(usbd_dfu::Error::Erase)
            //Err(usbd_dfu::Error::CheckErased)
            //Err(usbd_dfu::Error::Programming)
            if sr.bits() != 0 {
                Err(usbd_dfu::Error::Programming)
            } else if self.ptr < self.used {
                self.unlock(flash)?;

                let target_addr = self.program_ptr;
                let sector = get_sector(target_addr as usize).unwrap();
                if Some(sector) != self.current_sector {
                    self.current_sector = Some(sector);
                    self.erase_sector(flash, sector);
                } else {
                    self.program(flash);
                }

                Ok(false)
            } else {
                let rd_ptr = unsafe {
                    core::slice::from_raw_parts(
                        self.program_ptr.offset(-(self.used as isize)),
                        self.used,
                    )
                };
                if rd_ptr != &self.array[..self.used] {
                    Err(usbd_dfu::Error::Verify)
                } else {
                    Ok(true)
                }
            }
        } else {
            Ok(false)
        }
    }

    fn unlock(&mut self, flash: &mut stm32f4xx_hal::pac::FLASH) -> Result<(), usbd_dfu::Error> {
        if flash.cr.read().lock().bit_is_set() {
            flash.keyr.write(|w| unsafe { w.bits(0x45670123) });
            flash.keyr.write(|w| unsafe { w.bits(0xCDEF89AB) });

            if flash.cr.read().lock().bit_is_set() {
                return Err(usbd_dfu::Error::Write);
            }
        }
        Ok(())
    }

    fn erase_sector(&mut self, flash: &mut stm32f4xx_hal::pac::FLASH, sector: Sector) {
        flash
            .cr
            .modify(|_, w| unsafe { w.ser().set_bit().snb().bits(sector.0) });
        flash.cr.modify(|_, w| w.strt().set_bit());
    }

    fn program(&mut self, flash: &mut stm32f4xx_hal::pac::FLASH) {
        use stm32f4xx_hal::pac::flash::cr::PSIZE_A;

        let usize_target_addr = self.program_ptr as usize;
        let to_write = usize::min(self.used - self.ptr, 4);

        let psize = if usize_target_addr & 1 == 1 || to_write == 1 {
            PSIZE_A::PSIZE8
        } else if usize_target_addr & 2 == 2 || to_write < 4 {
            PSIZE_A::PSIZE16
        } else {
            PSIZE_A::PSIZE32
        };

        flash
            .cr
            .modify(|_, w| w.pg().set_bit().psize().variant(psize));

        unsafe {
            let ptr = self.program_ptr as *mut u8;
            let src = &self.array[self.ptr..self.used];
            match psize {
                PSIZE_A::PSIZE8 => {
                    core::ptr::write_volatile(ptr, src[0]);
                    self.ptr += 1;
                    self.program_ptr = self.program_ptr.offset(1);
                }
                PSIZE_A::PSIZE16 => {
                    let ptr = ptr as *mut u16;
                    let src = core::ptr::read(src.as_ptr() as *const u16);
                    core::ptr::write_volatile(ptr, src);
                    self.ptr += 2;
                    self.program_ptr = self.program_ptr.offset(2);
                }
                PSIZE_A::PSIZE32 => {
                    let ptr = ptr as *mut u32;
                    let src = core::ptr::read(src.as_ptr() as *const u32);
                    core::ptr::write_volatile(ptr, src);
                    self.ptr += 4;
                    self.program_ptr = self.program_ptr.offset(4);
                }
                PSIZE_A::PSIZE64 => unreachable!(),
            }
        }
    }
}

struct ManifestationState {
    sector: Sector,
}
impl ManifestationState {
    fn update(&mut self, flash: &mut stm32f4xx_hal::pac::FLASH) -> Result<bool, usbd_dfu::Error> {
        // check current sector is erased
        // get next sector
        // if none
        //  update manifest
        // else
        //  erase sector
        Ok(false)
    }
}

enum DFUModeState {
    DownloadState(DownloadState),
    Upload(&'static [u8]),
    Manifestation(ManifestationState),
    None,
}

pub struct DFUModeImpl {
    flash: stm32f4xx_hal::pac::FLASH,
    state: DFUModeState,
}
impl DFUModeImpl {
    pub fn new(flash: stm32f4xx_hal::pac::FLASH) -> Self {
        Self {
            flash,
            state: DFUModeState::None,
        }
    }
}

impl_capabilities!(DFUModeImpl);
impl usbd_dfu::mode::DeviceFirmwareUpgrade for DFUModeImpl {
    const POLL_TIMEOUT: u32 = 1;

    fn is_firmware_valid(&mut self) -> bool {
        let manifest = super::Manifest::get();

        dbgprint!("{:x?}\r\n", &manifest);

        let app = ApplicationRef::get();
        app.compute_hash() == manifest.hash
    }
    fn is_transfer_complete(&mut self) -> Result<bool, usbd_dfu::Error> {
        let state = match &mut self.state {
            DFUModeState::DownloadState(state) => state,
            _ => return Err(usbd_dfu::Error::Unknown),
        };
        dbgprint!("update_transfer\r\n");
        state.update(&mut self.flash)
    }
    fn is_manifestation_in_progress(&mut self) -> bool {
        //if state.program_ptr != (MANIFEST_REGION_START as *const u8) {
        //    return Err(usbd_dfu::Error::NotDone);
        //}
        dbgprint!("update manifest\r\n");
        self.state = DFUModeState::None;
        false
    }

    fn poll(&mut self) -> Result<(), usbd_dfu::Error> {
        match &mut self.state {
            DFUModeState::DownloadState(state) => state.update(&mut self.flash).map(|_| ()),
            _ => Ok(()),
        }
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
        if let DFUModeState::None = self.state {
            self.state = DFUModeState::Upload(super::ApplicationRef::get().0);
        }
        let app_slice = match &mut self.state {
            DFUModeState::Upload(state) => state,
            _ => return Err(usbd_dfu::Error::Unknown),
        };

        let size = usize::min(buf.len(), app_slice.len());
        buf[..size].copy_from_slice(&app_slice[..size]);
        if size != 0 {
            *app_slice = &app_slice[size..];
        } else {
            self.state = DFUModeState::None;
        }

        Ok(size)
    }
    fn download(
        &mut self,
        _block_number: u16,
        buf: &[u8],
    ) -> core::result::Result<(), usbd_dfu::Error> {
        dbgprint!("{}-{}\r\n", _block_number, buf.len());

        if let DFUModeState::None = self.state {
            self.state = DFUModeState::DownloadState(DownloadState {
                array: [0; Self::TRANSFER_SIZE as usize],
                ptr: 0,
                used: 0,
                program_ptr: APPLICATION_REGION_START as *const u8,
                current_sector: None,
            });
        }
        let state = match &mut self.state {
            DFUModeState::DownloadState(state) => state,
            _ => return Err(usbd_dfu::Error::Unknown),
        };

        let end_ptr = unsafe { state.program_ptr.offset(buf.len() as isize) };
        if end_ptr >= (FLASH_END as *const u8) {
            return Err(usbd_dfu::Error::Address);
        }

        state.array[..buf.len()].copy_from_slice(buf);
        state.used = buf.len();
        state.ptr = 0;
        Ok(())
    }
}
