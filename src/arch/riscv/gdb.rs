use super::regs::GeneralPurposeRegisters;
use crate::{GprIndex, GuestPageTableTrait, HyperCraftHal, HyperError as Error, VCpu, VM};
use gdbstub::{
    common::Signal,
    conn::ConnectionExt,
    stub::{state_machine::GdbStubStateMachine, GdbStub, SingleThreadStopReason},
    target::{
        ext::{
            base::{
                singlethread::{SingleThreadBase, SingleThreadResume, SingleThreadResumeOps},
                BaseOps,
            },
            breakpoints::{Breakpoints, BreakpointsOps, SwBreakpoint, SwBreakpointOps, WatchKind},
        },
        Target, TargetError, TargetResult,
    },
};
use gdbstub_arch::riscv::reg::RiscvCoreRegs;
use memory_addr::PhysAddr;
use page_table::{
    riscv::Sv39PageTable, MappingFlags, PageSize, PagingIf, PagingIfCallback, PagingResult,
};

impl<H, G, C> VM<H, G, C>
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
                                panic!("Shouldn't be here");
                            }
                        } else {
                            panic!("Shouldn't be here");
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

impl<H, G, C> VM<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn get_page(&mut self, addr: usize) -> PagingResult<(PhysAddr, MappingFlags, PageSize)> {
        let vcpu = self.get_current_vcpu();
        let root_paddr = vcpu.get_page_table_root();
        let mut paging = PagingIfCallback::new();
        paging.set_callback(|guest_addr| {
            let host_addr = self.gpt.translate(guest_addr.into()).unwrap();
            H::phys_to_virt(host_addr).into()
        });
        Sv39PageTable::create_from(root_paddr.into(), paging).query((addr as usize).into())
    }
}

impl<H, G, C> Target for VM<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    type Arch = gdbstub_arch::riscv::Riscv64;
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

impl<H, G, C> SingleThreadBase for VM<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn read_registers(&mut self, regs: &mut RiscvCoreRegs<u64>) -> TargetResult<(), Self> {
        let vcpu = self.get_current_vcpu();
        let mut gprs = GeneralPurposeRegisters::default();
        vcpu.save_gprs(&mut gprs);
        for i in 0..32 {
            let reg = gprs.reg(GprIndex::from_raw(i as u32).unwrap());
            regs.x[i] = reg as u64;
        }
        regs.pc = vcpu.get_pc() as u64;
        Ok(())
    }

    fn write_registers(&mut self, regs: &RiscvCoreRegs<u64>) -> TargetResult<(), Self> {
        let vcpu = self.get_current_vcpu();
        let mut gprs = GeneralPurposeRegisters::default();
        for i in 0..32 {
            gprs.set_reg(GprIndex::from_raw(i as u32).unwrap(), regs.x[i] as usize);
        }
        vcpu.restore_gprs(&mut gprs);
        vcpu.set_pc(regs.pc as usize);
        Ok(())
    }

    fn read_addrs(&mut self, start_addr: u64, data: &mut [u8]) -> TargetResult<usize, Self> {
        let vcpu = self.get_current_vcpu();
        let (mut addr, buf, mut count) = (start_addr as usize, data.as_mut_ptr(), data.len());
        if vcpu.get_page_table_root() != 0 {
            let (paddr, _, size) = self.get_page(addr).map_err(|_| TargetError::Errno(1))?;
            addr = paddr.as_usize();
            count = count.min(size as usize);
        }
        self.gpt
            .read_guest_phys_addrs(addr, buf, count)
            .map_err(|_| TargetError::Errno(1))
    }

    fn write_addrs(&mut self, start_addr: u64, data: &[u8]) -> TargetResult<(), Self> {
        let vcpu = self.get_current_vcpu();
        let (mut addr, buf, mut count) = (start_addr as usize, data.as_ptr(), data.len());
        if vcpu.get_page_table_root() != 0 {
            let (paddr, _, size) = self.get_page(addr).map_err(|_| TargetError::Errno(1))?;
            addr = paddr.as_usize();
            count = count.min(size as usize);
        }
        self.gpt
            .write_guest_phys_addrs(addr, buf, count)
            .map_err(|_| TargetError::Errno(1))
    }

    #[inline(always)]
    fn support_resume(&mut self) -> Option<SingleThreadResumeOps<'_, Self>> {
        Some(self)
    }
}

impl<H, G, C> SingleThreadResume for VM<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn resume(&mut self, _signal: Option<Signal>) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl<H, G, C> Breakpoints for VM<H, G, C>
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

impl<H, G, C> SwBreakpoint for VM<H, G, C>
where
    H: HyperCraftHal,
    G: GuestPageTableTrait,
    C: ConnectionExt,
{
    fn add_sw_breakpoint(&mut self, bp_addr: u64, _kind: usize) -> TargetResult<bool, Self> {
        let vcpu = self.get_current_vcpu();
        let mut addr = bp_addr as usize;
        if vcpu.get_page_table_root() != 0 {
            match self.get_page(addr) {
                Ok((paddr, _, _)) => addr = paddr.as_usize(),
                Err(_) => return Ok(false),
            }
        }
        let mut inst = [0; 2];
        let ebreak = [2, 144];
        if self
            .gpt
            .read_guest_phys_addrs(addr, inst.as_mut_ptr(), inst.len())
            .is_err()
        {
            return Ok(false);
        }
        if self
            .gpt
            .write_guest_phys_addrs(addr, ebreak.as_ptr(), ebreak.len())
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
                self.gpt
                    .write_guest_phys_addrs(addr, inst.as_ptr(), inst.len())
                    .map_err(|_| TargetError::Errno(1))?;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}
