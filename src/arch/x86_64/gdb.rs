use crate::{GprIndex, GuestPageTableTrait, HyperCraftHal, HyperError as Error, VCpu};
use gdbstub::{
    common::Signal,
    conn::ConnectionExt,
    stub::{state_machine::GdbStubStateMachine, GdbStub, SingleThreadStopReason},
    target::{
        ext::base::{
            singlethread::{
                SingleThreadBase, SingleThreadResume, SingleThreadResumeOps,
                SingleThreadSingleStep, SingleThreadSingleStepOps,
            },
            BaseOps,
        },
        Target, TargetError, TargetResult,
    },
};
use gdbstub_arch::x86::reg::X86_64CoreRegs;

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

    /// Gdbserver report stop reason
    pub fn gdbserver_report(&mut self) {
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

    fn guard_rail_implicit_sw_breakpoints(&self) -> bool {
        true
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
        self.ept
            .read_guest_phys_addrs(start_addr as usize, data)
            .map_err(|_| TargetError::Errno(1))
    }

    fn write_addrs(&mut self, start_addr: u64, data: &[u8]) -> TargetResult<(), Self> {
        match self.ept.write_guest_phys_addrs(start_addr as usize, data) {
            Ok(_) => Ok(()),
            Err(_) => Err(TargetError::Errno(1)),
        }
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
