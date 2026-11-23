//! A minimal pass manager and the context a pass runs in.

use rakit_codegen::Context;
use rakit_module::Module;

use crate::analyses::Analyses;

/// Everything a pass may touch: the function being optimized (inside `ctx`),
/// the module it will be emitted into, and the shared analysis cache.
pub struct PassCtx<'a> {
    pub ctx: &'a mut Context,
    pub module: &'a mut dyn Module,
    pub analyses: &'a mut Analyses,
    pub crate_name: &'a str,
    pub user_crate: bool,
}

pub trait Pass {
    fn name(&self) -> &'static str;

    /// Run the pass over `pctx.ctx.func`. Returns whether the function changed.
    fn run(&mut self, pctx: &mut PassCtx<'_>) -> bool;
}

#[derive(Default)]
pub struct PassManager {
    passes: Vec<Box<dyn Pass>>,
}

impl PassManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, pass: Box<dyn Pass>) -> &mut Self {
        self.passes.push(pass);
        self
    }

    pub fn passes(&self) -> impl Iterator<Item = &dyn Pass> {
        self.passes.iter().map(|p| p.as_ref())
    }

    pub fn run(&mut self, pctx: &mut PassCtx<'_>) {
        for pass in &mut self.passes {
            pass.run(pctx);
            pctx.analyses.invalidate();
        }
    }
}
