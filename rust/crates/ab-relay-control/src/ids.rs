use crate::{Error, Result, decode};
use rusqlite::{
    ToSql,
    types::{ToSqlOutput, ValueRef},
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AccountId(String);
impl AccountId {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        if decode(value)?.len() != 16 {
            return Err(Error::Invalid);
        }
        Ok(Self(value.to_owned()))
    }
}
impl ToSql for AccountId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(ValueRef::Text(self.0.as_bytes())))
    }
}
impl std::ops::Deref for AccountId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeviceId(String);
impl DeviceId {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        if value.len() != 24
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(Error::Invalid);
        }
        Ok(Self(value.to_owned()))
    }
}
impl ToSql for DeviceId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Borrowed(ValueRef::Text(self.0.as_bytes())))
    }
}
impl std::ops::Deref for DeviceId {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceAction {
    Suspend,
    Resume,
    Rotate,
    Revoke,
}
impl DeviceAction {
    pub(crate) fn parse(value: &str) -> Result<Self> {
        match value {
            "suspend" => Ok(Self::Suspend),
            "resume" => Ok(Self::Resume),
            "rotate" => Ok(Self::Rotate),
            "revoke" => Ok(Self::Revoke),
            _ => Err(Error::Invalid),
        }
    }
    pub(crate) fn transition(self, current: &str) -> Result<&str> {
        if current == "revoked" {
            return Err(Error::Forbidden);
        }
        if !["active", "suspended"].contains(&current) {
            return Err(Error::Unavailable);
        }
        Ok(match self {
            Self::Suspend => "suspended",
            Self::Resume => "active",
            Self::Revoke => "revoked",
            Self::Rotate => current,
        })
    }
}
