use sea_orm::DbErr;

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
	#[error("device not found")]
	NotFound,
	#[error("you do not have access to this device")]
	Forbidden,
	#[error("an API key cannot issue another API key with inherited permissions")]
	InheritedCredentialRequiresSession,
	#[error("device has been revoked")]
	Revoked,
	#[error("invalid device name: {0}")]
	InvalidName(String),
	#[error("invalid library scope: {0}")]
	InvalidScope(String),
	#[error("invalid Kindle address: {0}")]
	InvalidEmail(String),
	#[error("failed to mint credential: {0}")]
	Credential(String),
	#[error(transparent)]
	Database(#[from] DbErr),
}

pub type DeviceResult<T> = Result<T, DeviceError>;
