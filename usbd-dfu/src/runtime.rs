use usb_device::class_prelude::*;
use usb_device::Result;

use super::{
    Capabilities, Request, State, DFU_FUNCTIONAL, DFU_VERSION, USB_CLASS_DFU,
    USB_DFU_RUNTIME_PROTOCOL, USB_SUB_CLASS_DFU,
};

pub trait DeviceFirmwareUpgrade: Capabilities {
    /// Called by the USB stack when a reset is triggered by the host.
    fn on_reset(&mut self);

    /// Called by the USB stack when a detach request is received by the device. If `will_detach`
    /// is false, the device must initiate the detach-attach sequence now.
    fn on_detach_request(&mut self, timeout_ms: u16);
}

#[allow(non_snake_case)]
pub struct DFURuntimeClass<D: DeviceFirmwareUpgrade> {
    handler: D,
    interface_number: InterfaceNumber,
    state: State,
}

impl<H: DeviceFirmwareUpgrade> DFURuntimeClass<H> {
    pub fn new<B: UsbBus>(alloc: &UsbBusAllocator<B>, handler: H) -> Self {
        Self {
            handler,
            interface_number: alloc.interface(),
            state: State::AppIdle,
        }
    }

    /// Updates the state of the driver. Takes the number of nano-second since last update.
    /// Ideally this method should be called once every millisecond.
    pub fn poll(&mut self, elapsed_ms: u32) {
        if let State::AppDetach(remaining) = &mut self.state {
            let rem = remaining.saturating_sub(elapsed_ms);
            if rem == 0 {
                self.state = State::AppIdle;
            } else {
                *remaining = rem;
            }
        }
    }
}

impl<H: DeviceFirmwareUpgrade, B: UsbBus> UsbClass<B> for DFURuntimeClass<H> {
    fn get_configuration_descriptors(&self, writer: &mut DescriptorWriter) -> Result<()> {
        writer.interface(
            self.interface_number,
            USB_CLASS_DFU,
            USB_SUB_CLASS_DFU,
            USB_DFU_RUNTIME_PROTOCOL,
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

    fn reset(&mut self) {
        if let State::AppDetach(_) = self.state {
            self.handler.on_reset();
        }
    }

    fn control_in(&mut self, xfer: ControlIn<B>) {
        let req = xfer.request();
        if !(req.request_type == control::RequestType::Class
            && req.recipient == control::Recipient::Interface
            && req.index == u8::from(self.interface_number).into())
        {
            return;
        }

        let _ = match req.request {
            Request::DFU_GETSTATE => xfer.accept_with(&[u8::from(self.state)]),
            Request::DFU_GETSTATUS => {
                // there is no error status in runtime dfu
                xfer.accept_with(&[0, 1, self.state.into(), 0])
            }
            _ => {
                self.state = State::AppIdle;
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

        if req.request == Request::DFU_DETACH {
            let timeout_ms = xfer.request().value;

            self.state = State::AppDetach(timeout_ms.into());

            // propagate the event to the handler
            self.handler.on_detach_request(timeout_ms);

            let _ = xfer.accept();
        } else {
            self.state = State::AppIdle;
            let _ = xfer.reject();
        };
    }
}
