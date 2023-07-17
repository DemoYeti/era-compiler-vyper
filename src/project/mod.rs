//!
//! The Vyper project.
//!

pub mod contract;
pub mod dependency_data;

use std::collections::BTreeMap;
use std::path::Path;

use rayon::iter::IntoParallelIterator;
use rayon::iter::ParallelIterator;
use sha3::Digest;

use crate::build::contract::Contract as ContractBuild;
use crate::build::Build;
use crate::process::input::Input as ProcessInput;

use self::contract::llvm_ir::Contract as LLVMIRContract;
use self::contract::zkasm::Contract as ZKASMContract;
use self::contract::Contract;

///
/// The Vyper project.
///
#[derive(Debug, Clone)]
pub struct Project {
    /// The Vyper compiler version.
    pub version: semver::Version,
    /// The project source code hash.
    pub source_code_hash: [u8; compiler_common::BYTE_LENGTH_FIELD],
    /// The contract data,
    pub contracts: BTreeMap<String, Contract>,
}

impl Project {
    ///
    /// A shortcut constructor.
    ///
    pub fn new(
        version: semver::Version,
        source_code_hash: [u8; compiler_common::BYTE_LENGTH_FIELD],
        contracts: BTreeMap<String, Contract>,
    ) -> Self {
        Self {
            version,
            source_code_hash,
            contracts,
        }
    }

    ///
    /// Parses the LLVM IR source code file and returns the source data.
    ///
    pub fn try_from_llvm_ir_path(path: &Path) -> anyhow::Result<Self> {
        let source_code = std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("LLVM IR file {:?} reading error: {}", path, error))?;
        let path = path.to_string_lossy().to_string();

        let source_code_hash = sha3::Keccak256::digest(source_code.as_bytes()).into();

        let mut project_contracts = BTreeMap::new();
        project_contracts.insert(
            path,
            LLVMIRContract::new(compiler_llvm_context::LLVM_VERSION, source_code).into(),
        );

        Ok(Self::new(
            compiler_llvm_context::LLVM_VERSION,
            source_code_hash,
            project_contracts,
        ))
    }

    ///
    /// Parses the zkEVM assembly source code file and returns the source data.
    ///
    pub fn try_from_zkasm_path(path: &Path) -> anyhow::Result<Self> {
        let source_code = std::fs::read_to_string(path).map_err(|error| {
            anyhow::anyhow!("zkEVM assembly file {:?} reading error: {}", path, error)
        })?;
        let path = path.to_string_lossy().to_string();

        let source_code_hash = sha3::Keccak256::digest(source_code.as_bytes()).into();

        let mut project_contracts = BTreeMap::new();
        project_contracts.insert(
            path,
            ZKASMContract::new(compiler_llvm_context::ZKEVM_VERSION, source_code).into(),
        );

        Ok(Self::new(
            compiler_llvm_context::ZKEVM_VERSION,
            source_code_hash,
            project_contracts,
        ))
    }

    ///
    /// Compiles all contracts, returning the build.
    ///
    pub fn compile(
        self,
        optimizer_settings: compiler_llvm_context::OptimizerSettings,
        include_metadata_hash: bool,
        bytecode_encoding: zkevm_assembly::RunningVmEncodingMode,
        debug_config: Option<compiler_llvm_context::DebugConfig>,
    ) -> anyhow::Result<Build> {
        let mut build = Build::default();
        let source_code_hash = if include_metadata_hash {
            Some(self.source_code_hash)
        } else {
            None
        };
        let results: BTreeMap<String, anyhow::Result<ContractBuild>> = self
            .contracts
            .into_par_iter()
            .map(|(full_path, contract)| {
                let process_output = crate::process::call(ProcessInput::new(
                    full_path.clone(),
                    contract,
                    source_code_hash,
                    bytecode_encoding == zkevm_assembly::RunningVmEncodingMode::Testing,
                    optimizer_settings.clone(),
                    debug_config.clone(),
                ));

                (full_path, process_output.map(|output| output.build))
            })
            .collect();

        let is_forwarder_used = results.iter().any(|(_path, result)| {
            result
                .as_ref()
                .map(|contract| {
                    contract
                        .build
                        .factory_dependencies
                        .contains_key(crate::r#const::FORWARDER_CONTRACT_HASH.as_str())
                })
                .unwrap_or_default()
        });
        if is_forwarder_used {
            let forwarder_build = compiler_llvm_context::Build::new(
                crate::r#const::FORWARDER_CONTRACT_ASSEMBLY.to_owned(),
                None,
                crate::r#const::FORWARDER_CONTRACT_BYTECODE.clone(),
                crate::r#const::FORWARDER_CONTRACT_HASH.clone(),
            );
            build.contracts.insert(
                crate::r#const::FORWARDER_CONTRACT_NAME.to_owned(),
                ContractBuild::new(forwarder_build),
            );
        }

        for (path, result) in results.into_iter() {
            match result {
                Ok(contract) => {
                    build.contracts.insert(path, contract);
                }
                Err(error) => {
                    anyhow::bail!("Contract `{}` compiling error: {:?}", path, error);
                }
            }
        }

        Ok(build)
    }
}
