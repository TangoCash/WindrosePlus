use anyhow::Result;
use bson::{Bson, Document};
use serde::Serialize;

use crate::levels::{self, LevelEntry};
use crate::player_db;

const P_TOTAL_EXP: &str = "PlayerMetadata.PlayerProgression.TotalExp";
const P_REWARD_LEVEL: &str = "PlayerMetadata.PlayerProgression.RewardLevel";
const P_STAT_POINTS: &str = "PlayerMetadata.PlayerProgression.StatTree.ProgressionPoints";
const P_TALENT_POINTS: &str = "PlayerMetadata.PlayerProgression.TalentTree.ProgressionPoints";

const STAT_TREE_NODES: &str = "PlayerMetadata.PlayerProgression.StatTree.Nodes";
const TALENT_TREE_NODES: &str = "PlayerMetadata.PlayerProgression.TalentTree.Nodes";

#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    pub total_exp: i64,
    pub current_level: usize,
    pub earned_stat: i64,
    pub earned_talent: i64,
    pub recorded_reward_level: i64,
    pub recorded_stat_points: i64,
    pub recorded_talent_points: i64,
    pub spent_stat: i64,
    pub spent_talent: i64,
    pub drift_detected: bool,
    pub stat_nodes_allocated: u32,
    pub talent_nodes_allocated: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum Strategy {
    Safe,
    ForceReset,
}

impl Strategy {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "safe" => Ok(Self::Safe),
            "force-reset" => Ok(Self::ForceReset),
            other => anyhow::bail!("unknown strategy: {}", other),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::ForceReset => "force-reset",
        }
    }
}

pub fn diagnose(doc: &Document, levels: &[LevelEntry]) -> Result<Diagnosis> {
    let total_exp = player_db::get_i64(doc, P_TOTAL_EXP)?;
    let current_level = levels::current_level_index(levels, total_exp);
    let earned_stat = levels::sum_stat_rewards(levels, current_level);
    let earned_talent = levels::sum_talent_rewards(levels, current_level);

    let recorded_reward_level = player_db::get_i64(doc, P_REWARD_LEVEL)?;
    let recorded_stat_points = player_db::get_i64(doc, P_STAT_POINTS)?;
    let recorded_talent_points = player_db::get_i64(doc, P_TALENT_POINTS)?;

    let (spent_stat, stat_nodes_allocated) = count_spent(doc, STAT_TREE_NODES)?;
    let (spent_talent, talent_nodes_allocated) = count_spent(doc, TALENT_TREE_NODES)?;

    let accounted_stat_points = recorded_stat_points + spent_stat;
    let accounted_talent_points = recorded_talent_points + spent_talent;
    let drift_detected = recorded_reward_level != current_level as i64
        || accounted_stat_points != earned_stat
        || accounted_talent_points != earned_talent;

    Ok(Diagnosis {
        total_exp,
        current_level,
        earned_stat,
        earned_talent,
        recorded_reward_level,
        recorded_stat_points,
        recorded_talent_points,
        spent_stat,
        spent_talent,
        drift_detected,
        stat_nodes_allocated,
        talent_nodes_allocated,
    })
}

fn count_spent(doc: &Document, path: &str) -> Result<(i64, u32)> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = doc.get(parts[0]);
    for p in &parts[1..] {
        match cur {
            Some(Bson::Document(d)) => cur = d.get(*p),
            _ => anyhow::bail!("missing or invalid tree path: {}", path),
        }
    }
    let arr = match cur {
        Some(Bson::Array(a)) => a,
        _ => anyhow::bail!("tree path is not an array: {}", path),
    };
    let mut total: i64 = 0;
    let mut count: u32 = 0;
    for node in arr {
        let nd = match node {
            Bson::Document(nd) => nd,
            _ => anyhow::bail!("tree node is not a document: {}", path),
        };
        let level = match nd.get("NodeLevel") {
            Some(Bson::Int32(i)) => *i as i64,
            Some(Bson::Int64(i)) => *i,
            _ => anyhow::bail!("tree node has missing or invalid NodeLevel: {}", path),
        };
        if level > 0 {
            count += 1;
            let cost = match nd.get("NodeData") {
                Some(Bson::Document(nd_data)) => match nd_data.get("NodePointsCost") {
                    Some(Bson::Int32(i)) => *i as i64,
                    Some(Bson::Int64(i)) => *i,
                    _ => anyhow::bail!("tree node has missing or invalid NodePointsCost: {}", path),
                },
                _ => anyhow::bail!("tree node has missing or invalid NodeData: {}", path),
            };
            total += level * cost;
        }
    }
    Ok((total, count))
}

