use anyhow::Result;
use bson::Document;
use serde::Serialize;

use crate::player_db;

#[derive(Debug, Serialize)]
pub struct Summary {
    pub key_hex: String,
    pub value_size: usize,
    pub value_sha256: String,
    pub player_name: Option<String>,
    pub guid: Option<String>,
    pub total_exp: i64,
    pub reward_level: i64,
    pub stat_progression_points: i64,
    pub talent_progression_points: i64,
}

impl Summary {
    pub fn from_doc(doc: &Document, key: &[u8], value_len: usize) -> Result<Self> {
        let player_name = player_db::get_string(doc, "PlayerName").ok();
        let guid = player_db::get_string(doc, "_guid").ok();
        let total_exp = player_db::get_i64(doc, "PlayerMetadata.PlayerProgression.TotalExp")?;
        let reward_level = player_db::get_i64(doc, "PlayerMetadata.PlayerProgression.RewardLevel")?;
        let stat = player_db::get_i64(
            doc,
            "PlayerMetadata.PlayerProgression.StatTree.ProgressionPoints",
        )?;
        let talent = player_db::get_i64(
            doc,
            "PlayerMetadata.PlayerProgression.TalentTree.ProgressionPoints",
        )?;

        // re-emit to compute the sha256 of canonical encoding
        let bytes = player_db::emit_bson(doc)?;

        Ok(Self {
            key_hex: hex::encode(key),
            value_size: value_len,
            value_sha256: player_db::sha_hex(&bytes),
            player_name,
            guid,
            total_exp,
            reward_level,
            stat_progression_points: stat,
            talent_progression_points: talent,
        })
    }
}
