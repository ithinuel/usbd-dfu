#![no_std]
#![no_main]
#![feature(maybe_uninit_ref)]
#![feature(alloc_error_handler)]

use usbd_dfu_demo::dbgprint;
use usbd_dfu_demo::executor;
use usbd_dfu_demo::platform;

use alloc_cortex_m::CortexMHeap;
use embedded_hal::digital::v2::OutputPin;

use usb_device::prelude::*;
use usbd_dfu::runtime::DFURuntimeClass;
use usbd_serial::{SerialPort, /* CDC_SUBCLASS_ACM,*/ USB_CLASS_CDC};

#[global_allocator]
static ALLOCATOR: CortexMHeap = CortexMHeap::empty();

#[alloc_error_handler]
fn oom(layout: core::alloc::Layout) -> ! {
    panic!(
        "oom with: {:?}\r\nused: {}\r\nfree: {}\r\n",
        layout,
        ALLOCATOR.used(),
        ALLOCATOR.free()
    );
}

#[cortex_m_rt::entry]
fn main() -> ! {
    let (usb_bus, mut led, mut cp, dfu) = platform::init();

    // Initialize the allocator BEFORE you use it
    let start = cortex_m_rt::heap_start() as usize;
    let size = 80 * 1024; // in bytes
    unsafe { ALLOCATOR.init(start, size) };

    let mut serial = SerialPort::new_with_store(
        &usb_bus,
        unsafe { core::mem::MaybeUninit::<[u8; 128]>::uninit().assume_init() },
        unsafe { core::mem::MaybeUninit::<[u8; 1024]>::uninit().assume_init() },
    );
    let mut dfu = DFURuntimeClass::new(&usb_bus, dfu);
    let mut usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x16c0, 0x27dd))
        .manufacturer("Fake company")
        .product("Serial port")
        .serial_number("TEST")
        .max_packet_size_0(64)
        .device_class(USB_CLASS_CDC)
        //.device_sub_class(CDC_SUBCLASS_ACM)
        .device_sub_class(2)
        //.device_protocol(CDC_PROTOCOL_NONE)
        .device_protocol(0)
        .build();

    let mut timestamp = 0u64;

    executor::block_on(async move {
        loop {
            if cp.SYST.has_wrapped() {
                dfu.poll(1);
                if usb_dev.state() == UsbDeviceState::Configured {
                    timestamp += 1;
                } else {
                    timestamp = 0;
                }
            }

            usb_dev.poll(&mut [&mut serial, &mut dfu]);
            let mut buf = [0u8; 256];

            let mut count = match serial.read(&mut buf) {
                Ok(count) => {
                    let _ = led.set_low(); // Turn on

                    // Echo back in upper case
                    buf.iter_mut().take(count).for_each(|c| {
                        if let 0x61..=0x7a = *c {
                            *c &= !0x20;
                        }
                    });
                    count
                }
                _ => 0,
            };

            if let &[0x0D, ..] = &buf {
                // read flash desc
                let v = dfu.handler().read().await;
                dbgprint!("{:?}", v);
            }

            // transfers previous error to trace buffer
            if timestamp >= 5 && usb_dev.state() == UsbDeviceState::Configured {
                unsafe {
                    let err = usbd_dfu_demo::trace::ERROR.assume_init_mut();
                    if err.len > 0 && err.len < err.buffer.len() {
                        dbgprint!(
                            "{} \"{}\"\r\n",
                            err.len,
                            core::str::from_utf8_unchecked(&err.buffer[..err.len])
                        );
                    } else {
                        dbgprint!("All clear, you're good to go.\n");
                    }
                    err.len = 0;
                };
            }
            // transfers trace buffer to output buffer
            platform::consume_debug(|dbg| {
                let len = core::cmp::min(dbg.len(), buf.len() - count);
                buf[count..count + len].copy_from_slice(&dbg[..len]);
                count += len;
                len
            });
            if count == 0 {
                continue;
            }

            // transfers trace buffer to output buffer
            let mut wr_ptr = &buf[..count];
            while !wr_ptr.is_empty() {
                let _ = serial.write(wr_ptr).map(|len| {
                    wr_ptr = &wr_ptr[len..];
                });
            }

            let _ = led.set_high(); // Turn off
        }
    });
    unreachable!();
}
