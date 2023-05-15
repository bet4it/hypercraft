use super::{traps, HyperCallMsg, RiscvCsrTrait, CSR};
use crate::{
    vcpus::VM_CPUS_MAX, GuestPageTableTrait, HyperCraftHal, HyperError, HyperResult, VmCpus,
    VmExitInfo,
};

/// A VM that is being run.
pub struct VM<H: HyperCraftHal, G: GuestPageTableTrait> {
    vcpus: VmCpus<H>,
    gpt: G,
}

impl<H: HyperCraftHal, G: GuestPageTableTrait> VM<H, G> {
    pub fn new(vcpus: VmCpus<H>, gpt: G) -> HyperResult<Self> {
        Ok(Self { vcpus, gpt })
    }

    pub fn init_vcpus(&mut self) {
        for vcpu_id in 0..VM_CPUS_MAX {
            let vcpu = self.vcpus.get_vcpu(vcpu_id).unwrap();
            vcpu.init_page_map(self.gpt.token());
        }
    }

    /// Run the host VM's vCPU with ID `vcpu_id`. Does not return.
    pub fn run(&mut self, vcpu_id: usize) {
        let vcpu = self.vcpus.get_vcpu(vcpu_id).unwrap();

        // Set htimedelta for ALL VCPU'f of the VM.
        loop {
            let vm_exit_info = vcpu.run();

            H::vmexit_handler(vcpu, vm_exit_info);

            if let VmExitInfo::Ecall(sbi_msg) = vm_exit_info {
                if let Some(info) = sbi_msg {
                    if let HyperCallMsg::SetTimer(_) = info {
                        // Clear guest timer interrupt
                        // hvip.read_and_clear_bits(traps::interrupt::VIRTUAL_SUPERVISOR_TIMER);
                        CSR.hvip
                            .read_and_clear_bits(traps::interrupt::VIRTUAL_SUPERVISOR_TIMER);
                        //  Enable host timer interrupt
                        CSR.sie
                            .read_and_set_bits(traps::interrupt::SUPERVISOR_TIMER);
                    }
                }
            }
        }
    }
}

// Privaie function
impl<H: HyperCraftHal, G: GuestPageTableTrait> VM<H, G> {}
