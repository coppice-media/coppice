//! The one funnel every library/series/book query passes through to decide
//! what an authenticated request may see.
//!
//! Two independent narrowings compose here:
//!
//! 1. the user's own [`library_exclusion`](crate::entity::library_exclusion)
//!    rows, which hide libraries from that user everywhere; and
//! 2. the library scope of the *device* whose credential authenticated the
//!    request (see `devices.library_scope`), which is resolved once per
//!    request onto [`AuthUser::device_library_scope`].
//!
//! They are combined with `AND`, never `OR`: a device scope can only narrow
//! what its user already sees. A session user has no device, so the scope is
//! `None` and the query is exactly the pre-device-scope one.

use sea_orm::{sea_query::IntoCondition, ColumnTrait, Condition};

use crate::entity::{library_exclusion, user::AuthUser};

/// What one request may see: its user, plus the library ids its device is
/// restricted to (`None` = inherit the user's visibility).
///
/// Cheap to copy and to derive; every visibility helper takes
/// `impl Into<VisibilityScope>` so both an `&AuthUser` (the ordinary path,
/// which carries the request's device scope) and an explicit scope work.
#[derive(Clone, Copy, Debug)]
pub struct VisibilityScope<'a> {
	user: &'a AuthUser,
	device_libraries: Option<&'a [String]>,
}

impl<'a> VisibilityScope<'a> {
	/// The scope the request resolved to: the user's visibility narrowed by
	/// the authenticating device's library scope, if it has one.
	pub fn for_user(user: &'a AuthUser) -> Self {
		Self {
			user,
			device_libraries: user.device_library_scope.as_deref(),
		}
	}

	/// The user's own visibility, ignoring any device scope.
	///
	/// Only for the few places that must reason about what the *user* owns
	/// rather than what the device currently sees — e.g. describing a book
	/// that just left a Kobo's scope so the device can drop it.
	pub fn inherit(user: &'a AuthUser) -> Self {
		Self {
			user,
			device_libraries: None,
		}
	}

	/// An explicit scope, e.g. when replaying a device's visibility outside a
	/// request.
	pub fn scoped(user: &'a AuthUser, device_libraries: &'a [String]) -> Self {
		Self {
			user,
			device_libraries: Some(device_libraries),
		}
	}

	pub fn user(&self) -> &'a AuthUser {
		self.user
	}

	/// The library ids the device is restricted to, or `None` when it inherits.
	pub fn device_libraries(&self) -> Option<&'a [String]> {
		self.device_libraries
	}

	/// The visibility predicate for a column holding a library id
	/// (`libraries.id`, `series.library_id`).
	///
	/// Exclusions and device scope are `AND`ed, so the device scope is an
	/// intersection and can never reveal a library the user has hidden. An
	/// empty device scope matches nothing, which is what a device restricted
	/// to zero libraries should see.
	pub fn library_condition<C: ColumnTrait>(&self, column: C) -> Condition {
		let condition = column
			.not_in_subquery(library_exclusion::Entity::library_hidden_to_user_query(
				self.user,
			))
			.into_condition();
		match self.device_libraries {
			Some(ids) => condition.add(column.is_in(ids.iter().map(String::as_str))),
			None => condition,
		}
	}
}

impl<'a> From<&'a AuthUser> for VisibilityScope<'a> {
	fn from(user: &'a AuthUser) -> Self {
		Self::for_user(user)
	}
}

impl<'a> From<&VisibilityScope<'a>> for VisibilityScope<'a> {
	fn from(scope: &VisibilityScope<'a>) -> Self {
		*scope
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{entity::library, tests::common::*};
	use pretty_assertions::assert_eq;
	use sea_orm::{EntityTrait, QueryFilter, QueryTrait};

	fn filter_sql(scope: VisibilityScope<'_>) -> String {
		library::Entity::find()
			.filter(scope.library_condition(library::Column::Id))
			.build(sea_orm::DatabaseBackend::Sqlite)
			.to_string()
	}

	#[test]
	fn inherit_is_exclusions_only() {
		let user = get_default_user();
		assert_eq!(
			filter_sql(VisibilityScope::inherit(&user)),
			r#"SELECT "libraries"."id", "libraries"."name", "libraries"."description", "libraries"."path", "libraries"."status", "libraries"."thumbnail_meta", "libraries"."thumbnail_path", "libraries"."created_at", "libraries"."updated_at", "libraries"."emoji", "libraries"."config_id", "libraries"."last_scanned_at", "libraries"."source_provider" FROM "libraries" WHERE "libraries"."id" NOT IN (SELECT "library_id" FROM "library_exclusions" WHERE "library_exclusions"."user_id" = '42')"#
		);
	}

	#[test]
	fn device_scope_intersects_exclusions() {
		let user = get_default_user();
		let ids = ["lib-1".to_string(), "lib-2".to_string()];
		let sql = filter_sql(VisibilityScope::scoped(&user, &ids));
		assert!(
			sql.contains(
				r#""libraries"."id" NOT IN (SELECT "library_id" FROM "library_exclusions""#
			),
			"exclusions must survive a device scope: {sql}"
		);
		assert!(
			sql.contains(r#"AND "libraries"."id" IN ('lib-1', 'lib-2')"#),
			"device scope must be ANDed in: {sql}"
		);
	}

	#[test]
	fn empty_device_scope_matches_nothing() {
		let user = get_default_user();
		let sql = filter_sql(VisibilityScope::scoped(&user, &[]));
		// sea-query renders an empty `IN` as the always-false `1 = 2`.
		assert!(
			sql.contains("AND 1 = 2"),
			"an empty scope must not degrade to unfiltered: {sql}"
		);
	}

	#[test]
	fn for_user_picks_up_the_requests_device_scope() {
		let mut user = get_default_user();
		user.device_library_scope = Some(vec!["lib-1".to_string()]);
		let sql = filter_sql(VisibilityScope::for_user(&user));
		assert!(
			sql.contains(r#"AND "libraries"."id" IN ('lib-1')"#),
			"the resolved device scope must reach the query: {sql}"
		);
	}
}
