use usb_device::class_prelude::*;
use usb_device::Result;

use super::{
    Capabilities, Error, Request, State, DFU_FUNCTIONAL, DFU_VERSION, USB_CLASS_DFU,
    USB_DFU_MODE_PROTOCOL, USB_SUB_CLASS_DFU,
};

pub trait DeviceFirmwareUpgrade: Capabilities {
    const POLL_TIMEOUT: u32;

    fn is_firmware_valid(&mut self) -> bool;
    fn is_transfer_complete(&mut self) -> core::result::Result<bool, Error>;
    fn is_manifestation_in_progress(&mut self) -> bool;

    fn poll(&mut self) -> core::result::Result<(), Error>;

    fn upload(&mut self, block_number: u16, buf: &mut [u8]) -> core::result::Result<usize, Error>;
    fn download(&mut self, block_number: u16, buf: &[u8]) -> core::result::Result<(), Error>;
}

pub struct DFUModeClass<H: DeviceFirmwareUpgrade, B: UsbBus> {
    interface_number: InterfaceNumber,
    handler: H,
    state: State,
    _bus: core::marker::PhantomData<B>,
}
impl<H: DeviceFirmwareUpgrade, B: UsbBus> DFUModeClass<H, B> {
    pub fn new(alloc: &UsbBusAllocator<B>, mut handler: H) -> Self {
        let interface_number = alloc.interface();
        let firmware_is_valid = handler.is_firmware_valid();
        Self {
            interface_number,
            handler,
            state: if firmware_is_valid {
                State::DfuIdle
            } else {
                State::DfuError(Error::Firmware)
            },
            _bus: core::marker::PhantomData,
        }
    }

