use super::ApplicationRef;

enum DFUModeState {
    DownloadState,
    Upload(&'static [u8]),
    None,
}

pub struct DFUModeImpl {
    state: DFUModeState,
}
impl DFUModeImpl {
    pub fn new<T>(_: T) -> Self {
        Self {
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

    fn poll(&mut self) -> Result<(), usbd_dfu::Error> {
        todo!()
        //match &mut self.state {
        //    DFUModeState::DownloadState(state) => state.update(&mut self.flash).map(|_| ()),
        //    _ => Ok(()),
        //}
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
    fn download(
        &mut self,
        _block_number: u16,
        buf: &[u8],
    ) -> core::result::Result<(), usbd_dfu::Error> {
        dbgprint!("{}-{}\r\n", _block_number, buf.len());

        //if let DFUModeState::None = self.state {
        //    self.state = DFUModeState::DownloadState(DownloadState {
        //        array: [0; Self::TRANSFER_SIZE as usize],
        //        ptr: 0,
        //        used: 0,
        //        program_ptr: APPLICATION_REGION_START as *const u8,
        //        current_sector: None,
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
