#![no_std]
#![feature(maybe_uninit_ref)]
#![feature(panic_info_message)]
#![feature(const_raw_ptr_to_usize_cast)]

extern crate alloc;

pub mod executor;
pub mod trace;

#[cfg(feature = "duet3d")]
pub mod duet3d;
#[cfg(feature = "duet3d")]
pub use duet3d as platform;
#[cfg(feature = "duet3d")]
pub use duet3d::dbgprint;

//usb's not yet supported on stm32l4x5
//#[cfg(feature = "disco-l475")]
//#[macro_use]
//mod disco_l475;
//#[cfg(feature = "disco-l475")]
//pub use disco_l475 as platform;

#[cfg(feature = "nucleo-f401re")]
#[macro_use]
pub mod nucleo_f401re;
#[cfg(feature = "nucleo-f401re")]
pub use nucleo_f401re as platform;
