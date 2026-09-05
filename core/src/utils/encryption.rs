use crate::{CoreError, CoreResult};
use models::entity::server_config;
use sea_orm::{DatabaseConnection, EntityTrait, SelectColumns};
use simple_crypt::{decrypt, encrypt};

/// Retrieves the encryption key from the persisted server configuration
pub async fn fetch_encryption_key(conn: &DatabaseConnection) -> CoreResult<String> {
	let record = server_config::Entity::find()
		.select_column(server_config::Column::EncryptionKey)
		.one(conn)
		.await?;

	record
		.and_then(|config| config.encryption_key)
		.ok_or(CoreError::EncryptionKeyNotSet)
}

pub fn create_encryption_key() -> CoreResult<String> {
	let random_bytes = rand::random::<[u8; 32]>();

	Ok(data_encoding::BASE64.encode(&random_bytes))
}

pub fn encrypt_string(str: &str, encryption_key: &String) -> CoreResult<String> {
	let encrypted_bytes = encrypt(str.as_bytes(), encryption_key.as_bytes())
		.map_err(|e| CoreError::EncryptionFailed(e.to_string()))?;
	Ok(data_encoding::BASE64.encode(&encrypted_bytes))
}

pub fn decrypt_string(
	encrypted_str: &str,
	encryption_key: &String,
) -> CoreResult<String> {
	let encrypted_bytes = data_encoding::BASE64
		.decode(encrypted_str.as_bytes())
		.map_err(|e| CoreError::DecryptionFailed(e.to_string()))?;
	let decrypted_bytes = decrypt(&encrypted_bytes, encryption_key.as_bytes())
		.map_err(|e| CoreError::DecryptionFailed(e.to_string()))?;
	String::from_utf8(decrypted_bytes)
		.map_err(|e| CoreError::DecryptionFailed(e.to_string()))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_create_encryption_key() {
		let key = create_encryption_key();
		assert!(key.is_ok(), "Failed to create key: {:?}", key.err());
	}

	#[test]
	fn test_encrypt_decrypt_string() {
		let encryption_key = create_encryption_key().unwrap();

		// Do a round-trip test
		let test_string = "Test data";
		let encrypted = encrypt_string(test_string, &encryption_key).unwrap();
		let decrypted = decrypt_string(&encrypted, &encryption_key).unwrap();
		assert_eq!(decrypted, test_string);
	}

	#[test]
	fn test_incorrect_encryption_key() {
		let correct_key = create_encryption_key().unwrap();
		let incorrect_key = create_encryption_key().unwrap();

		// Do a (failing) decryption test
		let test_string = "Test data";
		let encrypted = encrypt_string(test_string, &correct_key).unwrap();
		let res = decrypt_string(&encrypted, &incorrect_key);
		assert!(res.is_err());
	}
}