    fn idle_in(&mut self, xfer: ControlIn<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_UPLOAD if H::CAN_UPLOAD && req.length <= H::TRANSFER_SIZE => {
                self.accept_upload(xfer)
            }
            Request::DFU_GETSTATUS => self.accept_get_status(xfer, 1),
            Request::DFU_GETSTATE => self.accept_get_state(xfer),
            _ => self.stall_in(xfer),
        }
    }
    fn idle_out(&mut self, xfer: ControlOut<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_DNLOAD if H::CAN_DOWNLOAD && req.length > 0 => self.accept_download(xfer),
            Request::DFU_ABORT => xfer.accept(),
            _ => self.stall_out(xfer),
        }
    }
    fn download_sync_in(&mut self, xfer: ControlIn<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_GETSTATE => self.accept_get_state(xfer),
            Request::DFU_GETSTATUS => {
                self.state = match self.handler.is_transfer_complete() {
                    Ok(true) => State::DfuDnloadIdle,
                    Ok(false) => State::DfuDnloadBusy(H::POLL_TIMEOUT),
                    Err(e) => State::DfuError(e),
                };
                self.accept_get_status(xfer, H::POLL_TIMEOUT)
            }
            _ => self.stall_in(xfer),
        }
    }

    fn download_idle_in(&mut self, xfer: ControlIn<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_GETSTATE => self.accept_get_state(xfer),
            Request::DFU_GETSTATUS => self.accept_get_status(xfer, H::POLL_TIMEOUT),
            _ => self.stall_in(xfer),
        }
    }
    fn download_idle_out(&mut self, xfer: ControlOut<B>) -> Result<()> {
        let req = xfer.request();

        let block_number = req.value;
        let data = xfer.data();

        match req.request {
            Request::DFU_DNLOAD if req.length > 0 => {
                self.state = State::DfuDnloadSync;
                if let Err(e) = self.handler.download(block_number, data) {
                    self.state = State::DfuError(e);
                }
                xfer.accept()
            }
            Request::DFU_DNLOAD => {
                if let Ok(true) = self.handler.is_transfer_complete() {
                    self.state = State::DfuManifestSync;
                    xfer.accept()
                } else {
                    // sets the status to StalledPkt so we can discard the transfer error
                    self.stall_out(xfer)
                }
            }
            _ => self.stall_out(xfer),
        }
    }
    fn manifest_sync_in(&mut self, xfer: ControlIn<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_GETSTATE => self.accept_get_state(xfer),
            Request::DFU_GETSTATUS if self.handler.is_manifestation_in_progress() => {
                self.state = State::DfuManifest(H::POLL_TIMEOUT);
                self.accept_get_status(xfer, H::POLL_TIMEOUT)
            }
            Request::DFU_GETSTATUS
                if H::IS_MANIFESTATION_TOLERANT && !self.handler.is_manifestation_in_progress() =>
            {
                self.state = State::DfuIdle;
                self.accept_get_status(xfer, H::POLL_TIMEOUT)
            }
            _ => self.stall_in(xfer),
        }
    }
    fn upload_in(&mut self, xfer: ControlIn<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_UPLOAD => self.accept_upload(xfer),
            Request::DFU_GETSTATUS => self.accept_get_status(xfer, 1),
            Request::DFU_GETSTATE => self.accept_get_state(xfer),
            _ => self.stall_in(xfer),
        }
    }
    fn upload_out(&mut self, xfer: ControlOut<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_ABORT => {
                self.state = State::DfuIdle;
                xfer.accept()
            }
            _ => self.stall_out(xfer),
        }
    }
    fn error_in(&mut self, xfer: ControlIn<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_GETSTATE => self.accept_get_state(xfer),
            Request::DFU_GETSTATUS => self.accept_get_status(xfer, 1),
            _ => xfer.reject(),
        }
    }
    fn error_out(&mut self, xfer: ControlOut<B>) -> Result<()> {
        let req = xfer.request();
        match req.request {
            Request::DFU_CLRSTATUS => {
                self.state = State::DfuIdle;
                xfer.accept()
            }
            _ => xfer.reject(),
        }
    }

    fn accept_download(&mut self, xfer: ControlOut<B>) -> Result<()> {
        let req = xfer.request();
        let block_number = req.value;
        let data = xfer.data();

        assert_eq!(usize::from(req.length), data.len());

        self.state = State::DfuDnloadSync;
        if let Err(e) = self.handler.download(block_number, data) {
            self.state = State::DfuError(e);
        }

        xfer.accept()
    }

    fn accept_upload(&mut self, xfer: ControlIn<B>) -> Result<()> {
        let req = xfer.request();
        let block_number = req.value;
        let length = req.length.into();

        self.state = State::DfuUploadIdle;

        xfer.accept(|buf| {
            let res = self.handler.upload(block_number, buf);

            match res {
                Ok(sz) => {
                    if sz < length {
                        self.state = State::DfuIdle;
                    }
                    Ok(sz)
                }
                Err(e) => {
                    self.state = State::DfuError(e);
                    Ok(0)
                }
            }
        })
    }

    fn accept_get_state(&mut self, xfer: ControlIn<B>) -> Result<()> {
        xfer.accept_with(&[self.state.into()])
    }
    fn accept_get_status(&mut self, xfer: ControlIn<B>, poll_timeout: u32) -> Result<()> {
        let status = if let State::DfuError(e) = self.state {
            e.into()
        } else {
            0
        };
        let poll_timeout = &poll_timeout.to_le_bytes()[..3];
        let mut status = [status, 0, 0, 0, self.state.into(), 0];
        status[1..4].copy_from_slice(poll_timeout);

        xfer.accept_with(&status)
    }

    fn stall_in(&mut self, xfer: ControlIn<B>) -> Result<()> {
        self.state = State::DfuError(Error::StalledPkt);
        xfer.reject()
    }
    fn stall_out(&mut self, xfer: ControlOut<B>) -> Result<()> {
        self.state = State::DfuError(Error::StalledPkt);
        xfer.reject()
    }

    pub fn poll(&mut self, elapsed: u32) {
        match &mut self.state {
            State::DfuDnloadBusy(timeout) => match self.handler.poll() {
                Ok(_) => {
                    let remaining = timeout.saturating_sub(elapsed);
                    if remaining == 0 {
                        self.state = State::DfuDnloadSync;
                    } else {
                        *timeout = remaining;
                    }
                }
                Err(e) => self.state = State::DfuError(e),
            },
            State::DfuManifest(timeout) => match self.handler.poll() {
                Ok(_) => {
                    let remaining = timeout.saturating_sub(elapsed);
                    if remaining != 0 {
                        *timeout = remaining;
                    } else if H::IS_MANIFESTATION_TOLERANT {
                        self.state = State::DfuManifestSync;
                    } else {
                        self.state = State::DfuManifestWaitReset;
                    }
                }
                Err(e) => self.state = State::DfuError(e),
            },
            _ => {}
        }
    }

    pub fn state(&self) -> State {
        self.state
    }
}
impl<B: UsbBus, H: DeviceFirmwareUpgrade> UsbClass<B> for DFUModeClass<H, B> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface(
            self.interface_number,
            USB_CLASS_DFU,
            USB_SUB_CLASS_DFU,
            USB_DFU_MODE_PROTOCOL,
        )?;

        let attributes = {
            (if H::WILL_DETACH { 0b0000_1000 } else { 0 })
                | (if H::IS_MANIFESTATION_TOLERANT {
                    0b0000_0100
                } else {
                    0
                })
                | (if H::CAN_UPLOAD { 0b0000_0010 } else { 0 })
                | (if H::CAN_DOWNLOAD { 0b0000_0001 } else { 0 })
        };

        let mut descriptor = [attributes, 0, 0, 0, 0, 0, 0];
        descriptor[1..3].copy_from_slice(&H::DETACH_TIMEOUT.to_le_bytes());
        descriptor[3..5].copy_from_slice(&H::TRANSFER_SIZE.to_le_bytes());
        descriptor[5..7].copy_from_slice(&DFU_VERSION.to_le_bytes());
        writer.write(DFU_FUNCTIONAL, &descriptor)?;

        Ok(())
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = xfer.request();
        if !(req.request_type == control::RequestType::Class
            && req.recipient == control::Recipient::Interface
            && req.index == u8::from(self.interface_number).into())
        {
            return;
        }

        let _ = match self.state {
            State::DfuIdle => self.idle_in(xfer),
            State::DfuDnloadSync => self.download_sync_in(xfer),
            State::DfuDnloadIdle => self.download_idle_in(xfer),
            State::DfuManifestSync => self.manifest_sync_in(xfer),
            State::DfuManifestWaitReset => xfer.accept_with_static(&[]),
            State::DfuUploadIdle => self.upload_in(xfer),
            State::DfuError(_) => self.error_in(xfer),
            _ => {
                self.state = State::DfuError(Error::StalledPkt);
                xfer.reject()
            }
        };
    }
    fn control_out(&mut self, xfer: ControlOut<B>) {
        let req = xfer.request();
        if !(req.request_type == control::RequestType::Class
            && req.recipient == control::Recipient::Interface
            && req.index == u8::from(self.interface_number).into())
        {
            return;
        }

        let _ = match self.state {
            State::DfuIdle => self.idle_out(xfer),
            State::DfuDnloadIdle => self.download_idle_out(xfer),
            State::DfuManifestWaitReset => xfer.accept(),
            State::DfuUploadIdle => self.upload_out(xfer),
            State::DfuError(_) => self.error_out(xfer),
            _ => {
                self.state = State::DfuError(Error::StalledPkt);
                xfer.reject()
            }
        };
    }
}
