use super::regs::GeneralPurposeRegisters;
use crate::{GprIndex, GuestPageTableTrait, HyperCraftHal, HyperError as Error, VCpu, VM};
use gdbstub::{
    conn::ConnectionExt,
    stub::{state_machine::GdbStubStateMachine, GdbStub},
    target::{self, ext::base::singlethread::SingleThreadBase, Target, TargetError, TargetResult},
};
use gdbstub_arch::riscv::reg::RiscvCoreRegs;

impl<H: HyperCraftHal, G: GuestPageTableTrait, C: ConnectionExt> VM<H, G, C> {
    pub(crate) fn init_gdbserver(&mut self, conn: C) {
        let gdbstub = GdbStub::new(conn);
        self.gdbstub = Some(gdbstub);
    }

    pub(crate) fn start_gdbserver(&mut self) {
        if let Some(gdbstub) = self.gdbstub.take() {
            if let Ok(gdbstub) = gdbstub.run_state_machine(self) {
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
                        GdbStubStateMachine::Disconnected(_gdb_inner) => {
                            debug!("Enter GdbStubStateMachine::Disconnected");
                            break;
                        }
                    }
                }
            }
        }
    }
}

impl<H: HyperCraftHal, C: ConnectionExt, G: GuestPageTableTrait> Target for VM<H, G, C> {
    type Arch = gdbstub_arch::riscv::Riscv64;
    type Error = Error;

    #[inline(always)]
    fn base_ops(&mut self) -> target::ext::base::BaseOps<'_, Self::Arch, Self::Error> {
        target::ext::base::BaseOps::SingleThread(self)
    }

    fn guard_rail_implicit_sw_breakpoints(&self) -> bool {
        true
    }
}

impl<H: HyperCraftHal, G: GuestPageTableTrait, C: ConnectionExt> SingleThreadBase for VM<H, G, C> {
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
        self.gpt
            .translate(start_addr as usize)
            .map_err(|_| TargetError::Errno(1))?;
        self.vm_pages
            .copy_from_guest(data, start_addr as usize)
            .map_err(|_| TargetError::Errno(1))
    }

    fn write_addrs(&mut self, start_addr: u64, data: &[u8]) -> TargetResult<(), Self> {
        self.gpt
            .translate(start_addr as usize)
            .map_err(|_| TargetError::Errno(1))?;
        match self.vm_pages.copy_to_guest(start_addr as usize, data) {
            Ok(_) => Ok(()),
            Err(_) => Err(TargetError::Errno(1)),
        }
    }
}
