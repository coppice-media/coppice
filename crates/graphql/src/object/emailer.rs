use async_graphql::{ComplexObject, Context, Result, SimpleObject};

use models::entity::{emailer, emailer_send_record};
use sea_orm::prelude::*;

use crate::data::CoreContext;

use super::emailer_send_record::EmailerSendRecord;

#[derive(Debug, SimpleObject)]
#[graphql(complex)]
pub struct Emailer {
	#[graphql(flatten)]
	pub model: emailer::Model,
}

impl From<emailer::Model> for Emailer {
	fn from(entity: emailer::Model) -> Self {
		Self { model: entity }
	}
}
/// Server SMTP metadata. Passwords are intentionally absent; this object is
/// only readable by server owners/ManageServer.
#[derive(Debug, Clone, SimpleObject)]
pub struct SmtpSettings {
	pub configured: bool,
	pub sender_email: Option<String>,
	pub sender_display_name: Option<String>,
	pub smtp_host: Option<String>,
	pub smtp_port: Option<i32>,
	pub tls_enabled: Option<bool>,
	pub last_used_at: Option<DateTimeWithTimeZone>,
}

impl From<Option<emailer::Model>> for SmtpSettings {
	fn from(model: Option<emailer::Model>) -> Self {
		match model {
			Some(model) => Self {
				configured: true,
				sender_email: Some(model.sender_email),
				sender_display_name: Some(model.sender_display_name),
				smtp_host: Some(model.smtp_host),
				smtp_port: Some(model.smtp_port),
				tls_enabled: Some(model.tls_enabled),
				last_used_at: model.last_used_at,
			},
			None => Self {
				configured: false,
				sender_email: None,
				sender_display_name: None,
				smtp_host: None,
				smtp_port: None,
				tls_enabled: None,
				last_used_at: None,
			},
		}
	}
}

#[ComplexObject]
impl Emailer {
	async fn send_history(&self, ctx: &Context<'_>) -> Result<Vec<EmailerSendRecord>> {
		let conn = ctx.data::<CoreContext>()?.conn.as_ref();

		let records = emailer_send_record::Entity::find()
			.filter(emailer_send_record::Column::EmailerId.eq(self.model.id))
			.all(conn)
			.await?;

		Ok(records.into_iter().map(EmailerSendRecord::from).collect())
	}
}
