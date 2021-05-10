#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

use alloc_cortex_m::CortexMHeap;

use usbd_dfu_demo::executor;
use usbd_dfu_demo::platform;

use usb_device::prelude::*;
use usbd_dfu::mode::DFUModeClass;

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
    #[cfg(features = "need-alloc")]
    {
        // Initialize the allocator BEFORE you use it
        let start = cortex_m_rt::heap_start() as usize;
        let size = 80 * 1024; // in bytes
        unsafe { ALLOCATOR.init(start, size) };
    }

    let (usb_bus, mut _led, mut cp, mut _ctx, dfu) = platform::init();

    let mut dfu = DFUModeClass::new(&usb_bus, dfu);
    let mut usb_dev = UsbDeviceBuilder::new(&usb_bus, UsbVidPid(0x16c0, 0x27dd))
        .manufacturer("Fake company")
        .product("Serial port")
        .serial_number("TEST")
        .max_packet_size_0(64)
        //.device_class(USB_CLASS_CDC)
        //.device_sub_class(CDC_SUBCLASS_ACM)
        .device_sub_class(2)
        //.device_protocol(CDC_PROTOCOL_NONE)
        .device_protocol(0)
        .build();

    executor::block_on(async move {
        loop {
            if cp.SYST.has_wrapped() {
                dfu.poll(1);
            }

            usb_dev.poll(&mut [&mut dfu]);
        }
    })
}