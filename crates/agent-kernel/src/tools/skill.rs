//! `skill` — the agent-facing adapter over the skill backend.
//!
//! Everything real (discovery, loading, rendering, caching) lives in
//! [`crate::skills`]; this file is only the tool contract: args in, a framed
//! instruction block out. Progressive disclosure is the point — the system
//! prompt carries a catalog (name + clipped description), the body is fetched
//! here, on demand, when the model decides a skill fits the task. Inlining
//! every body would cost ~80k tokens on a normal machine.
//!
//! Read-only: it reads instruction files, never the workspace, so it is
//! auto-approved. That is also the whole security story: **loading a skill
//! grants nothing.** The text lands in the turn as instructions, and every
//! action it asks for still goes through Tool → Capability → Policy → Audit →
//! Approval like any other call. A malicious SKILL.md can try to talk the model
//! into something; it cannot widen a tool's permissions.

use std::sync::Arc;

use futures::FutureExt;

use super::registry::{schema_for, ExecFn, ToolError, ToolResult, ToolSpec};
use crate::skills::SkillManager;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SkillArgs {
    /// Skill name exactly as the `## Skills` catalog lists it (e.g. `review`).
    /// Omit when `list` is set.
    name: Option<String>,
    /// Arguments for the skill. An object keyed by the names the skill
    /// declares (`{"path": "src/x.rs"}`) is validated against them; a plain
    /// string is passed through for free-form skills.
    #[serde(default)]
    args: Option<serde_json::Value>,
    /// Dump the whole catalog. A fallback for when the system prompt's catalog
    /// is missing or stale — not the normal path.
    #[serde(default)]
    list: Option<bool>,
}

