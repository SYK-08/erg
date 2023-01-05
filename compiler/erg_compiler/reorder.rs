use erg_common::config::ErgConfig;
use erg_common::dict::Dict;
use erg_common::log;
use erg_common::traits::{Locational, Stream};
use erg_common::Str;

use erg_parser::ast::{ClassDef, Expr, Methods, Module, PatchDef, PreDeclTypeSpec, TypeSpec, AST};

use crate::error::{TyCheckError, TyCheckErrors};

/// Combine method definitions across multiple modules, specialized class contexts, etc.
#[derive(Debug, Default)]
pub struct Reorderer {
    cfg: ErgConfig,
    // TODO: inner scope types
    pub def_root_pos_map: Dict<Str, usize>,
    pub deps: Dict<Str, Vec<Str>>,
    pub errs: TyCheckErrors,
}

impl Reorderer {
    pub fn new(cfg: ErgConfig) -> Self {
        Self {
            cfg,
            def_root_pos_map: Dict::new(),
            deps: Dict::new(),
            errs: TyCheckErrors::empty(),
        }
    }

    pub fn reorder(mut self, ast: AST) -> Result<AST, TyCheckErrors> {
        log!(info "the reordering process has started.");
        let mut new = vec![];
        for chunk in ast.module.into_iter() {
            match chunk {
                Expr::Def(def) => {
                    match def.body.block.first().unwrap() {
                        Expr::Call(call) => {
                            match call.obj.get_name().map(|s| &s[..]) {
                                // TODO: decorator
                                Some("Class" | "Inherit" | "Inheritable") => {
                                    self.def_root_pos_map.insert(
                                        def.sig.ident().unwrap().inspect().clone(),
                                        new.len(),
                                    );
                                    let type_def = ClassDef::new(def, vec![]);
                                    new.push(Expr::ClassDef(type_def));
                                }
                                Some("Patch") => {
                                    self.def_root_pos_map.insert(
                                        def.sig.ident().unwrap().inspect().clone(),
                                        new.len(),
                                    );
                                    let type_def = PatchDef::new(def, vec![]);
                                    new.push(Expr::PatchDef(type_def));
                                }
                                _ => {
                                    new.push(Expr::Def(def));
                                }
                            }
                        }
                        _ => {
                            new.push(Expr::Def(def));
                        }
                    }
                }
                Expr::Methods(methods) => match &methods.class {
                    TypeSpec::PreDeclTy(PreDeclTypeSpec::Simple(simple)) => {
                        self.link_methods(simple.ident.inspect().clone(), &mut new, methods)
                    }
                    TypeSpec::TypeApp { spec, .. } => {
                        if let TypeSpec::PreDeclTy(PreDeclTypeSpec::Simple(simple)) = spec.as_ref()
                        {
                            self.link_methods(simple.ident.inspect().clone(), &mut new, methods)
                        } else {
                            let similar_name = self
                                .def_root_pos_map
                                .keys()
                                .fold("".to_string(), |acc, key| acc + &key[..] + ",");
                            self.errs.push(TyCheckError::no_var_error(
                                self.cfg.input.clone(),
                                line!() as usize,
                                methods.class.loc(),
                                "".into(),
                                &methods.class.to_string(),
                                Some(&Str::from(similar_name)),
                            ));
                        }
                    }
                    other => todo!("{other}"),
                },
                other => {
                    new.push(other);
                }
            }
        }
        let ast = AST::new(ast.name, Module::new(new));
        log!(info "the reordering process has completed:\n{}", ast);
        if self.errs.is_empty() {
            Ok(ast)
        } else {
            Err(self.errs)
        }
    }

    fn link_methods(&mut self, name: Str, new: &mut Vec<Expr>, methods: Methods) {
        if let Some(pos) = self.def_root_pos_map.get(&name) {
            match new.remove(*pos) {
                Expr::ClassDef(mut class_def) => {
                    class_def.methods_list.push(methods);
                    new.insert(*pos, Expr::ClassDef(class_def));
                }
                Expr::PatchDef(mut patch_def) => {
                    patch_def.methods_list.push(methods);
                    new.insert(*pos, Expr::PatchDef(patch_def));
                }
                _ => unreachable!(),
            }
        } else {
            let similar_name = self
                .def_root_pos_map
                .keys()
                .fold("".to_string(), |acc, key| acc + &key[..] + ",");
            self.errs.push(TyCheckError::no_var_error(
                self.cfg.input.clone(),
                line!() as usize,
                methods.class.loc(),
                "".into(),
                &name,
                Some(&Str::from(similar_name)),
            ));
        }
    }
}
