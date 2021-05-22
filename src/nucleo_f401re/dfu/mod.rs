use super::MANIFEST_REGION_START;

macro_rules! impl_capabilities {
    ($name:ty) => {
        impl ::usbd_dfu::Capabilities for $name {
            const CAN_UPLOAD: bool = true;
            const CAN_DOWNLOAD: bool = true;
            const IS_MANIFESTATION_TOLERANT: bool = true;
            const WILL_DETACH: bool = false;
            const DETACH_TIMEOUT: u16 = 50;
            const TRANSFER_SIZE: u16 = 128;
        }
    };
}

#[cfg(not(feature = "use-sha256"))]
const HASH_LENGTH: usize = 20;
#[cfg(feature = "use-sha256")]
const HASH_LENGTH: usize = 32;

type Hash = [u8; HASH_LENGTH];

#[repr(C)]
#[derive(Debug)]
pub struct Manifest {
    pub length: usize,
    pub hash: Hash,
}
impl Manifest {
    fn get() -> &'static Manifest {
        unsafe { &*(MANIFEST_REGION_START as *const Manifest) }
    }
}

#[cfg(feature = "application")]
pub mod runtime {
    pub struct DFURuntimeImpl;
    impl DFURuntimeImpl {
        pub async fn read_manifest(&self) -> &'static super::Manifest {
            super::Manifest::get()
        }
    }
    impl_capabilities!(DFURuntimeImpl);
    impl usbd_dfu::runtime::DeviceFirmwareUpgrade for DFURuntimeImpl {
        fn on_reset(&mut self) {
            super::super::reset();
        }

        fn on_detach_request(&mut self, _timeout_ms: u16) {}
    }
}

#[cfg(feature = "bootloader")]
pub mod mode;
