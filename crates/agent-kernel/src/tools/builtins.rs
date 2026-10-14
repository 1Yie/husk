//! The kernel's built-in tool set.
//!
//! `registry` holds the generic contract (specs, dispatch, the extension table);
//! this module names the concrete tools the kernel ships and the subset a
//! delegated child may draw from. It lives here, not in `registry`, so the
//! contract can be extracted without pulling in every tool module.

use super::registry::{ToolClass, ToolRegistry};

impl ToolRegistry {
    /// Register the phase-1 built-in set.
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        r.register(crate::tools::fs_read::spec());
        r.register(crate::tools::fs_patch::spec());
        r.register(crate::tools::apply_patch::spec());
        r.register(crate::tools::list_dir::spec());
        r.register(crate::tools::grep::spec());
        r.register(crate::tools::test_runner::spec());
        r.register(crate::tools::bash::spec());
        r.register(crate::tools::services::spec());
        r.register(crate::tools::todo::spec());
        r.register(crate::tools::skill::spec());
        r.register(crate::tools::remember::spec());
        r.register(crate::tools::serena::spec());
        r.register(crate::tools::web_fetch::spec());
        r.register(crate::tools::web_fetch::spec_alias());
        r.register(crate::tools::screenshot::spec());
        r.register(crate::tools::computer::spec());
        r.register(crate::tools::delegate::spec());
        r.register(crate::tools::ask::spec());
        r.register(crate::tools::batch::spec());
        r.register(crate::tools::blueprint::spec());
        r
    }

    /// The registry a delegated child may draw from: builtins minus the tools no
    /// child may hold. `delegate` is the recursion guard, `computer` and
    /// `HumanInteraction` need a human a child cannot reach, and `remember` writes
    /// the persona every later parent turn reads — a child works on untrusted
    /// content, so it must not write that.
    pub fn child_builtins() -> Self {
        Self::with_builtins().filtered(|s| {
            s.name != "delegate"
                && s.name != "remember"
                && s.name != "computer"
                && s.class != ToolClass::HumanInteraction
        })
    }
}
