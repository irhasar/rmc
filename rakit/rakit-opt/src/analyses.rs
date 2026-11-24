//! Lazily computed, cacheable function analyses shared by the pass pipeline.
//!
//! Before this existed every pass that needed a CFG, a dominator tree or a loop
//! forest rebuilt it from scratch, and a pass that ran twice in a row rebuilt
//! the same analysis twice. The manager now keeps one copy per function and
//! invalidates it whenever a pass has rewritten the function.

use rakit_codegen::dominator_tree::DominatorTree;
use rakit_codegen::flowgraph::ControlFlowGraph;
use rakit_codegen::ir::Function;
use rakit_codegen::loop_analysis::LoopAnalysis;

#[derive(Default)]
pub struct Analyses {
    cfg: Option<ControlFlowGraph>,
    domtree: Option<DominatorTree>,
    loops: Option<LoopAnalysis>,
}

impl Analyses {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop every cached analysis. The manager calls this after each pass, since
    /// a pass may change the CFG or the dominator tree.
    pub fn invalidate(&mut self) {
        self.cfg = None;
        self.domtree = None;
        self.loops = None;
    }

    pub fn cfg(&mut self, func: &Function) -> &ControlFlowGraph {
        if self.cfg.is_none() {
            self.cfg = Some(ControlFlowGraph::with_function(func));
        }
        self.cfg.as_ref().unwrap()
    }

    pub fn domtree(&mut self, func: &Function) -> &DominatorTree {
        if self.domtree.is_none() {
            if self.cfg.is_none() {
                self.cfg = Some(ControlFlowGraph::with_function(func));
            }
            let mut dom = DominatorTree::new();
            dom.compute(func, self.cfg.as_ref().unwrap());
            self.domtree = Some(dom);
        }
        self.domtree.as_ref().unwrap()
    }

    pub fn loops(&mut self, func: &Function) -> &LoopAnalysis {
        if self.loops.is_none() {
            if self.cfg.is_none() {
                self.cfg = Some(ControlFlowGraph::with_function(func));
            }
            if self.domtree.is_none() {
                let mut dom = DominatorTree::new();
                dom.compute(func, self.cfg.as_ref().unwrap());
                self.domtree = Some(dom);
            }
            let mut loops = LoopAnalysis::new();
            loops.compute(
                func,
                self.cfg.as_ref().unwrap(),
                self.domtree.as_ref().unwrap(),
            );
            self.loops = Some(loops);
        }
        self.loops.as_ref().unwrap()
    }
}
