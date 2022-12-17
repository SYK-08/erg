use std::fmt;

use erg_common::traits::{Runnable, Stream};

use crate::context::Context;
use crate::error::CompileErrors;
use crate::hir::HIR;

#[derive(Debug)]
pub struct CompleteArtifact<Inner = HIR> {
    pub object: Inner,
    pub warns: CompileErrors,
}

impl<Inner> CompleteArtifact<Inner> {
    pub const fn new(object: Inner, warns: CompileErrors) -> Self {
        Self { object, warns }
    }
}

#[derive(Debug)]
pub struct IncompleteArtifact<Inner = HIR> {
    pub object: Option<Inner>,
    pub errors: CompileErrors,
    pub warns: CompileErrors,
}

impl fmt::Display for IncompleteArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.warns.is_empty() {
            writeln!(f, "{}", self.warns)?;
        }
        write!(f, "{}", self.errors)
    }
}

impl std::error::Error for IncompleteArtifact {}

impl<Inner> IncompleteArtifact<Inner> {
    pub const fn new(object: Option<Inner>, errors: CompileErrors, warns: CompileErrors) -> Self {
        Self {
            object,
            errors,
            warns,
        }
    }
}

#[derive(Debug)]
pub struct ErrorArtifact {
    pub errors: CompileErrors,
    pub warns: CompileErrors,
}

impl fmt::Display for ErrorArtifact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.warns.is_empty() {
            writeln!(f, "{}", self.warns)?;
        }
        write!(f, "{}", self.errors)
    }
}

impl std::error::Error for ErrorArtifact {}

impl From<IncompleteArtifact> for ErrorArtifact {
    fn from(artifact: IncompleteArtifact) -> Self {
        Self {
            errors: artifact.errors,
            warns: artifact.warns,
        }
    }
}

impl ErrorArtifact {
    pub const fn new(errors: CompileErrors, warns: CompileErrors) -> Self {
        Self { errors, warns }
    }
}

pub trait Buildable<T = HIR> {
    fn build(
        &mut self,
        src: String,
        mode: &str,
    ) -> Result<CompleteArtifact<T>, IncompleteArtifact<T>>;
    fn pop_context(&mut self) -> Option<Context>;
    fn get_context(&self) -> Option<&Context>;
}

pub trait BuildRunnable<T = HIR>: Buildable<T> + Runnable {}
