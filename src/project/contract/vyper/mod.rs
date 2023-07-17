//!
//! The Vyper contract.
//!

pub mod expression;
pub mod function;

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;

use compiler_llvm_context::Dependency;
use compiler_llvm_context::WriteLLVM;

use crate::build::contract::Contract as ContractBuild;
use crate::metadata::Metadata as SourceMetadata;
use crate::project::contract::metadata::Metadata as ContractMetadata;
use crate::project::dependency_data::DependencyData;

use self::expression::Expression;
use self::function::Function;

///
/// The Vyper contract.
///
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Contract {
    /// The Vyper compiler version.
    pub version: semver::Version,
    /// The Vyper contract source code.
    pub source_code: String,
    /// The source metadata.
    pub source_metadata: SourceMetadata,
    /// The inner expression.
    pub expression: Expression,
    /// The contract ABI data.
    pub abi: BTreeMap<String, String>,
    /// The dependency data.
    pub dependency_data: DependencyData,
}

impl Contract {
    /// The number of vyper compiler output lines per contract.
    pub const EXPECTED_LINES: usize = 3;

    ///
    /// A shortcut constructor.
    ///
    pub fn new(
        version: semver::Version,
        source_code: String,
        source_metadata: SourceMetadata,
        expression: Expression,
        abi: BTreeMap<String, String>,
    ) -> Self {
        Self {
            version,
            source_code,
            source_metadata,
            expression,
            abi,
            dependency_data: DependencyData::default(),
        }
    }

    ///
    /// Parses three lines with JSONs, returned by the Vyper compiler.
    /// The order must be:
    /// 1. The LLL IR JSON
    /// 2. The contract functions metadata
    /// 3. The contract ABI data
    ///
    pub fn try_from_lines(
        version: semver::Version,
        source_code: String,
        mut lines: Vec<&str>,
    ) -> anyhow::Result<Self> {
        if lines.len() != Self::EXPECTED_LINES {
            anyhow::bail!(
                "Expected {} lines with JSONs, found {}",
                Self::EXPECTED_LINES,
                lines.len()
            );
        }

        let mut deserializer = serde_json::Deserializer::from_str(lines.remove(0));
        deserializer.disable_recursion_limit();
        let deserializer = serde_stacker::Deserializer::new(&mut deserializer);
        let expression = Expression::deserialize(deserializer)?;

        let metadata: SourceMetadata = serde_json::from_str(lines.remove(0))?;

        let abi: BTreeMap<String, String> = serde_json::from_str(lines.remove(0))?;

        Ok(Self::new(version, source_code, metadata, expression, abi))
    }

    ///
    /// Compiles the contract, returning the build.
    ///
    pub fn compile(
        mut self,
        contract_path: &str,
        source_code_hash: Option<[u8; compiler_common::BYTE_LENGTH_FIELD]>,
        optimizer_settings: compiler_llvm_context::OptimizerSettings,
        debug_config: Option<compiler_llvm_context::DebugConfig>,
    ) -> anyhow::Result<ContractBuild> {
        let llvm = inkwell::context::Context::create();
        let optimizer = compiler_llvm_context::Optimizer::new(optimizer_settings);

        let metadata_hash = source_code_hash.map(|source_code_hash| {
            ContractMetadata::new(
                &source_code_hash,
                &self.version,
                semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("Always valid"),
                optimizer.settings().to_owned(),
            )
            .keccak256()
        });

        let dependency_data = DependencyData::default();
        let mut context = compiler_llvm_context::Context::<DependencyData>::new(
            &llvm,
            llvm.create_module(contract_path),
            optimizer,
            Some(dependency_data),
            metadata_hash.is_some(),
            debug_config,
        );

        self.declare(&mut context).map_err(|error| {
            anyhow::anyhow!(
                "The contract `{}` LLVM IR generator declaration pass error: {}",
                contract_path,
                error
            )
        })?;
        self.into_llvm(&mut context).map_err(|error| {
            anyhow::anyhow!(
                "The contract `{}` LLVM IR generator definition pass error: {}",
                contract_path,
                error
            )
        })?;

        let is_forwarder_used = context.vyper().is_forwarder_used();
        let mut build = context.build(contract_path, metadata_hash)?;

        if is_forwarder_used {
            build.factory_dependencies.insert(
                crate::r#const::FORWARDER_CONTRACT_HASH.clone(),
                crate::r#const::FORWARDER_CONTRACT_NAME.to_owned(),
            );
        }

        Ok(ContractBuild::new(build))
    }
}

