use crate::platform;
use cortex_m_rt::exception;

#[link_section = ".uninit"]
pub static mut ERROR: core::mem::MaybeUninit<LastPanicMessage> = core::mem::MaybeUninit::uninit();

pub struct LastPanicMessage {
    pub len: usize,
    pub buffer: [u8; 1024],
}
impl core::fmt::Write for LastPanicMessage {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let len = core::cmp::min(s.len(), self.buffer.len());
        let start = self.len;
        let end = start + len;
        self.buffer[start..end].copy_from_slice(&s.as_bytes()[..len]);
        self.len = end;
        Ok(())
    }
}

#[panic_handler]
fn on_panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        use core::fmt::Write;
        let err = ERROR.assume_init_mut();
        let _ = write!(err, "Woops that's a hard one");
        platform::reset();
    }
}

#[cfg(not(feature = "bootloader"))]
#[exception]
#[allow(non_snake_case)]
fn HardFault(ef: &cortex_m_rt::ExceptionFrame) -> ! {
    panic!("Hardfault: {:#?}", ef)
}

#[cfg(feature = "bootloader")]
#[exception]
#[allow(non_snake_case)]
fn HardFault(_ef: &cortex_m_rt::ExceptionFrame) -> ! {
    panic!("Hardfault");
}

#[exception]
#[allow(non_snake_case)]
unsafe fn DefaultHandler(irqn: i16) {
    panic!("DefaultHandler: {}", irqn);
}
