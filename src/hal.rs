use crate::{GuestPageTableTrait, HostPageNum, HostPhysAddr, HostVirtAddr, HyperResult, memory::PAGE_SIZE_4K};
use gdbstub::conn::ConnectionExt;

/// The interfaces which the underlginh software(kernel or hypervisor) must implement.
pub trait HyperCraftHal: Sized {
    /// Page size.
    const PAGE_SIZE: usize = PAGE_SIZE_4K;

    /// Allocates a 4K-sized contiguous physical page, returns its physical address.
    fn alloc_page() -> Option<HostVirtAddr> {
        Self::alloc_pages(1)
    }
    /// Deallocates the given physical page.
    fn dealloc_page(va: HostVirtAddr) {
        Self::dealloc_pages(va, 1)
    }
    /// Allocates a 16K-sized & 16K-align physical page, uesd in root page table.
    #[cfg(target_arch = "riscv64")]
    fn alloc_16_page() -> Option<HostPageNum> {
        Self::alloc_pages(4)
    }
    /// Deallocates the given 16K-sized physical page.
    #[cfg(target_arch = "riscv64")]
    fn dealloc_16_page(ppn: HostPageNum) {
        Self::dealloc_pages(ppn, 4)
    }
    /// Allocates contiguous pages, returns its physical address.
    fn alloc_pages(num_pages: usize) -> Option<HostVirtAddr>;
    /// Gives back the allocated pages starts from `pa` to the page allocator.
    fn dealloc_pages(va: HostVirtAddr, num_pages: usize);
    // /// VM-Exit handler
    // fn vmexit_handler(vcpu: &mut crate::VCpu<Self>, vm_exit_info: VmExitInfo);

    /// Convert a host physical address to host virtual address.
    fn phys_to_virt(pa: HostPhysAddr) -> HostVirtAddr;
    /// Convert a host virtual address to host physical address.
    fn virt_to_phys(va: HostVirtAddr) -> HostPhysAddr;
    /// VM-Exit handler.
    #[cfg(target_arch = "x86_64")]
    fn vmexit_handler<G: GuestPageTableTrait, C: ConnectionExt>(vcpu: &mut crate::arch::VCpu<Self, G, C>) -> HyperResult;
    /// Current time in nanoseconds.
    #[cfg(target_arch = "x86_64")]
    fn current_time_nanos() -> u64;
}
