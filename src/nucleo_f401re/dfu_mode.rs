use alloc::{boxed::Box, rc::Rc};
use core::{cell::RefCell, convert::TryFrom, pin::Pin, task::Context};

use futures::{Future, TryFutureExt};

use super::super::{DFUImpl, Manifest, Memory, Result, Sector, MANIFEST_REGION_START};
use super::ApplicationRef;

async fn program<F: futures::Future<Output = Result<usize>>>(
    mut addr: usize,
    mut receive: impl FnMut(&mut [u8]) -> F,
    memory: Rc<RefCell<Memory>>,
) -> Result<()> {
    let mut memory = memory
        .try_borrow_mut()
        .map_err(|_| usbd_dfu::Error::Unknown)?;

    let mut buffer = [0u8; <DFUImpl as usbd_dfu::Capabilities>::TRANSFER_SIZE as usize];
    let mut current_sector = Sector::try_from(addr)?;
    let mut app_length = 0;
    loop {
        let len = receive(&mut buffer).await?;
        if len == 0 {
            break;
        }
        app_length += len;

        let mut wr_slice = &buffer[..len];
        while wr_slice.len() > 0 {
            let sector = Sector::try_from(addr)?;
            if sector != current_sector {
                memory.erase(sector).await?;
                current_sector = sector;
            }

            let increment = memory.program(addr, wr_slice).await?;
            addr += increment;
            wr_slice = &wr_slice[increment..];
        }
    }

    // erase remaining memory
    for sector in current_sector {
        memory.erase(sector).await?;
    }

    let manifest = Manifest {
        length: app_length,
        hash: ApplicationRef::get_with_length(app_length).compute_hash(),
    };
    let manifest: [u8; core::mem::size_of::<Manifest>()] =
        unsafe { core::mem::transmute(manifest) };

    let mut wr_slice = &manifest[..];
    let mut addr = MANIFEST_REGION_START;
    while wr_slice.len() > 0 {
        let increment = memory.program(addr, wr_slice).await?;
        addr += increment;
        wr_slice = &wr_slice[increment..];
    }
    Ok(())
}

enum DFUModeState {
    DownloadState(core::pin::Pin<alloc::boxed::Box<dyn Future<Output = Result<()>>>>),
    Upload(&'static [u8]),
    None,
}

pub struct DFUModeImpl {
    state: DFUModeState,
    memory: alloc::rc::Rc<core::cell::RefCell<Memory>>,
}
impl DFUModeImpl {
    pub(crate) fn new(memory: Memory) -> Self {
        Self {
            state: DFUModeState::None,
            memory: alloc::rc::Rc::new(core::cell::RefCell::new(memory)),
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
    fn is_transfer_complete(&mut self) -> Result<bool> {
        todo!()
        //let state = match &mut self.state {
        //    DFUModeState::DownloadState(state) => state,
        //    _ => return Err(usbd_dfu::Error::Unknown),
        //};
        //dbgprint!("update_transfer\r\n");
        //state.update(&mut self.flash)
    }
    fn is_manifestation_in_progress(&mut self) -> bool {
        //if state.program_ptr != (MANIFEST_REGION_START as *const u8) {
        //    return Err(usbd_dfu::Error::NotDone);
        //}
        dbgprint!("update manifest\r\n");
        //self.state = DFUModeState::None;
        false
    }

    fn poll(&mut self) -> Result<()> {
        match &mut self.state {
            DFUModeState::DownloadState(state) => {
                if let Some(res) = crate::executor::poll_once(state) {
                    self.state = DFUModeState::None;
                    res
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        }
    }

    fn upload(&mut self, _block_number: u16, buf: &mut [u8]) -> Result<usize> {
        //dbgprint!(
        //    "{:?} {} {}\r\n",
        //    self.upload_ptr.map(|slice| slice.len()),
        //    block_number,
        //    buf.len()
        //);
        //if let DFUModeState::None = self.state {
        //    self.state = DFUModeState::Upload(super::ApplicationRef::get().0);
        //}
        //let app_slice = match &mut self.state {
        //    DFUModeState::Upload(state) => state,
        //    _ => return Err(usbd_dfu::Error::Unknown),
        //};

        //let size = usize::min(buf.len(), app_slice.len());
        //buf[..size].copy_from_slice(&app_slice[..size]);
        //if size != 0 {
        //    *app_slice = &app_slice[size..];
        //} else {
        //    self.state = DFUModeState::None;
        //}

        //Ok(size)
        todo!()
    }
    fn download(&mut self, _block_number: u16, buf: &[u8]) -> Result<()> {
        dbgprint!("{}-{}\r\n", _block_number, buf.len());

        self.state = DFUModeState::DownloadState(Box::pin(program(
            super::APPLICATION_REGION_START,
            |buffer| {
                core::future::poll_fn(|ctx| {
                    ctx.waker().wake_by_ref();
                    core::task::Poll::Ready(Ok(0))
                })
            },
            self.memory.clone(),
        ))
            as Pin<Box<dyn Future<Output = Result<()>>>>);

        //if let DFUModeState::None = self.state {
        //    self.state = DFUModeState::DownloadState(DownloadState {
        //    });
        //}
        //let state = match &mut self.state {
        //    DFUModeState::DownloadState(state) => state,
        //    _ => return Err(usbd_dfu::Error::Unknown),
        //};

        //let end_ptr = unsafe { state.program_ptr.offset(buf.len() as isize) };
        //if end_ptr >= (FLASH_END as *const u8) {
        //    return Err(usbd_dfu::Error::Address);
        //}

        //state.array[..buf.len()].copy_from_slice(buf);
        //state.used = buf.len();
        //state.ptr = 0;
        Ok(())
    }
}
