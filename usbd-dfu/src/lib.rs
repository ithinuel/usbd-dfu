#![no_std]

pub mod mode;
pub mod runtime;

pub const USB_CLASS_DFU: u8 = 0xFE;
pub const USB_SUB_CLASS_DFU: u8 = 0x01;
pub const USB_DFU_RUNTIME_PROTOCOL: u8 = 0x01;
pub const USB_DFU_MODE_PROTOCOL: u8 = 0x02;

pub const DFU_FUNCTIONAL: u8 = 0x21;

pub const DFU_VERSION: u16 = 0x0100; // bcdDFUVersion

pub trait Capabilities {
    /// If true, the device generates a detach-attach sequence on its own upon receipt of a detach
    /// request. Otherwise the device waits for a USB reset until a time out expires.
    const WILL_DETACH: bool;

    /// If true, the device is able to handle other DFU task after a download has completed.
    /// Otherwise the device expects a USB reset.
    const IS_MANIFESTATION_TOLERANT: bool;

    /// True if the device can send its current firmware to the host.
    const CAN_UPLOAD: bool;

    /// True if the device can receive a firmware from the host.
    const CAN_DOWNLOAD: bool;

    /// Time, in milliseconds, that the device will wait after receipt of the detach request. If
    /// this time elapses without a USB reset, then the device will terminate the reconfiguration
    /// phase and revert back to normal operation. This represents the maximum time that the device
    /// can wait (depending on its timers, etc.). The host may specify a shorter timeout in the
    /// detach request.
    const DETACH_TIMEOUT: u16;

    /// Maximum number of bytes the device can accept between per control-write transaction.
    ///
    /// **Note:** Must be less or equal to the maximum control endpoint buffer's size usually set to
    /// 128Bytes. See the feature `control-buffer-256` of the `usb_device` crate.
    const TRANSFER_SIZE: u16;
}

pub type Result<T> = core::result::Result<T, Error>;

struct Request;
impl Request {
    pub const DFU_DETACH: u8 = 0;
    pub const DFU_DNLOAD: u8 = 1;
    pub const DFU_UPLOAD: u8 = 2;
    pub const DFU_GETSTATUS: u8 = 3;
    pub const DFU_CLRSTATUS: u8 = 4;
    pub const DFU_GETSTATE: u8 = 5;
    pub const DFU_ABORT: u8 = 6;
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// File is not targeted for use by this device.
    Target = 0x01,
    /// File is for this device but fails some vendor-specific verification test.
    File = 0x02,
    /// Device is unable to write memory.
    Write = 0x03,
    /// Memory erase function failed
    Erase = 0x04,
    /// Memory erase check failed.
    CheckErased = 0x05,
    /// Program memory function failed.
    Programming = 0x06,
    /// Programmed memory failed verification.
    Verify = 0x07,
    /// Cannot program memory due to received address that is out of range.
    Address = 0x08,
    /// Received DFU_DNLOAD with wLength = 0, but device does not think it has all of the data yet.
    NotDone = 0x09,
    /// Device’s firmware is corrupt. It cannot return to run-time (non-DFU) operations.
    Firmware = 0x0A,
    /// iString indicates a vendor-specific error.
    Vendor = 0x0B,
    /// Device detected unexpected USB reset signaling.
    UsbReset = 0x0C,
    /// Device detected unexpected power on reset.
    PowerOnReset = 0x0D,
    /// Something went wrong, but the device does not know what it was.
    Unknown = 0x0E,
    /// Device stalled an unexpected request.
    /// TODO: Render that variant private to this crate
    StalledPkt = 0x0F,
}
impl From<Error> for u8 {
    fn from(err: Error) -> u8 {
        err as u8
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    AppIdle,
    /// Timestamp (+/- the poll latency) when the detach request was received.
    AppDetach(u32),
    DfuIdle,
    DfuDnloadSync,
    DfuDnloadBusy(u32),
    DfuDnloadIdle,
    DfuManifestSync,
    DfuManifest(u32),
    DfuManifestWaitReset,
    DfuUploadIdle,
    DfuError(Error),
}

impl From<State> for u8 {
    fn from(state: State) -> Self {
        match state {
            State::AppIdle => 0,
            State::AppDetach(_) => 1,
            State::DfuIdle => 2,
            State::DfuDnloadSync => 3,
            State::DfuDnloadBusy(_) => 4,
            State::DfuDnloadIdle => 5,
            State::DfuManifestSync => 6,
            State::DfuManifest(_) => 7,
            State::DfuManifestWaitReset => 8,
            State::DfuUploadIdle => 9,
            State::DfuError(_) => 10,
        }
    }
}
