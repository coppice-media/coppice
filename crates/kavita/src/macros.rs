/// Declares an integer-backed Kavita enum that serializes as its discriminant
/// (Kavita renders every enum as its numeric value).
macro_rules! int_enum {
	($(#[$meta:meta])* $name:ident { $($variant:ident = $value:expr),+ $(,)? }) => {
		$(#[$meta])*
		#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
		pub enum $name {
			$($variant),+
		}

		impl $name {
			pub const ALL: &'static [$name] = &[$($name::$variant),+];

			pub fn value(self) -> i32 {
				match self {
					$($name::$variant => $value),+
				}
			}
		}

		impl From<$name> for i32 {
			fn from(value: $name) -> i32 {
				value.value()
			}
		}

		impl TryFrom<i32> for $name {
			type Error = i32;

			fn try_from(value: i32) -> Result<Self, i32> {
				match value {
					$(v if v == $value => Ok($name::$variant),)+
					other => Err(other),
				}
			}
		}

		impl serde::Serialize for $name {
			fn serialize<S: serde::Serializer>(
				&self,
				serializer: S,
			) -> Result<S::Ok, S::Error> {
				serializer.serialize_i32(self.value())
			}
		}

		impl<'de> serde::Deserialize<'de> for $name {
			fn deserialize<D: serde::Deserializer<'de>>(
				deserializer: D,
			) -> Result<Self, D::Error> {
				let value = <i32 as serde::Deserialize>::deserialize(deserializer)?;
				$name::try_from(value).map_err(|value| {
					<D::Error as serde::de::Error>::custom(format!(
						"{value} is not a valid {}",
						stringify!($name)
					))
				})
			}
		}
	};
}
