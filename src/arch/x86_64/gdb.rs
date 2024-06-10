use crate::{
    GprIndex, GuestPageTableTrait, HyperCraftHal, HyperError as Error, NestedPageTable, VCpu,
};
use gdbstub::{
    common::Signal,
    conn::ConnectionExt,
    stub::{state_machine::GdbStubStateMachine, GdbStub, SingleThreadStopReason},
    target::{
        ext::{
            base::{
                singlethread::{
                    SingleThreadBase, SingleThreadResume, SingleThreadResumeOps,
                    SingleThreadSingleStep, SingleThreadSingleStepOps,
                },
                BaseOps,
            },
            breakpoints::{Breakpoints, BreakpointsOps, SwBreakpoint, SwBreakpointOps},
        },
        Target, TargetError, TargetResult,
    },
};
use gdbstub_arch::x86::reg::X86_64CoreRegs;
use memory_addr::PhysAddr;
use page_table::{
    x86_64::X64PageTable, MappingFlags, PageSize, PagingIf, PagingIfCallback, PagingResult,
};
use x86_64::registers::control::Cr0Flags;

impl<H, G, C> VCpu<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    /// Initialize gdbserver with connection
    pub fn gdbserver_init(&mut self, conn: C) {
        let gdbstub = GdbStub::new(conn).run_state_machine(self);
        if let Ok(gdbstub) = gdbstub {
            self.gdbstub = Some(gdbstub)
        }
    }

    pub(crate) fn gdbserver_loop(&mut self) {
        if let Some(gdbstub) = self.gdbstub.take() {
            let mut gdb = gdbstub;
            loop {
                gdb = match gdb {
                    GdbStubStateMachine::Idle(mut gdb_inner) => {
                        if let Ok(byte) = gdb_inner.borrow_conn().read() {
                            if let Ok(x) = gdb_inner.incoming_data(self, byte) {
                                x
                            } else {
                                panic!("Shouldn't be here1");
                            }
                        } else {
                            panic!("Shouldn't be here2");
                        }
                    }
                    GdbStubStateMachine::Running(_) => break,
                    GdbStubStateMachine::CtrlCInterrupt(_) => {
                        debug!("Enter GdbStubStateMachine::CtrlCInterrupt");
                        break;
                    }
                    GdbStubStateMachine::Disconnected(_) => {
                        debug!("Enter GdbStubStateMachine::Disconnected");
                        break;
                    }
                }
            }
            self.gdbstub = Some(gdb);
        }
    }

    pub(crate) fn gdbserver_report(&mut self) {
        let mut gdb = self.gdbstub.take().unwrap();
        let reason = SingleThreadStopReason::DoneStep;

        if let GdbStubStateMachine::Running(gdb_inner) = gdb {
            match gdb_inner.report_stop(self, reason) {
                Ok(gdb_state) => gdb = gdb_state,
                Err(_) => {
                    debug!("Report stop error!");
                    return;
                }
            }
        }
        self.gdbstub = Some(gdb);
        self.gdbserver_loop();
    }
}

impl<H, G, C> VCpu<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn get_page(&self, addr: usize) -> PagingResult<(PhysAddr, MappingFlags, PageSize)> {
        let mut paging = PagingIfCallback::new();
        paging.set_callback(|guest_addr| {
            let host_addr = self.ept.translate(guest_addr.into()).unwrap();
            H::phys_to_virt(host_addr).into()
        });
        X64PageTable::create_from(self.cr3().into(), paging).query((addr as usize).into())
    }
}

impl<H, G, C> Target for VCpu<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    type Arch = gdbstub_arch::x86::X86_64_SSE;
    type Error = Error;

    #[inline(always)]
    fn base_ops(&mut self) -> BaseOps<'_, Self::Arch, Self::Error> {
        BaseOps::SingleThread(self)
    }

    #[inline(always)]
    fn support_breakpoints(&mut self) -> Option<BreakpointsOps<Self>> {
        Some(self)
    }
}

