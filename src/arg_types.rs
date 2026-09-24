use std::{ops::Deref, str::FromStr};

#[derive(Clone)]
pub struct IdentifierToken(String);

impl IdentifierToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for IdentifierToken {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err("The value is empty".to_string());
        }
        Ok(Self(value.to_string()))
    }
}

impl Deref for IdentifierToken {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}