pub fn apply_heal(
    mut doc: Document,
    diag: &Diagnosis,
    strategy: Strategy,
) -> Result<(Document, String)> {
    if !diag.drift_detected {
        return Ok((doc, "no-op: no drift detected".to_string()));
    }

    let safe_no_spend = diag.spent_stat == 0
        && diag.spent_talent == 0
        && diag.stat_nodes_allocated == 0
        && diag.talent_nodes_allocated == 0;

    let case = if safe_no_spend {
        // Case A — safest path: just correct the three scalars
        player_db::set_path_i32(&mut doc, P_REWARD_LEVEL, diag.current_level as i32)?;
        player_db::set_path_i32(&mut doc, P_STAT_POINTS, diag.earned_stat as i32)?;
        player_db::set_path_i32(&mut doc, P_TALENT_POINTS, diag.earned_talent as i32)?;
        "A: no spent points, corrected three scalars".to_string()
    } else {
        match strategy {
            Strategy::Safe => {
                anyhow::bail!(
                    "spent or allocated progression nodes detected (stat_spent={}, talent_spent={}, stat_nodes={}, talent_nodes={}); refusing safe automatic repair",
                    diag.spent_stat,
                    diag.spent_talent,
                    diag.stat_nodes_allocated,
                    diag.talent_nodes_allocated
                );
            }
            Strategy::ForceReset => {
                zero_tree_nodes(&mut doc, STAT_TREE_NODES)?;
                zero_tree_nodes(&mut doc, TALENT_TREE_NODES)?;
                player_db::set_path_i32(&mut doc, P_REWARD_LEVEL, diag.current_level as i32)?;
                player_db::set_path_i32(&mut doc, P_STAT_POINTS, diag.earned_stat as i32)?;
                player_db::set_path_i32(&mut doc, P_TALENT_POINTS, diag.earned_talent as i32)?;
                "C: spent points reset to zero, all points refunded to ProgressionPoints"
                    .to_string()
            }
        }
    };

    Ok((doc, case))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{doc, Bson};

    fn node(level: i32, cost: i32) -> Bson {
        Bson::Document(doc! {
            "NodeLevel": level,
            "NodeData": {
                "NodePointsCost": cost,
            },
        })
    }

    fn player_doc(
        total_exp: i64,
        reward_level: i32,
        stat_points: i32,
        talent_points: i32,
        stat_nodes: Vec<Bson>,
        talent_nodes: Vec<Bson>,
    ) -> Document {
        doc! {
            "PlayerMetadata": {
                "PlayerProgression": {
                    "TotalExp": total_exp,
                    "RewardLevel": reward_level,
                    "StatTree": {
                        "ProgressionPoints": stat_points,
                        "Nodes": stat_nodes,
                    },
                    "TalentTree": {
                        "ProgressionPoints": talent_points,
                        "Nodes": talent_nodes,
                    },
                },
            },
        }
    }

    #[test]
    fn spent_but_accounted_points_are_not_drift() {
        let levels = levels::vanilla_da_hero_levels();
        let doc = player_doc(1200, 2, 5, 1, vec![node(3, 1)], vec![node(1, 1)]);

        let diag = diagnose(&doc, &levels).unwrap();

        assert_eq!(diag.current_level, 2);
        assert_eq!(diag.earned_stat, 8);
        assert_eq!(diag.earned_talent, 2);
        assert_eq!(diag.spent_stat, 3);
        assert_eq!(diag.spent_talent, 1);
        assert!(!diag.drift_detected);
    }

    #[test]
    fn no_spend_drift_is_safe_to_repair() {
        let levels = levels::vanilla_da_hero_levels();
        let doc = player_doc(1200, 0, 0, 0, vec![], vec![]);
        let diag = diagnose(&doc, &levels).unwrap();

        assert!(diag.drift_detected);

        let (repaired, case) = apply_heal(doc, &diag, Strategy::Safe).unwrap();
        let repaired_diag = diagnose(&repaired, &levels).unwrap();

        assert_eq!(case, "A: no spent points, corrected three scalars");
        assert_eq!(repaired_diag.recorded_reward_level, 2);
        assert_eq!(repaired_diag.recorded_stat_points, 8);
        assert_eq!(repaired_diag.recorded_talent_points, 2);
        assert!(!repaired_diag.drift_detected);
    }

    #[test]
    fn spent_drift_requires_manual_or_force_reset() {
        let levels = levels::vanilla_da_hero_levels();
        let doc = player_doc(1200, 2, 4, 1, vec![node(3, 1)], vec![node(1, 1)]);
        let diag = diagnose(&doc, &levels).unwrap();

        assert!(diag.drift_detected);
        assert!(apply_heal(doc, &diag, Strategy::Safe).is_err());
    }

    #[test]
    fn malformed_tree_shape_is_not_treated_as_no_spend() {
        let levels = levels::vanilla_da_hero_levels();
        let doc = doc! {
            "PlayerMetadata": {
                "PlayerProgression": {
                    "TotalExp": 1200_i64,
                    "RewardLevel": 0,
                    "StatTree": {
                        "ProgressionPoints": 0,
                    },
                    "TalentTree": {
                        "ProgressionPoints": 0,
                        "Nodes": [],
                    },
                },
            },
        };

        let err = diagnose(&doc, &levels).unwrap_err();
        assert!(format!("{err:?}").contains("tree path is not an array"));
    }

    #[test]
    fn allocated_zero_cost_nodes_refuse_safe_repair() {
        let levels = levels::vanilla_da_hero_levels();
        let doc = player_doc(1200, 0, 0, 0, vec![node(1, 0)], vec![]);
        let diag = diagnose(&doc, &levels).unwrap();

        assert!(diag.drift_detected);
        assert_eq!(diag.spent_stat, 0);
        assert_eq!(diag.stat_nodes_allocated, 1);

        let err = apply_heal(doc, &diag, Strategy::Safe).unwrap_err();
        assert!(format!("{err:?}").contains("allocated progression nodes"));
    }
}

fn zero_tree_nodes(doc: &mut Document, path: &str) -> Result<()> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur: &mut Document = doc;
    for p in &parts[..parts.len() - 1] {
        let next = cur
            .get_mut(*p)
            .ok_or_else(|| anyhow::anyhow!("missing: {}", p))?;
        cur = match next {
            Bson::Document(d) => d,
            _ => anyhow::bail!("path segment {} not a document", p),
        };
    }
    let leaf = parts.last().unwrap();
    let arr = match cur
        .get_mut(*leaf)
        .ok_or_else(|| anyhow::anyhow!("missing leaf: {}", leaf))?
    {
        Bson::Array(a) => a,
        _ => anyhow::bail!("{} is not an array", leaf),
    };
    for node in arr.iter_mut() {
        if let Bson::Document(nd) = node {
            if let Some(existing) = nd.get("NodeLevel") {
                let zero = match existing {
                    Bson::Int32(_) => Bson::Int32(0),
                    Bson::Int64(_) => Bson::Int64(0),
                    _ => continue,
                };
                nd.insert("NodeLevel".to_string(), zero);
            }
        }
    }
    Ok(())
}
