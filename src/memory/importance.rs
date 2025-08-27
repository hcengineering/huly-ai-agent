// Copyright © 2025 Huly Labs. Use of this source code is governed by the MIT license.

use std::collections::HashMap;

use chrono::Utc;

use crate::memory::MemoryEntity;

pub(super) struct ImportanceCalculator {
    max_access_count: u32,
    max_relations_count: u32,
    decay_rates: HashMap<String, f32>,
}

impl ImportanceCalculator {
    pub fn new() -> Self {
        let mut decay_rates = HashMap::new();
        decay_rates.insert("topic".to_string(), 0.1);
        decay_rates.insert("location".to_string(), 0.07);
        decay_rates.insert("person".to_string(), 0.04);
        decay_rates.insert("concept".to_string(), 0.03);

        Self {
            max_access_count: 1000,
            max_relations_count: 20,
            decay_rates,
        }
    }

    pub fn calculate_importance(&self, memory: &MemoryEntity) -> f32 {
        let decay_rate = self.calculate_decay_rate(memory);
        let time_factor = self.calculate_time_factor(memory, decay_rate);
        let frequency_factor = self.calculate_frequency_factor(memory.access_count);
        let relations_factor = self.calculate_relations_factor(memory.relations.len());

        let combined_importance = 0.35 * memory.importance
            + 0.25 * time_factor
            + 0.25 * frequency_factor
            + 0.15 * relations_factor;

        combined_importance.clamp(0.0, 1.0)
    }

    fn calculate_time_factor(&self, memory: &MemoryEntity, decay_rate: f32) -> f32 {
        let elapsed = Utc::now().signed_duration_since(memory.updated_at);
        let hours_elapsed = elapsed.as_seconds_f32() / 3600.0;
        (-hours_elapsed * decay_rate).exp()
    }

    fn calculate_frequency_factor(&self, access_count: u32) -> f32 {
        if access_count == 0 {
            return 0.0;
        }

        let normalized_count = (access_count as f32).min(self.max_access_count as f32);
        (1.0 + normalized_count).ln() / (1.0 + self.max_access_count as f32).ln()
    }

    fn calculate_relations_factor(&self, relations_count: usize) -> f32 {
        (relations_count as f32 / self.max_relations_count as f32).min(1.0)
    }

    fn calculate_decay_rate(&self, memory: &MemoryEntity) -> f32 {
        let base_decay = self
            .decay_rates
            .get(&memory.entity_type)
            .copied()
            .unwrap_or(0.05);

        let mut decay_rate = if memory.access_count > 50 {
            base_decay * 0.5
        } else if memory.access_count < 5 {
            base_decay * 1.5
        } else {
            base_decay
        };

        if memory.importance > 0.9 {
            decay_rate *= 0.1;
        }

        if memory.importance < 0.2 {
            decay_rate *= 2.0;
        }
        decay_rate
    }
}
