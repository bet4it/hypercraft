use bit_field::BitField;
use core::marker::PhantomData;

use crate::arch::memory::PhysFrame;
use crate::{GuestPhysAddr, HostPhysAddr, HostVirtAddr, HyperCraftHal};
use crate::{HyperError, HyperResult};

/// VMCS/VMXON region in 4K size. (SDM Vol. 3C, Section 24.2)
#[derive(Debug)]
pub struct VmxRegion<H: HyperCraftHal> {
    frame: PhysFrame<H>,
}

impl<H: HyperCraftHal> VmxRegion<H> {
    pub const unsafe fn uninit() -> Self {
        Self {
            frame: PhysFrame::uninit(),
        }
    }

    pub fn new(revision_id: u32, shadow_indicator: bool) -> HyperResult<Self> {
        let frame = PhysFrame::alloc_zero()?;
        unsafe {
            (*(frame.as_mut_ptr() as *mut u32))
                .set_bits(0..=30, revision_id)
                .set_bit(31, shadow_indicator);
        }
        Ok(Self { frame })
    }

    pub fn phys_addr(&self) -> HostPhysAddr {
        self.frame.start_paddr()
    }

    pub fn virt_addr(&self) -> HostVirtAddr {
        self.frame.as_mut_ptr() as usize
    }
}

// (SDM Vol. 3C, Section 25.6.4)
// The VM-execution control fields include the 64-bit physical addresses of I/O bitmaps A and B (each of which are 4 KBytes in size).
// I/O bitmap A contains one bit for each I/O port in the range 0000H through 7FFFH;
// I/O bitmap B contains bits for ports in the range 8000H through FFFFH.
#[derive(Debug)]
pub struct IOBitmap<H: HyperCraftHal> {
    io_bitmap_a_frame: PhysFrame<H>,
    io_bitmap_b_frame: PhysFrame<H>,
}

impl<H: HyperCraftHal> IOBitmap<H> {
    pub fn passthrough_all() -> HyperResult<Self> {
        Ok(Self {
            io_bitmap_a_frame: PhysFrame::alloc_zero()?,
            io_bitmap_b_frame: PhysFrame::alloc_zero()?,
        })
    }

    #[allow(unused)]
    pub fn intercept_all() -> HyperResult<Self> {
        let mut io_bitmap_a_frame = PhysFrame::alloc()?;
        io_bitmap_a_frame.fill(u8::MAX);
        let mut io_bitmap_b_frame = PhysFrame::alloc()?;
        io_bitmap_b_frame.fill(u8::MAX);
        Ok(Self {
            io_bitmap_a_frame,
            io_bitmap_b_frame,
        })
    }

    pub fn phys_addr(&self) -> (HostPhysAddr, HostPhysAddr) {
        (
            self.io_bitmap_a_frame.start_paddr(),
            self.io_bitmap_b_frame.start_paddr(),
        )
    }

    // Execution of an I/O instruction causes a VM exit
    // if any bit in the I/O bitmaps corresponding to a port it accesses is 1.
    // See Section 26.1.3 for details.
    pub fn set_intercept(&mut self, port: u32, intercept: bool) {
        let (port, io_bit_map_frame) = if port <= 0x7fff {
            (port, &mut self.io_bitmap_a_frame)
        } else {
            (port - 0x8000, &mut self.io_bitmap_b_frame)
        };
        let bitmap =
            unsafe { core::slice::from_raw_parts_mut(io_bit_map_frame.as_mut_ptr(), 1024) };
        let byte = (port / 8) as usize;
        let bits = port % 8;
        if intercept {
            bitmap[byte] |= 1 << bits;
        } else {
            bitmap[byte] &= !(1 << bits);
        }
    }

    pub fn set_intercept_of_range(&mut self, port_base: u32, count: u32, intercept: bool) {
        for port in port_base..port_base + count {
            self.set_intercept(port, intercept)
        }
    }
}

#[derive(Debug)]
pub struct MsrBitmap<H: HyperCraftHal> {
    frame: PhysFrame<H>,
}

impl<H: HyperCraftHal> MsrBitmap<H> {
    pub fn passthrough_all() -> HyperResult<Self> {
        Ok(Self {
            frame: PhysFrame::alloc_zero()?,
        })
    }

    #[allow(unused)]
    pub fn intercept_all() -> HyperResult<Self> {
        let mut frame = PhysFrame::alloc()?;
        frame.fill(u8::MAX);
        Ok(Self { frame })
    }

    pub fn phys_addr(&self) -> HostPhysAddr {
        self.frame.start_paddr()
    }

    fn set_intercept(&mut self, msr: u32, is_write: bool, intercept: bool) {
        let offset = if msr <= 0x1fff {
            if !is_write {
                0 // Read bitmap for low MSRs (0x0000_0000..0x0000_1FFF)
            } else {
                2 // Write bitmap for low MSRs (0x0000_0000..0x0000_1FFF)
            }
        } else if (0xc000_0000..=0xc000_1fff).contains(&msr) {
            if !is_write {
                1 // Read bitmap for high MSRs (0xC000_0000..0xC000_1FFF)
            } else {
                3 // Write bitmap for high MSRs (0xC000_0000..0xC000_1FFF)
            }
        } else {
            unreachable!()
        } * 1024;
        let bitmap =
            unsafe { core::slice::from_raw_parts_mut(self.frame.as_mut_ptr().add(offset), 1024) };
        let msr = msr & 0x1fff;
        let byte = (msr / 8) as usize;
        let bits = msr % 8;
        if intercept {
            bitmap[byte] |= 1 << bits;
        } else {
            bitmap[byte] &= !(1 << bits);
        }
    }

    pub fn set_read_intercept(&mut self, msr: u32, intercept: bool) {
        self.set_intercept(msr, false, intercept);
    }

    pub fn set_write_intercept(&mut self, msr: u32, intercept: bool) {
        self.set_intercept(msr, true, intercept);
    }
}