impl<H, G, C> SingleThreadBase for VCpu<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn read_registers(&mut self, regs: &mut X86_64CoreRegs) -> TargetResult<(), Self> {
        let gpr = self.regs();
        regs.regs = [
            gpr.rax,
            gpr.rbx,
            gpr.rcx,
            gpr.rdx,
            gpr.rsi,
            gpr.rdi,
            gpr.rbp,
            self.stack_pointer() as u64,
            gpr.r8,
            gpr.r9,
            gpr.r10,
            gpr.r11,
            gpr.r12,
            gpr.r13,
            gpr.r14,
            gpr.r15,
        ];
        regs.rip = self.rip() as u64;
        Ok(())
    }

    fn write_registers(&mut self, regs: &X86_64CoreRegs) -> TargetResult<(), Self> {
        let gpr = self.regs_mut();
        gpr.rax = regs.regs[0];
        gpr.rbx = regs.regs[1];
        gpr.rcx = regs.regs[2];
        gpr.rdx = regs.regs[3];
        gpr.rsi = regs.regs[4];
        gpr.rdi = regs.regs[5];
        gpr.rbp = regs.regs[6];
        gpr.r8 = regs.regs[8];
        gpr.r9 = regs.regs[9];
        gpr.r10 = regs.regs[10];
        gpr.r11 = regs.regs[11];
        gpr.r12 = regs.regs[12];
        gpr.r13 = regs.regs[13];
        gpr.r14 = regs.regs[14];
        gpr.r15 = regs.regs[15];
        self.set_stack_pointer(regs.regs[7] as usize);
        self.set_rip(regs.rip as usize);
        Ok(())
    }

    fn read_addrs(&mut self, start_addr: u64, data: &mut [u8]) -> TargetResult<usize, Self> {
        let (mut addr, buf, mut count) = (start_addr as usize, data.as_mut_ptr(), data.len());
        let cr0 = Cr0Flags::from_bits_truncate(self.cr0() as u64);
        if cr0.contains(Cr0Flags::PAGING) {
            let (paddr, _, size) = self.get_page(addr).map_err(|_| TargetError::Errno(1))?;
            addr = paddr.as_usize();
            count = count.min(size as usize);
        }
        self.ept
            .read_guest_phys_addrs(addr, buf, count)
            .map_err(|_| TargetError::Errno(1))
    }

    fn write_addrs(&mut self, start_addr: u64, data: &[u8]) -> TargetResult<(), Self> {
        let (mut addr, buf, mut count) = (start_addr as usize, data.as_ptr(), data.len());
        let cr0 = Cr0Flags::from_bits_truncate(self.cr0() as u64);
        if cr0.contains(Cr0Flags::PAGING) {
            let (paddr, _, size) = self.get_page(addr).map_err(|_| TargetError::Errno(1))?;
            addr = paddr.as_usize();
            count = count.min(size as usize);
        }
        self.ept
            .write_guest_phys_addrs(addr, buf, count)
            .map_err(|_| TargetError::Errno(1))
    }

    #[inline(always)]
    fn support_resume(&mut self) -> Option<SingleThreadResumeOps<'_, Self>> {
        Some(self)
    }
}

impl<H, G, C> SingleThreadResume for VCpu<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn resume(&mut self, _signal: Option<Signal>) -> Result<(), Self::Error> {
        Ok(())
    }

    #[inline(always)]
    fn support_single_step(&mut self) -> Option<SingleThreadSingleStepOps<Self>> {
        Some(self)
    }
}

impl<H, G, C> SingleThreadSingleStep for VCpu<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn step(&mut self, _signal: Option<Signal>) -> Result<(), Self::Error> {
        self.set_monitor_trap_flag(true)
    }
}

impl<H, G, C> Breakpoints for VCpu<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    #[inline(always)]
    fn support_sw_breakpoint(&mut self) -> Option<SwBreakpointOps<Self>> {
        Some(self)
    }
}

impl<H, G, C> SwBreakpoint for VCpu<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn add_sw_breakpoint(&mut self, bp_addr: u64, _kind: usize) -> TargetResult<bool, Self> {
        let mut addr = bp_addr as usize;
        let cr0 = Cr0Flags::from_bits_truncate(self.cr0() as u64);
        if cr0.contains(Cr0Flags::PAGING) {
            match self.get_page(self.cr3()) {
                Ok((paddr, _, _)) => addr = paddr.as_usize(),
                Err(_) => return Ok(false),
            }
        }
        let mut inst = [0; 1];
        let int3 = [0xcc];
        if self
            .ept
            .read_guest_phys_addrs(addr, inst.as_mut_ptr(), inst.len())
            .is_err()
        {
            return Ok(false);
        }
        if self
            .ept
            .write_guest_phys_addrs(addr, int3.as_ptr(), int3.len())
            .is_err()
        {
            return Ok(false);
        }
        self.breakpoints.insert(bp_addr as usize, (addr, inst));
        Ok(true)
    }

    fn remove_sw_breakpoint(&mut self, bp_addr: u64, _kind: usize) -> TargetResult<bool, Self> {
        match self.breakpoints.remove(&(bp_addr as usize)) {
            Some((addr, inst)) => {
                self.ept
                    .write_guest_phys_addrs(addr, inst.as_ptr(), inst.len())
                    .map_err(|_| TargetError::Errno(1))?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}
