//! Deciding what the agent may do on its own.
//!
//! Every tool call passes through [`PolicyEngine`], which answers Allow, Deny, or Ask.
//! [`Gated`] puts that check on the one path a call takes, so no tool can skip it. Asking
//! the user is an injected [`Approver`], so nothing here draws an interface.
//!
//! Shell commands get more than a string comparison. A command line is split into the
//! programs it actually runs, and each is judged separately, so a rule about `git status`
//! says nothing about `git status; rm -rf ~`. Anything the parser will not vouch for —
//! substitution, subshells, expansion — is escalated to the user rather than assumed
//! harmless. See [`shell`].

mod decision;
mod error;
mod gate;
mod policy;
mod trust;

pub mod shell;

pub use decision::Approval;
pub use decision::ApprovalRequest;
pub use decision::Approver;
pub use decision::Decision;
pub use decision::DenyEverything;
pub use decision::Rule;
pub use error::PolicyError;
pub use error::Result;
pub use gate::gated_tools;
pub use gate::Gated;
pub use policy::micro_home;
pub use trust::TrustDecision;
pub use trust::TrustStore;
pub use trust::TRUST_FILE_NAME;
pub use policy::subject;
pub use policy::Mode;
pub use policy::Policy;
pub use policy::PolicyEngine;
pub use policy::MICRO_DIR_ENV;
pub use policy::POLICY_FILE_NAME;