pub fn spec() -> ToolSpec {
    let exec: ExecFn = Arc::new(|args, ctx| {
        let parsed: SkillArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => {
                return async move { Err(ToolError::Args(format!("skill args: {e}"))) }.boxed()
            }
        };
        let manager = SkillManager::new(ctx.workspace_root.as_ref().to_path_buf());
        async move {
            // `list` is the escape hatch: the catalog is already in the system
            // prompt, so a model that calls this first just spends a round trip.
            if parsed.list.unwrap_or(false) {
                return Ok(ToolResult::text(manager.catalog_listing()));
            }
            // Missing name is a caller mistake, not a lookup miss — the
            // registry's convention is an `Args` error for that.
            let name = parsed.name.as_deref().map(str::trim).unwrap_or("");
            if name.is_empty() {
                return Err(ToolError::Args(
                    "`skill` needs a `name` (or `list: true` for the catalog)".into(),
                ));
            }
            let loaded = match parsed.args.as_ref() {
                // A bare string stays free-form; anything that is not an object
                // is a caller mistake the model can fix immediately.
                Some(serde_json::Value::String(text)) => manager.load(name, Some(text)),
                Some(value) => manager.load_named(name, value),
                None => manager.load(name, None),
            };
            match loaded {
                Ok(skill) => Ok(ToolResult::text(crate::skills::prompt::for_model(&skill))),
                // Errors carry "did you mean …" / "unknown argument `x` — this
                // skill declares: …" so the model corrects itself in one step.
                Err(message) => Err(ToolError::Failed(message)),
            }
        }
        .boxed()
    });

    ToolSpec {
        name: "skill",
        schema: schema_for::<SkillArgs>(
            "Load an installed skill's instructions. The `## Skills` catalog in the\n\
             system prompt already lists what is installed — call this with the skill's\n\
             `name` when a task matches one, BEFORE starting it; the returned\n\
             instructions govern that task. Pass `args` as an object keyed by the\n\
             argument names the catalog shows (a plain string only for free-form\n\
             skills). Reach for `list: true` only when the catalog is missing or you\n\
             need to verify a name.",
        ),
        // Reads instruction files only — no workspace mutation, so it is
        // auto-approved and stays available in plan mode.
        readonly: true,
        // Not Observation: what a load returns governs the turn, and one buried
        // in a `batch_execute` result is how instructions get skimmed.
        // SessionMutation gives it the serial, never-batched slot.
        class: super::registry::ToolClass::SessionMutation,
        network: false,
        exec,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Unique per run — the backend also reads `$HOME`, so a generic name could
    /// collide with a real global skill.
    fn temp_root_with_skill() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("husk-test-skill-{}", std::process::id());
        let skill_dir = dir.path().join(".agents/skills").join(&name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test skill for the unit suite\n---\n\nDo the thing.\n"),
        )
        .unwrap();
        (dir, name)
    }

    fn ctx(root: &std::path::Path) -> Arc<super::super::registry::ToolCtx> {
        Arc::new(super::super::registry::ToolCtx::new(root))
    }

    #[tokio::test]
    async fn loads_a_skill_and_names_its_manifest() {
        let (dir, name) = temp_root_with_skill();
        let out = (spec().exec)(json!({"name": name}), ctx(dir.path())).await.unwrap();
        assert!(out.content.contains("Do the thing."), "{}", out.content);
        assert!(out.content.contains("SKILL.md"), "source is named: {}", out.content);
    }

    /// The tool path and the composer's `$name` path must deliver the same body
    /// in the same frame — a skill cannot behave differently by entry point.
    #[tokio::test]
    async fn tool_and_dollar_paths_share_one_frame() {
        let (dir, name) = temp_root_with_skill();
        let via_tool = (spec().exec)(json!({"name": name}), ctx(dir.path()))
            .await
            .unwrap()
            .content;

        let manager = SkillManager::new(dir.path());
        let loaded = manager.load(&name, None).unwrap();
        let via_dollar = crate::skills::prompt::for_user(&loaded, &format!("${name}"));

        let body_of = |s: &str| {
            let start = s.find("---\n").expect("frame opens") + 4;
            let end = s.rfind("\n---").expect("frame closes");
            s[start..end].to_string()
        };
        assert_eq!(body_of(&via_tool), body_of(&via_dollar));

        // Arguments ride along on both paths.
        let with_args = crate::skills::prompt::for_user(
            &manager.load(&name, Some("src/engine.rs")).unwrap(),
            &format!("${name}"),
        );
        assert!(with_args.contains("Skill arguments: src/engine.rs"));
    }

    #[tokio::test]
    async fn list_is_the_fallback_and_reports_the_catalog() {
        let (dir, name) = temp_root_with_skill();
        let out = (spec().exec)(json!({"list": true}), ctx(dir.path())).await.unwrap();
        assert!(out.content.contains(&name), "{}", out.content);
        assert!(out.content.contains("test skill for the unit suite"), "{}", out.content);
    }

    #[tokio::test]
    async fn unknown_name_suggests_and_missing_name_is_an_arg_error() {
        let (dir, name) = temp_root_with_skill();
        // A near miss, not a different name: the suggestion must actually fire.
        let typo = format!("{}zz", &name[..name.len() - 2]);
        let err = (spec().exec)(json!({"name": typo}), ctx(dir.path()))
            .await
            .expect_err("unknown skill");
        let msg = format!("{err}");
        assert!(msg.contains("did you mean"), "{msg}");
        assert!(msg.contains(&name), "{msg}");

        // No name at all is an argument error, not a lookup miss.
        let err = (spec().exec)(json!({}), ctx(dir.path())).await.expect_err("no name");
        assert!(matches!(err, ToolError::Args(_)), "{err}");
    }

    #[tokio::test]
    async fn structured_arguments_are_validated_and_rendered() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("husk-test-args-{}", std::process::id());
        let skill_dir = dir.path().join(".agents/skills").join(&name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: takes arguments\narguments:\n  <path>: what to look at\n  focus: what to look for\n---\n\nReview it.\n"
            ),
        )
        .unwrap();
        let ctx = ctx(dir.path());

        let out = (spec().exec)(
            json!({"name": name, "args": {"focus": "security", "path": "src/engine.rs"}}),
            ctx.clone(),
        )
        .await
        .unwrap();
        // Declared order, not JSON order; description text intact.
        assert!(out.content.contains("Skill arguments:\n- path: src/engine.rs\n- focus: security"), "{}", out.content);

        let err = (spec().exec)(
            json!({"name": name, "args": {"pth": "src/engine.rs"}}),
            ctx.clone(),
        )
        .await
        .expect_err("unknown argument");
        let msg = format!("{err}");
        assert!(msg.contains("unknown argument `pth`"), "{msg}");
        assert!(msg.contains("path*, focus"), "declared names are named back: {msg}");

        let err = (spec().exec)(json!({"name": name, "args": {"focus": "security"}}), ctx.clone())
            .await
            .expect_err("missing required");
        assert!(format!("{err}").contains("missing required argument `path`"), "{err}");

        // A string arg still works — the composer's shape, and what a skill
        // that declares nothing expects.
        let out = (spec().exec)(json!({"name": name, "args": "src/engine.rs security"}), ctx)
            .await
            .unwrap();
        assert!(out.content.contains("- path: src/engine.rs"), "{}", out.content);
    }

    /// A skill is knowledge, not capability: the tool is readonly whatever the
    /// skill text says, so a malicious SKILL.md cannot widen permissions.
    #[test]
    fn loading_never_grants_more_than_a_read() {
        let spec = spec();
        assert!(spec.readonly);
        assert_eq!(spec.class, super::super::registry::ToolClass::SessionMutation);
        assert!(!spec.network);
    }
}
