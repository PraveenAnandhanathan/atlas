//! The full ATLAS capability catalog (design §7.1).
//!
//! Adapters parse a flat string like `"atlas.fs.read"` into a
//! [`Capability`] so the dispatcher can be exhaustive.

use serde::{Deserialize, Serialize};

/// One concrete capability, namespaced exactly as in design §7.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum Capability {
    // Read / write
    fs_stat,
    fs_list,
    fs_read,
    fs_read_text,
    fs_read_tensor,
    fs_read_schema,
    fs_write,
    fs_append,
    fs_delete,

    // Semantic
    semantic_query,
    semantic_similar,
    semantic_embed,
    semantic_describe,

    // Versioning
    version_log,
    version_diff,
    version_branch_create,
    version_branch_list,
    version_checkout,
    version_commit,
    version_tag,

    // Lineage
    lineage_upstream,
    lineage_downstream,
    lineage_provenance,
    lineage_record,

    // Governance
    policy_show,
    policy_check,
    policy_audit,
    policy_set,

    // Agent / workflow
    agent_scratchpad_create,
    agent_checkpoint,
    agent_fork,
}

impl Capability {
    /// Canonical wire name (`atlas.<namespace>.<verb>`).
    pub fn name(self) -> &'static str {
        use Capability::*;
        match self {
            fs_stat => "atlas.fs.stat",
            fs_list => "atlas.fs.list",
            fs_read => "atlas.fs.read",
            fs_read_text => "atlas.fs.read_text",
            fs_read_tensor => "atlas.fs.read_tensor",
            fs_read_schema => "atlas.fs.read_schema",
            fs_write => "atlas.fs.write",
            fs_append => "atlas.fs.append",
            fs_delete => "atlas.fs.delete",
            semantic_query => "atlas.semantic.query",
            semantic_similar => "atlas.semantic.similar",
            semantic_embed => "atlas.semantic.embed",
            semantic_describe => "atlas.semantic.describe",
            version_log => "atlas.version.log",
            version_diff => "atlas.version.diff",
            version_branch_create => "atlas.version.branch_create",
            version_branch_list => "atlas.version.branch_list",
            version_checkout => "atlas.version.checkout",
            version_commit => "atlas.version.commit",
            version_tag => "atlas.version.tag",
            lineage_upstream => "atlas.lineage.upstream",
            lineage_downstream => "atlas.lineage.downstream",
            lineage_provenance => "atlas.lineage.provenance",
            lineage_record => "atlas.lineage.record",
            policy_show => "atlas.policy.show",
            policy_check => "atlas.policy.check",
            policy_audit => "atlas.policy.audit",
            policy_set => "atlas.policy.set",
            agent_scratchpad_create => "atlas.agent.scratchpad_create",
            agent_checkpoint => "atlas.agent.checkpoint",
            agent_fork => "atlas.agent.fork",
        }
    }

    /// All capabilities in declaration order. Used for tool-listing.
    pub fn all() -> &'static [Capability] {
        use Capability::*;
        &[
            fs_stat,
            fs_list,
            fs_read,
            fs_read_text,
            fs_read_tensor,
            fs_read_schema,
            fs_write,
            fs_append,
            fs_delete,
            semantic_query,
            semantic_similar,
            semantic_embed,
            semantic_describe,
            version_log,
            version_diff,
            version_branch_create,
            version_branch_list,
            version_checkout,
            version_commit,
            version_tag,
            lineage_upstream,
            lineage_downstream,
            lineage_provenance,
            lineage_record,
            policy_show,
            policy_check,
            policy_audit,
            policy_set,
            agent_scratchpad_create,
            agent_checkpoint,
            agent_fork,
        ]
    }

    /// Whether this capability mutates state. Adapters use this to pick
    /// the matching [`atlas_governor::Permission`].
    pub fn mutates(self) -> bool {
        use Capability::*;
        matches!(
            self,
            fs_write
                | fs_append
                | fs_delete
                | semantic_embed
                | version_branch_create
                | version_checkout
                | version_commit
                | version_tag
                | lineage_record
                | policy_set
                | agent_scratchpad_create
                | agent_checkpoint
                | agent_fork
        )
    }
}

/// Parse `"atlas.fs.read"` style strings.  Accepts the underscore-form
/// (`fs_read`) too, since the OpenAI/Anthropic adapters normalise dots
/// to underscores in tool names.
pub fn parse_capability(name: &str) -> Option<Capability> {
    for &c in Capability::all() {
        if c.name() == name || c.name().replace('.', "_") == name {
            return Some(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_every_name() {
        for &c in Capability::all() {
            assert_eq!(parse_capability(c.name()), Some(c), "{}", c.name());
        }
    }

    #[test]
    fn underscore_form_parses() {
        assert_eq!(
            parse_capability("atlas_fs_read"),
            Some(Capability::fs_read)
        );
    }
}
