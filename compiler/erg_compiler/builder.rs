use erg_common::config::ErgConfig;
use erg_common::traits::{Runnable, Stream};
use erg_common::Str;

use erg_parser::ast::VarName;
use erg_parser::builder::ASTBuilder;

use crate::error::{CompileError, CompileErrors, TyCheckErrors};
// use crate::hir::HIR;
use crate::check::Checker;
use crate::mod_cache::SharedModuleCache;
use crate::reorder::Reorderer;

#[derive(Debug)]
pub struct HIRBuilder {
    checker: Checker,
    mod_cache: SharedModuleCache,
}

impl HIRBuilder {
    fn convert(&self, errs: TyCheckErrors) -> CompileErrors {
        errs.into_iter()
            .map(|e| CompileError::new(e.core, self.checker.input().clone(), e.caused_by))
            .collect::<Vec<_>>()
            .into()
    }

    pub fn new<S: Into<Str>>(cfg: ErgConfig, mod_name: S, mod_cache: SharedModuleCache) -> Self {
        Self {
            checker: Checker::new_with_cache(cfg, mod_name, mod_cache.clone()),
            mod_cache,
        }
    }

    pub fn build_and_cache(&mut self, var_name: VarName, mode: &str) -> Result<(), CompileErrors> {
        let mut ast_builder = ASTBuilder::new(self.checker.cfg().copy());
        let ast = ast_builder.build()?;
        let ast = Reorderer::new()
            .reorder(ast)
            .map_err(|errs| self.convert(errs))?;
        let (hir, ctx) = self
            .checker
            .check(ast, mode)
            .map_err(|errs| self.convert(errs))?;
        self.mod_cache.register(var_name, Some(hir), ctx);
        Ok(())
    }
}
