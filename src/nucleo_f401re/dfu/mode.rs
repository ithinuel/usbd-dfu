use core::{convert::TryFrom, task::Poll};

use stm32f4xx_hal::gpio::{gpioc::PC13, Floating, Input};
use usbd_dfu::{Capabilities, Result};

use super::{Hash, Manifest};
use crate::platform::{
    bootloader::{Memory, Sector, APPLICATION_LENGTH, APPLICATION_REGION_START},
    MANIFEST_REGION_START,
};

#[repr(C)]
pub struct ApplicationRef(&'static [u8]);
impl ApplicationRef {
    pub fn get_with_length(length: usize) -> Self {
        let length = usize::min(APPLICATION_LENGTH, length);
        unsafe {
            Self(core::slice::from_raw_parts(
                APPLICATION_REGION_START as *const u8,
                length,
            ))
        }
    }
    fn get() -> Self {
        let manifest = Manifest::get();
        Self::get_with_length(manifest.length)
    }
    pub fn compute_hash(&self) -> Hash {
        #[cfg(not(feature = "use-sha256"))]
        {
            let mut sha = sha1::Sha1::new();
            sha.update(&self.0);
            sha.digest().bytes()
        }
        #[cfg(feature = "use-sha256")]
        {
            hmac_sha256::Hash::hash(&self.0)
        }
    }
}

#[derive(Clone, Debug)]
enum ProgramState {
    AwaitData,
    AwaitEraseBeforeProgram {
        data: [u8; DFUModeImpl::TRANSFER_SIZE as usize],
        data_len: usize,
        wr_ptr: usize,
    },
    AwaitProgram {
        data: [u8; DFUModeImpl::TRANSFER_SIZE as usize],
        data_len: usize,
        wr_ptr: usize,
    },
    AwaitErase,
    AwaitProgramManifest {
        data: [u8; core::mem::size_of::<Manifest>()],
        wr_ptr: usize,
    },
    Done,
}

#[derive(Clone, Debug)]
struct Program {
    current_sector: Sector,
    addr: usize,
    state: ProgramState,
}
impl Program {
    fn new(memory: &mut Memory, buf: &[u8]) -> Result<Self> {
        if buf.len() >= APPLICATION_LENGTH {
            return Err(usbd_dfu::Error::Address);
        }
        let data_len = buf.len();
        let mut data = [0; DFUModeImpl::TRANSFER_SIZE as usize];
        data[..data_len].copy_from_slice(buf);

        // erase first sector
        let current_sector = Sector::try_from(APPLICATION_REGION_START)?;
        match memory.erase(current_sector) {
            Poll::Ready(e) => return Err(e),
            Poll::Pending => {}
        }

        Ok(Self {
            current_sector,
            addr: APPLICATION_REGION_START,
            state: ProgramState::AwaitEraseBeforeProgram {
                data,
                data_len,
                wr_ptr: 0,
            },
        })
    }
    fn update(&mut self, memory: &mut Memory, buf: &[u8]) -> Poll<usbd_dfu::Error> {
        match &mut self.state {
            ProgramState::AwaitData => {
                let data_len = buf.len();
                let mut data = [0; DFUModeImpl::TRANSFER_SIZE as usize];
                data[..data_len].copy_from_slice(buf);

                self.state = match Self::erase_or_program(
                    memory,
                    self.addr,
                    &mut self.current_sector,
                    data,
                    data_len,
                    0,
                ) {
                    Ok(state) => state,
                    Err(e) => return Poll::Ready(e),
                };
                Poll::Pending
            }
            _ => Poll::Ready(usbd_dfu::Error::Unknown),
        }
    }
    fn finalize(&mut self, memory: &mut Memory) -> Poll<usbd_dfu::Error> {
        match self.state {
            ProgramState::AwaitData => match self.erase_or_program_manifest(memory) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(_)) => unreachable!(),
                Poll::Ready(Err(e)) => Poll::Ready(e),
            },
            _ => Poll::Ready(usbd_dfu::Error::Unknown),
        }
    }
    fn poll(&mut self, memory: &mut Memory) -> Poll<Result<()>> {
        if let ProgramState::AwaitData = self.state {
            return Poll::Pending;
        }

        match memory.poll() {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(n)) => match self.state {
                ProgramState::AwaitData => unreachable!(),
                ProgramState::AwaitEraseBeforeProgram {
                    data,
                    data_len,
                    wr_ptr,
                } => {
                    match memory.program(self.addr, &data[wr_ptr..data_len]) {
                        Poll::Ready(e) => return Poll::Ready(Err(e)),
                        Poll::Pending => {}
                    }
                    self.state = ProgramState::AwaitProgram {
                        data,
                        data_len,
                        wr_ptr,
                    };
                    Poll::Pending
                }
                ProgramState::AwaitProgram {
                    data,
                    data_len,
                    mut wr_ptr,
                } => {
                    self.addr += n;
                    wr_ptr += n;

                    assert!(wr_ptr <= data_len);
                    self.state = if wr_ptr == data_len {
                        ProgramState::AwaitData
                    } else {
                        match Self::erase_or_program(
                            memory,
                            self.addr,
                            &mut self.current_sector,
                            data,
                            data_len,
                            wr_ptr,
                        ) {
                            Err(e) => return Poll::Ready(Err(e)),
                            Ok(state) => state,
                        }
                    };
                    Poll::Pending
                }
                ProgramState::AwaitErase => self.erase_or_program_manifest(memory),
                ProgramState::AwaitProgramManifest { data, mut wr_ptr } => {
                    self.addr += n;
                    wr_ptr += n;

                    assert!(wr_ptr <= data.len());
                    if wr_ptr == data.len() {
                        self.state = ProgramState::Done;
                        Poll::Ready(Ok(()))
                    } else {
                        self.state = ProgramState::AwaitProgramManifest { data, wr_ptr };
                        match memory.program(self.addr, &data[wr_ptr..]) {
                            Poll::Ready(e) => Poll::Ready(Err(e)),
                            Poll::Pending => Poll::Pending,
                        }
                    }
                }
                ProgramState::Done => unreachable!(),
            },
        }
    }

    fn erase_or_program(
        memory: &mut Memory,
        addr: usize,
        current_sector: &mut Sector,
        data: [u8; DFUModeImpl::TRANSFER_SIZE as usize],
        data_len: usize,
        wr_ptr: usize,
    ) -> Result<ProgramState> {
        if addr >= MANIFEST_REGION_START {
            return Err(usbd_dfu::Error::Address);
        }

        let sector = match Sector::try_from(addr) {
            Ok(sector) => sector,
            Err(e) => return Err(e),
        };

        let state = if sector != *current_sector {
            *current_sector = sector;
            match memory.erase(sector) {
                Poll::Ready(e) => return Err(e),
                Poll::Pending => {}
            }
            ProgramState::AwaitEraseBeforeProgram {
                data,
                data_len,
                wr_ptr,
            }
        } else {
            match memory.program(addr, &data[wr_ptr..data_len]) {
                Poll::Pending => {}
                Poll::Ready(e) => return Err(e),
            }

            ProgramState::AwaitProgram {
                data,
                data_len,
                wr_ptr,
            }
        };
        Ok(state)
    }

    fn erase_or_program_manifest(&mut self, memory: &mut Memory) -> Poll<Result<()>> {
        match self.current_sector.next() {
            Some(sector) => {
                self.state = ProgramState::AwaitErase;
                match memory.erase(sector) {
                    Poll::Ready(e) => Poll::Ready(Err(e)),
                    Poll::Pending => Poll::Pending,
                }
            }
            None => {
                let length = self.addr - APPLICATION_REGION_START;
                let manifest = Manifest {
                    length,
                    hash: ApplicationRef::get_with_length(length).compute_hash(),
                };
                let manifest: [u8; core::mem::size_of::<Manifest>()] =
                    unsafe { core::mem::transmute(manifest) };

                match memory.program(MANIFEST_REGION_START, &manifest[..]) {
                    Poll::Ready(e) => return Poll::Ready(Err(e)),
                    Poll::Pending => {}
                }
                self.addr = MANIFEST_REGION_START;
                self.state = ProgramState::AwaitProgramManifest {
                    data: manifest,
                    wr_ptr: 0,
                };
                Poll::Pending
            }
        }
    }
}

