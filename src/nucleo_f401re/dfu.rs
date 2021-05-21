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

use super::{APPLICATION_REGION_START, FLASH_END, MANIFEST_REGION_START};
const APPLICATION_LENGTH: usize = MANIFEST_REGION_START - APPLICATION_REGION_START;

type Hash = [u8; HASH_LENGTH];

#[repr(C)]
pub struct ApplicationRef(&'static [u8]);
impl ApplicationRef {
    pub fn get_with_length(length: usize) -> Self {
        unsafe {
            Self(core::slice::from_raw_parts(
                APPLICATION_REGION_START as *const u8,
                length,
            ))
        }
    }
    fn get() -> Self {
        let manifest = Manifest::get();
        let length = usize::min(APPLICATION_LENGTH, manifest.length);
        Self::get_with_length(length)
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

#[repr(C)]
#[derive(Debug)]
pub struct Manifest {
    pub length: usize,
    pub hash: [u8; HASH_LENGTH],
}
impl Manifest {
    fn get() -> &'static Manifest {
        unsafe { &*(MANIFEST_REGION_START as *const Manifest) }
    }
}

#[cfg(feature = "application")]
mod runtime {
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

#[path = "dfu_mode.rs"]
mod mode;
pub use mode::*;
