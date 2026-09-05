mod device;
mod event;
mod ingest;
mod log;

use device::DeviceSubscription;
use event::EventSubscription;
use ingest::IngestSubscription;
use log::LogSubscription;

#[derive(async_graphql::MergedSubscription, Default)]
pub struct Subscription(
	LogSubscription,
	EventSubscription,
	IngestSubscription,
	DeviceSubscription,
);
