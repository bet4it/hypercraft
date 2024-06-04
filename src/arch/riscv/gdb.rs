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
            breakpoints::{Breakpoints, SwBreakpoint, WatchKind},
        },
        Target, TargetError, TargetResult,
    },
};
use gdbstub_arch::riscv::reg::RiscvCoreRegs;

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

    fn guard_rail_implicit_sw_breakpoints(&self) -> bool {
        true
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
        self.gpt.read_guest_phys_addrs(start_addr as usize, data).map_err(|_| TargetError::Errno(1))
    }

    fn write_addrs(&mut self, start_addr: u64, data: &[u8]) -> TargetResult<(), Self> {
        match self.gpt.write_guest_phys_addrs(start_addr as usize, data) {
            Ok(_) => Ok(()),
            Err(_) => Err(TargetError::Errno(1)),
        }
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
