//! Regression guard: feature-gated scope leaves must survive the trip into a
//! crate that declares no features of its own. If this compiles, they did.

magic_map::magic_map_scope!();

use magic_map::magic_map;

pub mod db {
    #[derive(magic_map::MagicMap)]
    pub struct Row {
        pub id: uuid::Uuid,
        pub created_at: chrono::DateTime<chrono::Utc>,
        pub day: chrono::NaiveDate,
        pub amount: rust_decimal::Decimal,
        pub payload: serde_json::Value,
        pub count: i32,
    }
}

pub mod dtos {
    #[derive(magic_map::MagicMap)]
    pub struct RowResponse {
        pub id: String,                            // Uuid → String
        pub created_at: String,                    // DateTime<Utc> → String
        pub day: String,                           // NaiveDate → String
        pub amount: f64,                           // Decimal → f64
        pub payload: serde_json::Value,            // identity
        pub count: i64,                            // i32 → i64 widening
    }

    #[derive(magic_map::MagicMap)]
    pub struct PageResponse {
        pub rows: Vec<RowResponse>,
        pub first: Option<RowResponse>,
    }
}

pub mod db_page {
    #[derive(magic_map::MagicMap)]
    pub struct Page {
        pub rows: Vec<super::db::Row>,
        pub first: Option<super::db::Row>,
    }
}

// Every field here is a built-in leaf reached through the scope.
magic_map!(pub fn row_to_dto: db::Row => dtos::RowResponse);

// And the nested foreign→foreign pair, through Vec and Option — no overrides,
// which is the whole point: `row_to_dto`'s pair is visible to this mapping.
magic_map!(pub fn page_to_dto: db_page::Page => dtos::PageResponse);
