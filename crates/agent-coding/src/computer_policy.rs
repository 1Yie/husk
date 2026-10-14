//! Desktop input approval rules installed by the coding host.

use std::collections::HashSet;

use agent_kernel::permissions::{Decision, ToolPolicy};

/// Starting a program requires its own grant, beyond permission to drive the
/// current desktop.
pub const COMPUTER_LAUNCH_RULE: &str = "computer:launch";

pub struct ComputerPolicy;

impl ToolPolicy for ComputerPolicy {
    fn ask_by_default(&self) -> bool {
        true
    }

    fn escalate(
        &self,
        action: &str,
        decision: Decision,
        allow: &HashSet<String>,
        can_ask: bool,
        diff_summary: &str,
    ) -> Decision {
        if action == "launch"
            && matches!(decision, Decision::Allow)
            && !allow.contains(COMPUTER_LAUNCH_RULE)
        {
            if can_ask {
                Decision::Ask {
                    diff_summary: diff_summary.into(),
                }
            } else {
                Decision::Deny {
                    reason: "`launch` starts a program and needs its own approval".into(),
                }
            }
        } else {
            decision
        }
    }

    fn grants(&self) -> Vec<String> {
        vec![COMPUTER_LAUNCH_RULE.into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use agent_kernel::permissions::{PermissionGate, PermissionMode, PermissionRules};
    use agent_kernel::tools::registry::ToolClass;

    fn gate(mode: &str) -> PermissionGate {
        PermissionGate::from_mode_str(mode).with_policy("computer", Arc::new(ComputerPolicy))
    }

    fn decide(g: &PermissionGate) -> Decision {
        g.decide("computer", Some(ToolClass::Process), false, None, "x")
    }

    #[test]
    fn computer_asks_in_every_interactive_mode_until_granted() {
        for mode in ["default", "acceptEdits", "auto", "bypass"] {
            let mut g = gate(mode);
            assert!(matches!(decide(&g), Decision::Ask { .. }), "{mode}");
            g.allow_tool_for_session("computer");
            assert_eq!(decide(&g), Decision::Allow, "{mode}");
        }
        assert!(matches!(decide(&gate("dontAsk")), Decision::Deny { .. }));
    }

    #[test]
    fn launch_asks_even_under_a_session_grant() {
        for mode in ["default", "acceptEdits", "auto", "bypass"] {
            let mut g = gate(mode);
            g.allow_tool_for_session("computer");
            for action in ["left_click", "type"] {
                assert_eq!(
                    g.escalate("computer", action, decide(&g), "x"),
                    Decision::Allow
                );
            }
            assert!(
                matches!(
                    g.escalate("computer", "launch", decide(&g), "x"),
                    Decision::Ask { .. }
                ),
                "{mode}"
            );
        }
    }

    #[test]
    fn launch_is_denied_where_nobody_can_answer() {
        for mut g in [
            gate("dontAsk"),
            PermissionGate::for_subagent().with_policy("computer", Arc::new(ComputerPolicy)),
        ] {
            g.allow_tool_for_session("computer");
            assert!(matches!(
                g.escalate("computer", "launch", decide(&g), "x"),
                Decision::Deny { .. }
            ));
        }
    }

    #[test]
    fn explicit_launch_rule_pre_approves_it() {
        let mut g = gate("default");
        g.allow_tool_for_session("computer");
        g.allow_tool_for_session(COMPUTER_LAUNCH_RULE);
        assert_eq!(
            g.escalate("computer", "launch", decide(&g), "x"),
            Decision::Allow
        );
    }

    #[test]
    fn deny_and_ask_are_preserved_even_with_launch_pre_approved() {
        for rules in [
            PermissionRules {
                deny: ["computer".into()].into_iter().collect(),
                ..Default::default()
            },
            PermissionRules {
                ask: ["computer".into()].into_iter().collect(),
                ..Default::default()
            },
        ] {
            let mut g = PermissionGate::new(PermissionMode::Auto, rules)
                .with_policy("computer", Arc::new(ComputerPolicy));
            g.allow_tool_for_session("computer");
            g.allow_tool_for_session(COMPUTER_LAUNCH_RULE);
            let base = decide(&g);
            assert_ne!(base, Decision::Allow);
            assert_eq!(g.escalate("computer", "launch", base.clone(), "x"), base);
        }
    }

    #[test]
    fn turn_end_revokes_computer_and_launch_grants() {
        let mut g = gate("auto");
        g.allow_tool_for_session("computer");
        g.allow_tool_for_session(COMPUTER_LAUNCH_RULE);
        g.allow_tool_for_session("bash");
        assert!(g.revoke_grants("computer"));
        assert!(matches!(decide(&g), Decision::Ask { .. }));
        // Regranting desktop input must not revive the old launch grant.
        g.allow_tool_for_session("computer");
        assert!(matches!(
            g.escalate("computer", "launch", decide(&g), "x"),
            Decision::Ask { .. }
        ));
        assert_eq!(g.decide("bash", None, false, None, ""), Decision::Allow);
        assert!(g.revoke_grants("computer"));
        assert!(!g.revoke_grants("computer"));
    }
}
