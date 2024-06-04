use crate::{GprIndex, GuestPageTableTrait, HyperCraftHal, HyperError as Error, VCpu};
use gdbstub::{
    conn::ConnectionExt,
    stub::{state_machine::GdbStubStateMachine, GdbStub},
    target::{ext::base::{singlethread::SingleThreadBase, BaseOps}, Target, TargetError, TargetResult},
};
use gdbstub_arch::x86::reg::X86_64CoreRegs;

impl<H: HyperCraftHal, C: ConnectionExt> VCpu<H, C> {
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
                    GdbStubStateMachine::Running(_) => {
                        debug!("Enter GdbStubStateMachine::Running");
                        break;
                    }
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
}

impl<H: HyperCraftHal, C: ConnectionExt> Target for VCpu<H, C> {
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

impl<H: HyperCraftHal, C: ConnectionExt> SingleThreadBase for VCpu<H, C> {
    fn read_registers(&mut self, regs: &mut X86_64CoreRegs) -> TargetResult<(), Self> {
        let gpr = self.regs();
        regs.regs = [
            gpr.rax, gpr.rbx, gpr.rcx, gpr.rdx, gpr.rsi, gpr.rdi, gpr.rbp, self.stack_pointer() as u64,
            gpr.r8, gpr.r9, gpr.r10, gpr.r11, gpr.r12, gpr.r13, gpr.r14, gpr.r15,
        ];
        regs.rip = self.rip() as u64;
        Ok(())
    }

    fn write_registers(&mut self, _regs: &X86_64CoreRegs) -> TargetResult<(), Self> {
        Ok(())
    }

    fn read_addrs(&mut self, _start_addr: u64, _data: &mut [u8]) -> TargetResult<usize, Self> {
        Err(TargetError::Errno(1))
    }

    fn write_addrs(&mut self, _start_addr: u64, _data: &[u8]) -> TargetResult<(), Self> {
        Err(TargetError::Errno(1))
    }
}
