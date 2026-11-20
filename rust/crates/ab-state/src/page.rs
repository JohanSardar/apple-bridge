use crate::{ErrorCode, Result};
use serde_json::Value;

#[derive(Debug)]
pub struct ReadPage {
    pub offset: usize,
    pub limit: usize,
}

impl ReadPage {
    pub fn limit(limit: u64) -> Result<usize> {
        match limit {
            0 => Ok(50),
            1..=1000 => usize::try_from(limit).map_err(|_| ErrorCode::Invalid.into()),
            _ => Err(ErrorCode::Invalid.into()),
        }
    }

    pub(crate) fn parse(query: &Value) -> Result<Self> {
        let number = |field| {
            if query[field].is_null() {
                Ok(0)
            } else {
                query[field].as_u64().ok_or(ErrorCode::Invalid)
            }
        };
        Ok(Self {
            offset: usize::try_from(number("offset")?).map_err(|_| ErrorCode::Invalid)?,
            limit: Self::limit(number("limit")?)?,
        })
    }

    pub(crate) fn end(&self) -> usize {
        self.offset.saturating_add(self.limit)
    }

    pub(crate) fn continuation(&self, total: usize, result: &mut Value) {
        if self.end() < total {
            result["next_offset"] = self.end().into();
        }
    }
}
