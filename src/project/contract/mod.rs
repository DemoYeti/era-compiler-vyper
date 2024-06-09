//!
//! The contract.
//!

pub mod eravm_assembly;
pub mod llvm_ir;
pub mod metadata;
pub mod vyper;

use serde::Deserialize;
use serde::Serialize;

use crate::build::contract::Contract as ContractBuild;
use crate::warning_type::WarningType;

use self::eravm_assembly::Contract as EraVMAssemblyContract;
use self::llvm_ir::Contract as LLVMIRContract;
use self::vyper::Contract as VyperContract;

///
/// The contract.
///
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum Contract {
    /// The Vyper contract.
    Vyper(VyperContract),
    /// The LLVM IR contract.
    LLVMIR(LLVMIRContract),
    /// The LLVM IR contract.
    ZKASM(EraVMAssemblyContract),
}

impl From<VyperContract> for Contract {
    fn from(inner: VyperContract) -> Self {
        Self::Vyper(inner)
    }
}

impl From<LLVMIRContract> for Contract {
    fn from(inner: LLVMIRContract) -> Self {
        Self::LLVMIR(inner)
    }
}

impl From<EraVMAssemblyContract> for Contract {
    fn from(inner: EraVMAssemblyContract) -> Self {
        Self::ZKASM(inner)
    }
}

impl Contract {
    ///
    /// Compiles the contract, returning the build.
    ///
    pub fn compile(
        self,
        contract_path: &str,
        source_code_hash: Option<[u8; era_compiler_common::BYTE_LENGTH_FIELD]>,
        evm_version: Option<era_compiler_common::EVMVersion>,
        optimizer_settings: era_compiler_llvm_context::OptimizerSettings,
        llvm_options: Vec<String>,
        suppressed_warnings: Vec<WarningType>,
        debug_config: Option<era_compiler_llvm_context::DebugConfig>,
    ) -> anyhow::Result<ContractBuild> {
        match self {
            Self::Vyper(inner) => inner.compile(
                contract_path,
                source_code_hash,
                evm_version,
                optimizer_settings,
                llvm_options,
                suppressed_warnings,
                debug_config,
            ),
            Self::LLVMIR(inner) => inner.compile(
                contract_path,
                source_code_hash,
                optimizer_settings,
                llvm_options,
                suppressed_warnings,
                debug_config,
            ),
            Self::ZKASM(inner) => inner.compile(
                contract_path,
                source_code_hash,
                optimizer_settings,
                llvm_options,
                suppressed_warnings,
                debug_config,
            ),
        }
    }

    ///
    /// Returns the source code reference.
    ///
    pub fn source_code(&self) -> &str {
        match self {
            Self::Vyper(inner) => inner.source_code.as_str(),
            Self::LLVMIR(inner) => inner.source_code.as_str(),
            Self::ZKASM(inner) => inner.source_code.as_str(),
        }
    }
}
