//! Enums of names. Each variant has one name, as platform.db, the platform
//! API and URLs write it; `ALL` and `NAMES` come from the same list as
//! `as_str` and `parse`, so they can't drift apart.

/// An enum whose variants are written as names: `Variant = "name",` each.
macro_rules! names {
    ($(#[$meta:meta])* $vis:vis enum $name:ident { $($(#[$vmeta:meta])* $variant:ident = $text:literal,)+ }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        $vis enum $name {
            $($(#[$vmeta])* $variant,)+
        }

        #[allow(dead_code)]
        impl $name {
            /// Every variant, in order.
            pub const ALL: [$name; [$($text),+].len()] = [$($name::$variant),+];
            /// Every name, in the order of `ALL`.
            pub const NAMES: [&'static str; [$($text),+].len()] = [$($text),+];

            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)+
                }
            }

            pub fn parse(s: &str) -> Option<Self> {
                match s {
                    $($text => Some($name::$variant),)+
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.pad(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Self::parse(&s).ok_or_else(|| serde::de::Error::unknown_variant(&s, &Self::NAMES))
            }
        }

        impl rusqlite::ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
                Ok(self.as_str().into())
            }
        }

        impl rusqlite::types::FromSql for $name {
            fn column_result(v: rusqlite::types::ValueRef<'_>) -> rusqlite::types::FromSqlResult<Self> {
                let s = v.as_str()?;
                Self::parse(s).ok_or_else(|| rusqlite::types::FromSqlError::Other(format!("unknown {}: {s:?}", stringify!($name)).into()))
            }
        }
    };
}

#[cfg(test)]
mod tests {
    names! {
        enum Color {
            Red = "red",
            DarkBlue = "dark-blue",
        }
    }

    #[test]
    fn names_round_trip() {
        for c in Color::ALL {
            assert_eq!(Color::parse(c.as_str()), Some(c));
            assert_eq!(serde_json::from_value::<Color>(serde_json::to_value(c).unwrap()).unwrap(), c);
        }
        assert_eq!(Color::NAMES, ["red", "dark-blue"]);
        assert_eq!(Color::parse("blue"), None);
        assert!(serde_json::from_str::<Color>("\"blue\"").is_err());
    }
}
