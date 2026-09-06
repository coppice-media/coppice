#[allow(clippy::module_inception)]
mod emailer;
/// Visible to the whole `mutation` tree: the send-to-Kindle lane
/// (`mutation::kindle`) reuses this emailer resolution and SMTP path rather
/// than opening a second one.
pub(super) mod sender;

pub use emailer::EmailerMutation;