#[derive(Debug)]
enum DFUModeState {
    Download(Program),
    Manifetation(Program),
    Upload(&'static [u8]),
    Idle,
    Error,
}

pub struct DFUModeImpl {
    state: DFUModeState,
    memory: Memory,
    boot_mode: PC13<Input<Floating>>,
}
impl DFUModeImpl {
    pub fn new(memory: Memory, boot_mode: PC13<Input<Floating>>) -> Self {
        Self {
            state: DFUModeState::Idle,
            memory,
            boot_mode,
        }
    }
}

impl_capabilities!(DFUModeImpl);
impl usbd_dfu::mode::DeviceFirmwareUpgrade for DFUModeImpl {
    const POLL_TIMEOUT: u32 = 10;

    fn is_firmware_valid(&mut self) -> bool {
        let manifest = super::Manifest::get();

        dbgprint!("{:x?}\r\n", &manifest);

        use embedded_hal::digital::v2::InputPin;
        if self.boot_mode.is_low().unwrap_or_else(|_| unreachable!()) {
            return false;
        }

        let app = ApplicationRef::get();
        let is_hash_valid = app.compute_hash() == manifest.hash;

        if is_hash_valid {
            let (sp, reset) = unsafe {
                let ptr = APPLICATION_REGION_START as *const u32;
                (*ptr, *ptr.offset(1))
            };
            dbgprint!("{:x} {:x}\r\n", sp, reset);
            (0x2000_0000..0x2002_0000).contains(&sp)
                && (APPLICATION_REGION_START..MANIFEST_REGION_START).contains(&(reset as usize))
        } else {
            false
        }
    }
    fn is_transfer_complete(&mut self) -> Result<bool> {
        if let DFUModeState::Download(program) = &self.state {
            if let ProgramState::AwaitData = program.state {
                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            Err(usbd_dfu::Error::Unknown)
        }
    }
    fn is_manifestation_in_progress(&mut self) -> Result<bool> {
        dbgprint!("update manifest\r\n");
        let res = match &mut self.state {
            DFUModeState::Download(program) => match program.finalize(&mut self.memory) {
                Poll::Ready(e) => Err(e),
                Poll::Pending => {
                    self.state = DFUModeState::Manifetation(program.clone());
                    Ok(true)
                }
            },
            DFUModeState::Manifetation(program) => match program.poll(&mut self.memory) {
                Poll::Ready(Ok(())) => {
                    self.state = DFUModeState::Idle;
                    Ok(false)
                }
                Poll::Ready(Err(e)) => Err(e),
                Poll::Pending => Ok(true),
            },
            DFUModeState::Idle => Ok(false),
            _ => Err(usbd_dfu::Error::Unknown),
        };
        if res.is_err() {
            self.state = DFUModeState::Error;
        }
        res
    }

    fn poll(&mut self) -> Result<()> {
        //dbgprint!("{:x?}\r\n", &self.state);
        match &mut self.state {
            DFUModeState::Download(program) | DFUModeState::Manifetation(program) => {
                match program.poll(&mut self.memory) {
                    Poll::Pending => {}
                    Poll::Ready(Ok(())) => self.state = DFUModeState::Idle,
                    Poll::Ready(Err(e)) => {
                        self.state = DFUModeState::Error;
                        return Err(e);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn upload(&mut self, _block_number: u16, buf: &mut [u8]) -> Result<usize> {
        //dbgprint!(
        //    "{:?} {} {}\r\n",
        //    self.upload_ptr.map(|slice| slice.len()),
        //    block_number,
        //    buf.len()
        //);

        if let DFUModeState::Idle = self.state {
            self.state = DFUModeState::Upload(ApplicationRef::get().0);
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
            self.state = DFUModeState::Idle;
        }

        Ok(size)
    }
    fn download(&mut self, _block_number: u16, buf: &[u8]) -> Result<()> {
        dbgprint!("{}-{}\r\n", _block_number, buf.len());

        let res = match &mut self.state {
            DFUModeState::Idle => {
                let program_state = Program::new(&mut self.memory, buf)?;
                self.state = DFUModeState::Download(program_state);
                Ok(())
            }
            DFUModeState::Download(state) => match state.update(&mut self.memory, buf) {
                Poll::Ready(e) => Err(e),
                Poll::Pending => Ok(()),
            },
            _ => Err(usbd_dfu::Error::Unknown),
        };

        if res.is_err() {
            self.state = DFUModeState::Error;
        }
        res
    }
}