impl<D> WriteLLVM<D> for Contract
where
    D: Dependency + Clone,
{
    fn declare(&mut self, context: &mut compiler_llvm_context::Context<D>) -> anyhow::Result<()> {
        let mut entry = compiler_llvm_context::EntryFunction::default();
        entry.declare(context)?;

        let mut runtime =
            compiler_llvm_context::Runtime::new(compiler_llvm_context::AddressSpace::HeapAuxiliary);
        runtime.declare(context)?;

        compiler_llvm_context::DeployCodeFunction::new(
            compiler_llvm_context::DummyLLVMWritable::default(),
        )
        .declare(context)?;
        compiler_llvm_context::RuntimeCodeFunction::new(
            compiler_llvm_context::DummyLLVMWritable::default(),
        )
        .declare(context)?;

        for name in [
            compiler_llvm_context::Runtime::FUNCTION_DEPLOY_CODE,
            compiler_llvm_context::Runtime::FUNCTION_RUNTIME_CODE,
            compiler_llvm_context::Runtime::FUNCTION_ENTRY,
        ]
        .into_iter()
        {
            context
                .get_function(name)
                .expect("Always exists")
                .borrow_mut()
                .set_vyper_data(compiler_llvm_context::FunctionVyperData::default());
        }

        entry.into_llvm(context)?;

        runtime.into_llvm(context)?;

        Ok(())
    }

    fn into_llvm(mut self, context: &mut compiler_llvm_context::Context<D>) -> anyhow::Result<()> {
        let (mut runtime_code, immutables_size) =
            self.expression.extract_runtime_code()?.unwrap_or_default();
        let mut deploy_code = self.expression.try_into_deploy_code()?;

        match immutables_size {
            Expression::IntegerLiteral(number) => {
                let immutables_size = number
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("Immutable size `{}` parsing error", number))?;
                let vyper_data =
                    compiler_llvm_context::ContextVyperData::new(immutables_size as usize, false);
                context.set_vyper_data(vyper_data);
            }
            expression => anyhow::bail!("Invalid immutables size format: {:?}", expression),
        }

        let mut function_expressions = deploy_code
            .extract_functions()?
            .into_iter()
            .map(|(label, expression)| (label, expression, compiler_llvm_context::CodeType::Deploy))
            .collect::<Vec<(String, Expression, compiler_llvm_context::CodeType)>>();
        function_expressions.extend(
            runtime_code
                .extract_functions()?
                .into_iter()
                .map(|(label, expression)| {
                    (label, expression, compiler_llvm_context::CodeType::Runtime)
                })
                .collect::<Vec<(String, Expression, compiler_llvm_context::CodeType)>>(),
        );

        let mut functions = Vec::with_capacity(function_expressions.capacity());
        for (label, expression, code_type) in function_expressions.into_iter() {
            let mut metadata_label = label
                .strip_suffix(format!("_{}", compiler_llvm_context::CodeType::Deploy).as_str())
                .unwrap_or(label.as_str());
            metadata_label = label
                .strip_suffix(format!("_{}", compiler_llvm_context::CodeType::Runtime).as_str())
                .unwrap_or(metadata_label);
            metadata_label = label
                .strip_suffix(format!("_{}", crate::r#const::LABEL_SUFFIX_COMMON).as_str())
                .unwrap_or(metadata_label);

            let metadata_name =
                self.source_metadata
                    .function_info
                    .iter()
                    .find_map(|(name, function)| {
                        if metadata_label == function.ir_identifier.as_str() {
                            Some(name.to_owned())
                        } else {
                            None
                        }
                    });
            let metadata = match metadata_name {
                Some(metadata_name) => self
                    .source_metadata
                    .function_info
                    .get(metadata_name.as_str())
                    .cloned(),
                None => None,
            };
            functions.push((Function::new(label, metadata, expression), code_type));
        }
        for (function, _code_type) in functions.iter_mut() {
            function.declare(context)?;
        }
        for (function, code_type) in functions.into_iter() {
            context.set_code_type(code_type);
            function.into_llvm(context)?;
        }

        compiler_llvm_context::DeployCodeFunction::new(deploy_code).into_llvm(context)?;
        compiler_llvm_context::RuntimeCodeFunction::new(runtime_code).into_llvm(context)?;

        Ok(())
    }
}

impl Dependency for DependencyData {
    fn compile(
        _contract: Self,
        _name: &str,
        _optimizer_settings: compiler_llvm_context::OptimizerSettings,
        _is_system_mode: bool,
        _include_metadata_hash: bool,
        _debug_config: Option<compiler_llvm_context::DebugConfig>,
    ) -> anyhow::Result<String> {
        Ok(crate::r#const::FORWARDER_CONTRACT_HASH.clone())
    }

    fn resolve_path(&self, _identifier: &str) -> anyhow::Result<String> {
        anyhow::bail!("The dependency mechanism is not available in Vyper");
    }

    fn resolve_library(&self, _path: &str) -> anyhow::Result<String> {
        anyhow::bail!("The dependency mechanism is not available in Vyper");
    }
}
