use sea_orm::{
	prelude::*,
	sea_query::{ArrayType, Nullable, ValueType, ValueTypeErr},
	ColIdx, DeriveActiveEnum, EnumIter, QueryResult, TryGetError, TryGetableFromJson,
	Value,
};
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};
use strum::{Display, EnumString};

fn default_true() -> bool {
	true
}

#[derive(
	Eq,
	Copy,
	Hash,
	Debug,
	Clone,
	EnumIter,
	PartialEq,
	DeriveActiveEnum,
	EnumString,
	Display,
	Serialize,
	Deserialize,
)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[sea_orm(
	rs_type = "String",
	rename_all = "SCREAMING_SNAKE_CASE",
	db_type = "String(StringLen::None)"
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum SystemArrangement {
	Home,
	Explore,
	Libraries,
	SmartLists,
	BookClubs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
	feature = "graphql",
	derive(async_graphql::SimpleObject, async_graphql::InputObject)
)]
#[cfg_attr(
	feature = "graphql",
	graphql(input_name = "SystemArrangementConfigInput")
)]
pub struct SystemArrangementConfig {
	variant: SystemArrangement,
	#[cfg_attr(feature = "graphql", graphql(default))]
	links: Vec<FilterableArrangementEntityLink>,
}

#[derive(
	Eq,
	Copy,
	Default,
	Hash,
	Debug,
	Clone,
	EnumIter,
	PartialEq,
	DeriveActiveEnum,
	EnumString,
	Display,
	Serialize,
	Deserialize,
)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[sea_orm(
	rs_type = "String",
	rename_all = "SCREAMING_SNAKE_CASE",
	db_type = "String(StringLen::None)"
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum FilterableArrangementEntity {
	#[default]
	Books,
	Libraries,
	Series,
	SmartLists,
	BookClubs,
}

// TODO: Rename since I am now using this for both home and navigation arrangements.
#[derive(
	Eq,
	Copy,
	Hash,
	Debug,
	Clone,
	EnumIter,
	PartialEq,
	DeriveActiveEnum,
	EnumString,
	Display,
	Serialize,
	Deserialize,
)]
#[cfg_attr(feature = "graphql", derive(async_graphql::Enum))]
#[sea_orm(
	rs_type = "String",
	rename_all = "SCREAMING_SNAKE_CASE",
	db_type = "String(StringLen::None)"
)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[strum(serialize_all = "SCREAMING_SNAKE_CASE")]
pub enum FilterableArrangementEntityLink {
	Create,
	ShowAll,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
	feature = "graphql",
	derive(async_graphql::SimpleObject, async_graphql::InputObject)
)]
#[cfg_attr(
	feature = "graphql",
	graphql(input_name = "FilterableArrangementEntityLinkInput")
)]
pub struct CustomArrangementConfig {
	entity: FilterableArrangementEntity,
	name: Option<String>,
	// TODO(custom-arrangement): Support typed filters
	#[cfg(feature = "graphql")]
	filter: Option<async_graphql::Json<serde_json::Value>>,
	#[cfg(not(feature = "graphql"))]
	filter: Option<serde_json::Value>,
	order_by: Option<String>,
	#[cfg_attr(feature = "graphql", graphql(default))]
	links: Vec<FilterableArrangementEntityLink>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
	feature = "graphql",
	derive(async_graphql::SimpleObject, async_graphql::InputObject)
)]
#[cfg_attr(feature = "graphql", graphql(input_name = "InProgressBooksInput"))]
pub struct InProgressBooks {
	name: Option<String>,
	// filter: Option<Json<serde_json::Value>>,
	#[cfg_attr(feature = "graphql", graphql(default))]
	links: Vec<FilterableArrangementEntityLink>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
	feature = "graphql",
	derive(async_graphql::SimpleObject, async_graphql::InputObject)
)]
#[cfg_attr(feature = "graphql", graphql(input_name = "RecentlyAddedInput"))]
pub struct RecentlyAdded {
	entity: FilterableArrangementEntity,
	name: Option<String>,
	// filter: Option<Json<serde_json::Value>>,
	#[cfg_attr(feature = "graphql", graphql(default))]
	links: Vec<FilterableArrangementEntityLink>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
	feature = "graphql",
	derive(async_graphql::SimpleObject, async_graphql::InputObject)
)]
#[cfg_attr(feature = "graphql", graphql(input_name = "OnDeckBooksInput"))]
pub struct OnDeckBooks {
	name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
	feature = "graphql",
	derive(async_graphql::Union, async_graphql::OneofObject)
)]
#[cfg_attr(feature = "graphql", graphql(input_name = "ArrangementConfigInput"))]
pub enum ArrangementConfig {
	System(SystemArrangementConfig),
	InProgressBooks(InProgressBooks),
	OnDeckBooks(OnDeckBooks),
	RecentlyAdded(RecentlyAdded),
	Custom(CustomArrangementConfig),
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum TaggedArrangementConfigRef<'a> {
	System(&'a SystemArrangementConfig),
	InProgressBooks(&'a InProgressBooks),
	OnDeckBooks(&'a OnDeckBooks),
	RecentlyAdded(&'a RecentlyAdded),
	Custom(&'a CustomArrangementConfig),
}

impl Serialize for ArrangementConfig {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let tagged = match self {
			Self::System(config) => TaggedArrangementConfigRef::System(config),
			Self::InProgressBooks(config) => {
				TaggedArrangementConfigRef::InProgressBooks(config)
			},
			Self::OnDeckBooks(config) => TaggedArrangementConfigRef::OnDeckBooks(config),
			Self::RecentlyAdded(config) => {
				TaggedArrangementConfigRef::RecentlyAdded(config)
			},
			Self::Custom(config) => TaggedArrangementConfigRef::Custom(config),
		};
		tagged.serialize(serializer)
	}
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum TaggedArrangementConfig {
	System(SystemArrangementConfig),
	InProgressBooks(InProgressBooks),
	OnDeckBooks(OnDeckBooks),
	RecentlyAdded(RecentlyAdded),
	Custom(CustomArrangementConfig),
}

impl From<TaggedArrangementConfig> for ArrangementConfig {
	fn from(config: TaggedArrangementConfig) -> Self {
		match config {
			TaggedArrangementConfig::System(config) => Self::System(config),
			TaggedArrangementConfig::InProgressBooks(config) => {
				Self::InProgressBooks(config)
			},
			TaggedArrangementConfig::OnDeckBooks(config) => Self::OnDeckBooks(config),
			TaggedArrangementConfig::RecentlyAdded(config) => Self::RecentlyAdded(config),
			TaggedArrangementConfig::Custom(config) => Self::Custom(config),
		}
	}
}

impl<'de> Deserialize<'de> for ArrangementConfig {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		let value = serde_json::Value::deserialize(deserializer)?;
		let config = if value.get("type").is_some() {
			serde_json::from_value::<TaggedArrangementConfig>(value).map(Into::into)
		} else if value.get("variant").is_some() {
			serde_json::from_value::<SystemArrangementConfig>(value).map(Self::System)
		} else if value.get("entity").is_some() {
			if value.get("filter").is_some() || value.get("order_by").is_some() {
				serde_json::from_value::<CustomArrangementConfig>(value).map(Self::Custom)
			} else {
				serde_json::from_value::<RecentlyAdded>(value).map(Self::RecentlyAdded)
			}
		} else if value.get("links").is_some() {
			serde_json::from_value::<InProgressBooks>(value).map(Self::InProgressBooks)
		} else {
			serde_json::from_value::<OnDeckBooks>(value).map(Self::OnDeckBooks)
		};
		config.map_err(|error| DeError::custom(error.to_string()))
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
	feature = "graphql",
	derive(async_graphql::SimpleObject, async_graphql::InputObject)
)]
#[cfg_attr(feature = "graphql", graphql(input_name = "ArrangementSectionInput"))]
pub struct ArrangementSection {
	config: ArrangementConfig,
	#[serde(default = "default_true")]
	#[cfg_attr(feature = "graphql", graphql(default_with = "default_true()"))]
	visible: bool,
}

/// The sections displayed on a user's home page.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct HomeArrangement {
	pub sections: Vec<ArrangementSection>,
}

impl ArrangementSection {
	fn home_key(&self) -> Option<&'static str> {
		match &self.config {
			ArrangementConfig::InProgressBooks(_) => Some("continueReading"),
			ArrangementConfig::OnDeckBooks(_) => Some("onDeck"),
			ArrangementConfig::RecentlyAdded(config) => match config.entity {
				FilterableArrangementEntity::Books => Some("recentlyAddedBooks"),
				FilterableArrangementEntity::Series => Some("recentlyAddedSeries"),
				_ => None,
			},
			_ => None,
		}
	}
}

impl HomeArrangement {
	pub fn new(sections: Vec<ArrangementSection>) -> Self {
		let mut seen = std::collections::HashSet::new();
		let mut sections: Vec<_> = sections
			.into_iter()
			.filter(|section| section.home_key().is_some_and(|key| seen.insert(key)))
			.collect();

		for mut section in Arrangement::default_home().sections {
			if seen.insert(section.home_key().unwrap()) {
				section.visible = false;
				sections.push(section);
			}
		}
		Self { sections }
	}
}

impl From<HomeArrangement> for Arrangement {
	fn from(home: HomeArrangement) -> Self {
		Self {
			// TODO(arrangement): delete this field and usages
			locked: false,
			sections: home.sections,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct Arrangement {
	pub locked: bool,
	pub sections: Vec<ArrangementSection>,
}
impl TryGetableFromJson for Arrangement {
	fn try_get_from_json<I: ColIdx>(
		res: &QueryResult,
		idx: I,
	) -> Result<Self, TryGetError> {
		let json = res.try_get_by_nullable::<serde_json::Value, _>(idx)?;
		// Treat unreadable stored configurations as absent, so each caller can use
		// its home or navigation default without losing the other preferences.
		serde_json::from_value(json).map_err(|_| TryGetError::Null(format!("{idx:?}")))
	}
}

impl From<Arrangement> for Value {
	fn from(arrangement: Arrangement) -> Self {
		serde_json::to_value(arrangement)
			.expect("Failed to serialize Arrangement")
			.into()
	}
}

impl ValueType for Arrangement {
	fn try_from(value: Value) -> Result<Self, ValueTypeErr> {
		serde_json::from_value(<serde_json::Value as ValueType>::try_from(value)?)
			.map_err(|_| ValueTypeErr)
	}

	fn type_name() -> String {
		stringify!(Arrangement).to_owned()
	}

	fn array_type() -> ArrayType {
		ArrayType::Json
	}

	fn column_type() -> ColumnType {
		ColumnType::Json
	}
}

impl Nullable for Arrangement {
	fn null() -> Value {
		Value::Json(None)
	}
}

impl Arrangement {
	pub fn default_home() -> Arrangement {
		Arrangement {
			locked: false,
			sections: vec![
				ArrangementSection {
					config: ArrangementConfig::InProgressBooks(InProgressBooks::default()),
					visible: true,
				},
				ArrangementSection {
					config: ArrangementConfig::OnDeckBooks(OnDeckBooks::default()),
					visible: true,
				},
				ArrangementSection {
					config: ArrangementConfig::RecentlyAdded(RecentlyAdded {
						entity: FilterableArrangementEntity::Books,
						..Default::default()
					}),
					visible: true,
				},
				ArrangementSection {
					config: ArrangementConfig::RecentlyAdded(RecentlyAdded {
						entity: FilterableArrangementEntity::Series,
						..Default::default()
					}),
					visible: true,
				},
			],
		}
	}

	pub fn default_navigation() -> Arrangement {
		Arrangement {
			locked: true,
			sections: vec![
				ArrangementSection {
					config: ArrangementConfig::System(SystemArrangementConfig {
						variant: SystemArrangement::Home,
						links: vec![],
					}),
					visible: true,
				},
				ArrangementSection {
					config: ArrangementConfig::System(SystemArrangementConfig {
						variant: SystemArrangement::Explore,
						links: vec![],
					}),
					visible: true,
				},
				ArrangementSection {
					config: ArrangementConfig::System(SystemArrangementConfig {
						variant: SystemArrangement::Libraries,
						links: vec![FilterableArrangementEntityLink::Create],
					}),
					visible: true,
				},
				ArrangementSection {
					config: ArrangementConfig::System(SystemArrangementConfig {
						variant: SystemArrangement::SmartLists,
						links: vec![FilterableArrangementEntityLink::Create],
					}),
					visible: true,
				},
				ArrangementSection {
					config: ArrangementConfig::System(SystemArrangementConfig {
						variant: SystemArrangement::BookClubs,
						links: vec![FilterableArrangementEntityLink::Create],
					}),
					visible: true,
				},
			],
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use sea_orm::{ConnectionTrait, Database, DatabaseBackend, Statement, TryGetable};

	#[test]
	fn stored_arrangements_decode_without_losing_valid_values() {
		tokio_test::block_on(async {
			let db = Database::connect("sqlite::memory:").await.unwrap();
			let home = Arrangement::default_home();
			let navigation = Arrangement::default_navigation();
			let cases = [
				(home.clone().into(), Some(home)),
				(navigation.clone().into(), Some(navigation)),
				(<Arrangement as Nullable>::null(), None),
				(serde_json::json!(null).into(), None),
				(serde_json::json!({"sections": []}).into(), None),
				(
					serde_json::json!({"locked": false, "sections": [
						{"config": {"type": "Unknown"}}
					]})
					.into(),
					None,
				),
			];
			for (value, expected) in cases {
				let row = db
					.query_one(Statement::from_sql_and_values(
						DatabaseBackend::Sqlite,
						"SELECT ? AS arrangement",
						[value],
					))
					.await
					.unwrap()
					.unwrap();
				assert_eq!(
					row.try_get::<Option<Arrangement>>("", "arrangement")
						.unwrap(),
					expected
				);
				assert_eq!(
					row.try_get_by_index::<Option<Arrangement>>(0).unwrap(),
					expected
				);
				assert!(matches!(
					Arrangement::try_get_by(&row, "missing_column"),
					Err(TryGetError::DbErr(_))
				));
			}
		});
	}

	#[test]
	fn home_arrangement_round_trip_preserves_sections() {
		let arrangement = Arrangement::default_home();
		let json = serde_json::to_value(&arrangement).unwrap();
		let restored: Arrangement = serde_json::from_value(json).unwrap();
		assert_eq!(restored, arrangement);
	}

	#[test]
	fn legacy_arrangement_configs_keep_their_distinct_variants() {
		let recently_added = serde_json::from_value::<ArrangementConfig>(
			serde_json::json!({"entity": "BOOKS", "name": null, "links": []}),
		)
		.unwrap();
		assert!(matches!(
			recently_added,
			ArrangementConfig::RecentlyAdded(_)
		));

		let custom = serde_json::from_value::<ArrangementConfig>(serde_json::json!({
			"entity": "BOOKS",
			"name": null,
			"filter": null,
			"order_by": null,
			"links": []
		}))
		.unwrap();
		assert!(matches!(custom, ArrangementConfig::Custom(_)));
	}

	#[test]
	fn unknown_config_tag_errors() {
		assert!(serde_json::from_value::<ArrangementConfig>(
			serde_json::json!({"type": "Unknown"})
		)
		.is_err());
	}

	#[test]
	fn home_sections_are_normalized_without_changing_valid_order_or_visibility() {
		let defaults = Arrangement::default_home().sections;
		let mut hidden_books = defaults[2].clone();
		hidden_books.visible = false;
		let unsupported_entity = ArrangementSection {
			config: ArrangementConfig::RecentlyAdded(RecentlyAdded {
				entity: FilterableArrangementEntity::Libraries,
				..Default::default()
			}),
			visible: true,
		};
		let home = HomeArrangement::new(vec![
			defaults[3].clone(),
			hidden_books.clone(),
			defaults[2].clone(),
			Arrangement::default_navigation().sections[0].clone(),
			unsupported_entity,
		]);
		assert_eq!(home.sections.len(), 4);
		assert_eq!(home.sections[0], defaults[3]);
		assert_eq!(home.sections[1], hidden_books);
		assert_eq!(home.sections[2].home_key(), Some("continueReading"));
		assert_eq!(home.sections[3].home_key(), Some("onDeck"));
		assert!(!home.sections[2].visible);
		assert!(!home.sections[3].visible);
		assert_eq!(
			HomeArrangement::new(home.sections.clone()).sections,
			home.sections
		);
	}

	#[test]
	fn empty_home_sections_are_filled_as_hidden() {
		let home = HomeArrangement::new(vec![]);
		assert_eq!(home.sections.len(), 4);
		assert!(home.sections.iter().all(|section| !section.visible));
	}
}
