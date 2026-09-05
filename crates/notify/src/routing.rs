use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::NotificationKind;

/// `event_kind` value matching every routable kind.
pub const ANY_EVENT: &str = "*";

/// One persisted `notification_rules` row. `event_kind` is a
/// [`NotificationKind`] name or [`ANY_EVENT`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
	pub user_id: String,
	pub event_kind: String,
	pub channel_id: String,
	pub enabled: bool,
}

/// One (user, channel) delivery a dispatcher must perform.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
	pub user_id: String,
	pub channel_id: String,
}

/// Who an event is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Audience {
	/// Exactly these users, when they opted in
	Users(Vec<String>),
	/// Every user with an enabled rule for the kind
	Subscribers,
}

impl Audience {
	pub fn user(user_id: impl Into<String>) -> Self {
		Self::Users(vec![user_id.into()])
	}
}

/// The channels `user_id` routed `kind` to.
///
/// Precedence per channel: an exact-kind rule (enabled or not) wins over an
/// [`ANY_EVENT`] rule; with neither, nothing is delivered. Disabled rules are
/// therefore how a user mutes one kind on a channel that otherwise receives
/// everything. [`NotificationKind::Test`] never matches.
pub fn resolve_channels<'a>(
	rules: &'a [Rule],
	user_id: &str,
	kind: NotificationKind,
) -> Vec<&'a str> {
	if !kind.routable() {
		return Vec::new();
	}
	let kind_name = kind.to_string();
	let mut exact: BTreeSet<&str> = BTreeSet::new();
	let mut enabled: BTreeSet<&str> = BTreeSet::new();
	for rule in rules.iter().filter(|rule| rule.user_id == user_id) {
		if rule.event_kind == kind_name {
			exact.insert(&rule.channel_id);
			if rule.enabled {
				enabled.insert(&rule.channel_id);
			} else {
				enabled.remove(rule.channel_id.as_str());
			}
		}
	}
	for rule in rules.iter().filter(|rule| rule.user_id == user_id) {
		if rule.event_kind == ANY_EVENT
			&& rule.enabled
			&& !exact.contains(rule.channel_id.as_str())
		{
			enabled.insert(&rule.channel_id);
		}
	}
	enabled.into_iter().collect()
}

/// Every delivery for `kind` given the persisted rules and the event's audience.
pub fn resolve_targets(
	rules: &[Rule],
	audience: &Audience,
	kind: NotificationKind,
) -> Vec<Target> {
	let users: Vec<&str> = match audience {
		Audience::Users(users) => users.iter().map(String::as_str).collect(),
		Audience::Subscribers => rules
			.iter()
			.map(|rule| rule.user_id.as_str())
			.collect::<BTreeSet<_>>()
			.into_iter()
			.collect(),
	};
	let mut targets = Vec::new();
	for user_id in users {
		for channel_id in resolve_channels(rules, user_id, kind) {
			targets.push(Target {
				user_id: user_id.to_string(),
				channel_id: channel_id.to_string(),
			});
		}
	}
	targets.sort();
	targets.dedup();
	targets
}

#[cfg(test)]
mod tests {
	use super::*;

	fn rule(user: &str, kind: &str, channel: &str, enabled: bool) -> Rule {
		Rule {
			user_id: user.into(),
			event_kind: kind.into(),
			channel_id: channel.into(),
			enabled,
		}
	}

	#[test]
	fn no_rule_means_no_delivery() {
		let rules = [rule("a", "SCAN_FINISHED", "ntfy", true)];
		assert!(resolve_channels(&rules, "a", NotificationKind::DevicePaired).is_empty());
		assert!(resolve_channels(&rules, "b", NotificationKind::ScanFinished).is_empty());
	}

	#[test]
	fn exact_disabled_rule_mutes_a_wildcard() {
		let rules = [
			rule("a", ANY_EVENT, "ntfy", true),
			rule("a", ANY_EVENT, "email", true),
			rule("a", "SCAN_FINISHED", "email", false),
		];
		assert_eq!(
			resolve_channels(&rules, "a", NotificationKind::ScanFinished),
			vec!["ntfy"]
		);
		assert_eq!(
			resolve_channels(&rules, "a", NotificationKind::DevicePaired),
			vec!["email", "ntfy"]
		);
	}

	#[test]
	fn disabled_wildcard_does_not_suppress_exact_rule() {
		let rules = [
			rule("a", ANY_EVENT, "ntfy", false),
			rule("a", "DEVICE_PAIRED", "ntfy", true),
		];
		assert_eq!(
			resolve_channels(&rules, "a", NotificationKind::DevicePaired),
			vec!["ntfy"]
		);
		assert!(resolve_channels(&rules, "a", NotificationKind::ScanFinished).is_empty());
	}

	#[test]
	fn test_kind_is_never_routed() {
		let rules = [
			rule("a", ANY_EVENT, "ntfy", true),
			rule("a", "TEST", "ntfy", true),
		];
		assert!(resolve_channels(&rules, "a", NotificationKind::Test).is_empty());
	}

	#[test]
	fn subscribers_audience_covers_every_opted_in_user() {
		let rules = [
			rule("a", "SCAN_FINISHED", "ntfy", true),
			rule("b", ANY_EVENT, "webhook", true),
			rule("c", "SCAN_FINISHED", "ntfy", false),
		];
		let targets = resolve_targets(
			&rules,
			&Audience::Subscribers,
			NotificationKind::ScanFinished,
		);
		assert_eq!(
			targets,
			vec![
				Target {
					user_id: "a".into(),
					channel_id: "ntfy".into()
				},
				Target {
					user_id: "b".into(),
					channel_id: "webhook".into()
				},
			]
		);
	}

	#[test]
	fn users_audience_ignores_other_subscribers() {
		let rules = [
			rule("a", "DEVICE_PAIRED", "ntfy", true),
			rule("b", "DEVICE_PAIRED", "ntfy", true),
		];
		let targets =
			resolve_targets(&rules, &Audience::user("b"), NotificationKind::DevicePaired);
		assert_eq!(
			targets,
			vec![Target {
				user_id: "b".into(),
				channel_id: "ntfy".into()
			}]
		);
	}
}
