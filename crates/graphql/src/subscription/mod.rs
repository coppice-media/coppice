mod device;
mod event;
mod ingest;
mod log;
mod provider;

use device::DeviceSubscription;
use event::EventSubscription;
use ingest::IngestSubscription;
use log::LogSubscription;
use provider::ProviderSubscription;

#[derive(async_graphql::MergedSubscription, Default)]
pub struct Subscription(
	LogSubscription,
	EventSubscription,
	IngestSubscription,
	DeviceSubscription,
	ProviderSubscription,
);
