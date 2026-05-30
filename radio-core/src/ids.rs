use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_type!(QsoId);
id_type!(OperatorId);
id_type!(StationLocationId);
id_type!(OperatingSessionId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct() {
        let a = QsoId::new();
        let b = QsoId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn ids_roundtrip_through_uuid() {
        let id = QsoId::new();
        let uuid = id.as_uuid();
        assert_eq!(id, QsoId::from_uuid(uuid));
    }
}
