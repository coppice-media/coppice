use sea_orm::{prelude::*, DeriveActiveEnum, EnumIter, FromJsonQueryResult};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(
	feature = "graphql",
	derive(async_graphql::Union, async_graphql::OneofObject)
)]
#[cfg_attr(feature = "graphql", graphql(input_name = "ArrangementConfigInput"))]
#[serde(untagged)]
pub enum ArrangementConfig {
	System(SystemArrangementConfig),
	InProgressBooks(InProgressBooks),
	RecentlyAdded(RecentlyAdded),
	Custom(CustomArrangementConfig),
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

// TODO(graphql): There is enough distinction between sidebar/navigation and home arrangements that they should just be separate types.
// I'll aim to tackle this one I am closer to the end of the migration, as it is not the most important thing right now.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, FromJsonQueryResult)]
#[cfg_attr(feature = "graphql", derive(async_graphql::SimpleObject))]
pub struct Arrangement {
	pub locked: bool,
	pub sections: Vec<ArrangementSection>,
}

impl Arrangement {
	pub fn default_home() -> Arrangement {
		Arrangement {
			locked: true,
			sections: vec![
				ArrangementSection {
					config: ArrangementConfig::InProgressBooks(InProgressBooks::default()),
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
