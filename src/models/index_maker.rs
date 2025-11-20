use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexMakerInfoResponse {
    pub total_volume: String,
    pub total_managed: String,
}
