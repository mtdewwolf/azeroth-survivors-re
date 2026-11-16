//! Persistent progression: shared passive ranks/points, named character save slots,
//! zone/difficulty unlocks, starter bonuses, and legacy save migration.

use std::{collections::BTreeMap, io::ErrorKind, path::PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    data::{self, DifficultyTier},
    state::{PlayerId, PlayerSpawnSpec, RewardBreakdown},
    Run,
};

pub(super) const SLOTS: usize = data::MAX_PASSIVES;
pub(super) const MAX_CHARACTERS: usize = 8;
pub(super) const MAX_NAME_LEN: usize = 16;
pub(super) const SAVE_VERSION: u32 = 1;

const HERO_NAMES: &[&str] = &[
    "Aeloria", "Baelor", "Cairne", "Dorian", "Elara", "Falric", "Grommash", "Haldor", "Iriel",
    "Jorik", "Kaelen", "Lyra", "Morgath", "Nissa", "Orgrim", "Pyra", "Quelen", "Rhonin", "Sylvan",
    "Turalyon", "Uther", "Vaelen", "Wrynn", "Xalor", "Yrel", "Zuljin",
];

pub(super) fn random_hero_name(seed: u64) -> &'static str {
    HERO_NAMES[(seed as usize) % HERO_NAMES.len()]
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct LegacyProfile {
    points: u32,
    ranks: BTreeMap<String, u8>,
    equipped: Vec<String>,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(default)]
pub(super) struct AccountPassives {
    pub points: u32,
    pub ranks: BTreeMap<String, u8>,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(default)]
pub(super) struct LifetimeRecords {
    pub runs: u32,
    pub wins: u32,
    pub kills: u32,
    pub bosses: u32,
    pub best_time_secs: u32,
    pub best_level: u32,
    pub highest_damage: u64,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(default)]
pub(super) struct CharacterSlot {
    pub id: u32,
    pub name: String,
    pub class_id: u8,
    pub race: u8,
    pub sex: u8,
    pub last_zone: String,
    pub tier: DifficultyTier,
    pub level: u32,
    pub xp: u32,
    pub equipped: Vec<String>,
    pub cleared: BTreeMap<String, u8>,
    pub records: LifetimeRecords,
}

impl Default for CharacterSlot {
    fn default() -> Self {
        Self {
            id: 1,
            name: "Adventurer".into(),
            class_id: data::CLASSES[0].id,
            race: 1,
            sex: 0,
            last_zone: data::MAPS[0].name.into(),
            tier: DifficultyTier::Normal,
            level: 1,
            xp: 0,
            equipped: Vec::new(),
            cleared: BTreeMap::new(),
            records: LifetimeRecords::default(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(default)]
pub(super) struct SaveFile {
    pub version: u32,
    pub active_slot_id: u32,
    pub next_slot_id: u32,
    pub account: AccountPassives,
    pub slots: Vec<CharacterSlot>,
}

impl Default for SaveFile {
    fn default() -> Self {
        Self {
            version: SAVE_VERSION,
            active_slot_id: 1,
            next_slot_id: 2,
            account: AccountPassives::default(),
            slots: vec![CharacterSlot::default()],
        }
    }
}

/// Starter bonus granted at `start_run` based on character level and meta mastery.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct StarterBonus {
    pub starting_level: u32,
    pub initial_drafts: u32,
    pub starter_ability_bonus: u8,
}

#[derive(Resource)]
pub(super) struct Progress {
    save: SaveFile,
    path: Option<PathBuf>,
    pub(super) load_error: bool,
}

impl Default for Progress {
    fn default() -> Self {
        let mut save = SaveFile::default();
        sanitize_save(&mut save);
        Self {
            save,
            path: None,
            load_error: false,
        }
    }
}

pub(super) fn passive_cost_for_rank(rank: u8) -> u32 {
    3 + 2 * u32::from(rank)
}

pub(super) fn char_xp_to_next(level: u32) -> u32 {
    100u32.saturating_add(level.saturating_sub(1).saturating_mul(75))
}

impl Progress {
    pub(super) fn points(&self) -> u32 {
        self.save.account.points
    }

    pub(super) fn rank(&self, index: usize) -> u8 {
        self.save
            .account
            .ranks
            .get(data::PASSIVES[index].key)
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn total_ranks(&self) -> u32 {
        self.save
            .account
            .ranks
            .values()
            .map(|&r| u32::from(r))
            .sum()
    }

    pub(super) fn spent_points(&self) -> u32 {
        self.save
            .account
            .ranks
            .values()
            .map(|&rank| (0..rank).map(passive_cost_for_rank).sum::<u32>())
            .sum()
    }

    pub(super) fn price(&self, index: usize) -> Option<u32> {
        let rank = self.rank(index);
        (rank < data::MAX_PASSIVE_LEVEL).then(|| passive_cost_for_rank(rank))
    }

    pub(super) fn slots(&self) -> &[CharacterSlot] {
        &self.save.slots
    }

    pub(super) fn slot_count(&self) -> usize {
        self.save.slots.len()
    }

    pub(super) fn active_slot_id(&self) -> u32 {
        self.save.active_slot_id
    }

    pub(super) fn active_slot(&self) -> &CharacterSlot {
        self.save
            .slots
            .iter()
            .find(|s| s.id == self.save.active_slot_id)
            .unwrap_or(&self.save.slots[0])
    }

    pub(super) fn equipped(&self, index: usize) -> bool {
        let key = data::PASSIVES[index].key;
        self.active_slot().equipped.iter().any(|item| item == key)
    }

    pub(super) fn equipped_count(&self) -> usize {
        self.active_slot().equipped.len()
    }

    pub(super) fn is_zone_unlocked(&self, map_index: usize) -> bool {
        let map = &data::MAPS[map_index.min(data::MAPS.len() - 1)];
        data::is_zone_unlocked(data::map_key(map), &self.active_slot().cleared)
    }

    pub(super) fn zone_best_tier(&self, map_index: usize) -> Option<DifficultyTier> {
        let map = &data::MAPS[map_index.min(data::MAPS.len() - 1)];
        self.active_slot()
            .cleared
            .get(data::map_key(map))
            .copied()
            .and_then(DifficultyTier::from_rank)
    }

    pub(super) fn is_tier_unlocked(&self, map_index: usize, tier: DifficultyTier) -> bool {
        let map = &data::MAPS[map_index.min(data::MAPS.len() - 1)];
        data::is_tier_unlocked(data::map_key(map), tier, &self.active_slot().cleared)
    }

    pub(super) fn starter_bonus(&self) -> StarterBonus {
        let slot = self.active_slot();
        let meta_ranks = self.total_ranks();
        let (base_lv, base_drafts, base_ab) = match slot.level {
            0..=2 => (1, 0, 0),
            3..=5 => (2, 1, 0),
            6..=9 => (3, 2, 1),
            _ => (4, 3, 1),
        };
        let meta_draft = u32::from(meta_ranks >= 10);
        let meta_ab = u8::from(meta_ranks >= 20);
        StarterBonus {
            starting_level: base_lv,
            initial_drafts: (base_drafts + meta_draft).min(3),
            starter_ability_bonus: base_ab + meta_ab,
        }
    }

    pub(super) fn buy(&mut self, index: usize) -> Result<(), &'static str> {
        let price = self
            .price(index)
            .ok_or("This passive is at maximum rank.")?;
        let rank = self.rank(index);
        let mut next = self.save.clone();
        next.account.points = next
            .account
            .points
            .checked_sub(price)
            .ok_or("Need more passive points.")?;
        let key = data::PASSIVES[index].key;
        next.account.ranks.insert(key.to_owned(), rank + 1);
        let active_id = next.active_slot_id;
        if rank == 0 {
            if let Some(slot) = next.slots.iter_mut().find(|s| s.id == active_id) {
                if slot.equipped.len() < SLOTS && !slot.equipped.iter().any(|k| k == key) {
                    slot.equipped.push(key.to_owned());
                }
            }
        }
        self.commit(next)
    }

    pub(super) fn toggle(&mut self, index: usize) -> Result<(), &'static str> {
        if self.rank(index) == 0 {
            return Err("Unlock this passive first.");
        }
        let key = data::PASSIVES[index].key;
        let mut next = self.save.clone();
        let active_id = next.active_slot_id;
        let Some(slot) = next.slots.iter_mut().find(|s| s.id == active_id) else {
            return Err("No active character slot.");
        };
        if let Some(pos) = slot.equipped.iter().position(|item| item == key) {
            slot.equipped.remove(pos);
        } else if slot.equipped.len() < SLOTS {
            slot.equipped.push(key.to_owned());
        } else {
            return Err("Unequip a passive to free a starting slot.");
        }
        self.commit(next)
    }

    pub(super) fn respec(&mut self) -> Result<u32, &'static str> {
        let refund = self.spent_points();
        if refund == 0 {
            return Err("No passive points spent to refund.");
        }
        let mut next = self.save.clone();
        next.account.points = next.account.points.saturating_add(refund);
        next.account.ranks.clear();
        for slot in &mut next.slots {
            slot.equipped.clear();
        }
        self.commit(next).map(|()| refund)
    }

    pub(super) fn starting_passives(&self) -> Vec<(&'static data::PassiveDef, u8)> {
        self.active_slot()
            .equipped
            .iter()
            .filter_map(|key| {
                let passive = data::PASSIVES.iter().find(|passive| passive.key == key)?;
                let rank = *self.save.account.ranks.get(key)?;
                (rank > 0).then_some((passive, rank))
            })
            .collect()
    }

    pub(super) fn build_player_spawn_spec(
        &self,
        class: &'static data::ClassDef,
        race: u8,
        sex: u8,
    ) -> PlayerSpawnSpec {
        let slot = self.active_slot();
        let bonus = self.starter_bonus();
        PlayerSpawnSpec {
            id: PlayerId(0),
            slot_id: Some(slot.id),
            name: slot.name.clone(),
            class,
            race,
            sex,
            starting_level: bonus.starting_level,
            initial_drafts: bonus.initial_drafts,
            starter_ability_bonus: bonus.starter_ability_bonus,
            meta_passives: self.starting_passives(),
        }
    }

    pub(super) fn select_slot(&mut self, id: u32) -> Result<(), &'static str> {
        if !self.save.slots.iter().any(|s| s.id == id) {
            return Err("Character slot not found.");
        }
        if self.save.active_slot_id == id {
            return Ok(());
        }
        let mut next = self.save.clone();
        next.active_slot_id = id;
        self.commit(next)
    }

    pub(super) fn create_slot(
        &mut self,
        raw_name: &str,
        class_id: u8,
        race: u8,
        sex: u8,
        map_index: usize,
    ) -> Result<u32, &'static str> {
        if self.save.slots.len() >= MAX_CHARACTERS {
            return Err("Maximum character slots reached (8).");
        }
        let name = clean_name(raw_name).ok_or("Enter a hero name first.")?;
        let mut next = self.save.clone();
        let id = next.next_slot_id.max(1);
        next.next_slot_id = id.saturating_add(1);
        let zone = data::MAPS[map_index.min(data::MAPS.len() - 1)].name;
        let cleared = BTreeMap::new();
        let last_zone = if data::is_zone_unlocked(zone, &cleared) {
            zone.to_owned()
        } else {
            data::MAPS[0].name.to_owned()
        };
        let slot = CharacterSlot {
            id,
            name,
            class_id: if data::class_by_id(class_id).is_some() {
                class_id
            } else {
                data::CLASSES[0].id
            },
            race,
            sex: sex.min(1),
            last_zone,
            tier: DifficultyTier::Normal,
            level: 1,
            xp: 0,
            equipped: Vec::new(),
            cleared,
            records: LifetimeRecords::default(),
        };
        next.slots.push(slot);
        next.active_slot_id = id;
        self.commit(next).map(|()| id)
    }

    pub(super) fn rename_slot(&mut self, id: u32, raw_name: &str) -> Result<(), &'static str> {
        let name = clean_name(raw_name).ok_or("Enter a hero name first.")?;
        let mut next = self.save.clone();
        let Some(slot) = next.slots.iter_mut().find(|s| s.id == id) else {
            return Err("Character slot not found.");
        };
        if slot.name == name {
            return Ok(());
        }
        slot.name = name;
        self.commit(next)
    }

    pub(super) fn delete_slot(&mut self, id: u32) -> Result<(), &'static str> {
        if self.save.slots.len() <= 1 {
            return Err("Cannot delete the last character.");
        }
        let Some(idx) = self.save.slots.iter().position(|s| s.id == id) else {
            return Err("Character slot not found.");
        };
        let mut next = self.save.clone();
        next.slots.remove(idx);
        if next.active_slot_id == id {
            next.active_slot_id = next.slots[0].id;
        }
        self.commit(next)
    }

    pub(super) fn sync_active_pick(
        &mut self,
        class_id: u8,
        race: u8,
        sex: u8,
        map_index: usize,
        tier: DifficultyTier,
    ) {
        if self.load_error {
            return;
        }
        let zone = data::MAPS[map_index.min(data::MAPS.len() - 1)]
            .name
            .to_owned();
        let cur = self.active_slot();
        if cur.class_id == class_id
            && cur.race == race
            && cur.sex == sex
            && cur.last_zone == zone
            && cur.tier == tier
        {
            return;
        }
        let mut next = self.save.clone();
        let active_id = next.active_slot_id;
        if let Some(slot) = next.slots.iter_mut().find(|s| s.id == active_id) {
            slot.class_id = class_id;
            slot.race = race;
            slot.sex = sex;
            slot.last_zone = zone;
            slot.tier = tier;
        }
        let _ = self.commit(next);
    }

    fn commit(&mut self, mut next: SaveFile) -> Result<(), &'static str> {
        if self.load_error {
            return Err("Could not read saved progress.");
        }
        sanitize_save(&mut next);
        if let Some(path) = &self.path {
            let text = toml::to_string(&next).map_err(|_| "Could not save progress.")?;
            if let Err(error) = crate::local_state::write_atomic(path, &text) {
                warn!("Could not save survivors progress: {error}");
                return Err("Could not save progress.");
            }
        }
        self.save = next;
        Ok(())
    }
}

fn clean_name(raw: &str) -> Option<String> {
    let trimmed: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_LEN)
        .collect();
    let trimmed = trimmed.trim().to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn sanitize_save(save: &mut SaveFile) {
    save.version = SAVE_VERSION;
    save.account.ranks.retain(|key, rank| {
        *rank = (*rank).min(data::MAX_PASSIVE_LEVEL);
        *rank > 0 && data::PASSIVES.iter().any(|passive| passive.key == key)
    });
    if save.slots.is_empty() {
        save.slots.push(CharacterSlot::default());
    }
    if save.slots.len() > MAX_CHARACTERS {
        save.slots.truncate(MAX_CHARACTERS);
    }

    let mut seen_ids = Vec::with_capacity(save.slots.len());
    let mut max_id = 0u32;
    for slot in &mut save.slots {
        if slot.id == 0 || seen_ids.contains(&slot.id) {
            slot.id = max_id + 1;
        }
        max_id = max_id.max(slot.id);
        seen_ids.push(slot.id);

        slot.name = clean_name(&slot.name).unwrap_or_else(|| format!("Hero {}", slot.id));
        if data::class_by_id(slot.class_id).is_none() {
            slot.class_id = data::CLASSES[0].id;
        }
        slot.sex = slot.sex.min(1);
        slot.level = slot.level.max(1);

        // Keep valid zone clear ranks only.
        slot.cleared.retain(|zone, tier| {
            *tier = (*tier).min(DifficultyTier::Mythic.rank());
            *tier > 0 && data::map_index_by_key(zone).is_some()
        });

        let zone_valid = data::map_index_by_key(&slot.last_zone).is_some()
            && data::is_zone_unlocked(&slot.last_zone, &slot.cleared);
        if !zone_valid {
            slot.last_zone = data::MAPS[0].name.to_owned();
        }
        if !data::is_tier_unlocked(&slot.last_zone, slot.tier, &slot.cleared) {
            slot.tier = DifficultyTier::Normal;
        }

        let mut equipped = Vec::new();
        for key in &slot.equipped {
            if save.account.ranks.contains_key(key) && !equipped.contains(key) {
                equipped.push(key.clone());
            }
            if equipped.len() == SLOTS {
                break;
            }
        }
        slot.equipped = equipped;
    }

    save.next_slot_id = save.next_slot_id.max(max_id + 1);
    if !save.slots.iter().any(|s| s.id == save.active_slot_id) {
        save.active_slot_id = save.slots[0].id;
    }
}

fn migrate_legacy(
    passives_path: Option<&PathBuf>,
    pick_path: Option<&PathBuf>,
) -> Result<Option<SaveFile>, &'static str> {
    let read_legacy = |path: Option<&PathBuf>| -> Result<Option<String>, &'static str> {
        match path.map(|p| crate::local_state::read_to_string(p)) {
            None => Ok(None),
            Some(Ok(text)) => Ok(Some(text)),
            Some(Err(err)) if err.kind() == ErrorKind::NotFound => Ok(None),
            Some(Err(_)) => Err("Could not read legacy survivors save."),
        }
    };
    let legacy_profile = read_legacy(passives_path)?
        .map(|text| toml::from_str::<LegacyProfile>(&text))
        .transpose()
        .map_err(|_| "Could not parse legacy survivors passives.")?;
    let legacy_pick = read_legacy(pick_path)?
        .map(|text| {
            let values = text
                .split_whitespace()
                .map(str::parse::<usize>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "Could not parse legacy survivors pick.")?;
            match values[..] {
                [class, race, sex, map] => Ok((class, race as u8, sex as u8, map)),
                _ => Err("Could not parse legacy survivors pick."),
            }
        })
        .transpose()?;

    if legacy_profile.is_none() && legacy_pick.is_none() {
        return Ok(None);
    }

    let mut save = SaveFile::default();
    if let Some(profile) = legacy_profile {
        save.account.points = profile.points;
        save.account.ranks = profile.ranks;
        save.slots[0].equipped = profile.equipped;
    }
    if let Some((class_idx, race, sex, map_idx)) = legacy_pick {
        if let Some(c) = data::CLASSES.get(class_idx) {
            save.slots[0].class_id = c.id;
            save.slots[0].race = race;
            save.slots[0].sex = sex.min(1);
        }
        if let Some(m) = data::MAPS.get(map_idx) {
            save.slots[0].last_zone = m.name.to_owned();
        }
    }
    sanitize_save(&mut save);
    Ok(Some(save))
}

pub(super) fn load(mut progress: ResMut<Progress>) {
    let saves_path = crate::local_state::survivors_saves_path();
    progress.path = saves_path.clone();
    let Some(path) = saves_path else { return };

    match crate::local_state::read_to_string(&path) {
        Ok(text) => match toml::from_str::<SaveFile>(&text) {
            Ok(save) if save.version != SAVE_VERSION => {
                warn!("Unsupported survivors save version: {}", save.version);
                progress.load_error = true;
            }
            Ok(mut save) => {
                sanitize_save(&mut save);
                progress.save = save;
            }
            Err(error) => {
                warn!("Could not read saved survivors progress: {error}");
                progress.load_error = true;
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let legacy_passives = crate::local_state::survivors_legacy_passives_path();
            let legacy_pick = crate::local_state::survivors_legacy_pick_path();
            match migrate_legacy(legacy_passives.as_ref(), legacy_pick.as_ref()) {
                Ok(Some(save)) => {
                    if let Err(err) = progress.commit(save) {
                        warn!("Could not persist migrated survivors save: {err}");
                        progress.load_error = true;
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    warn!("Could not migrate survivors save: {err}");
                    progress.load_error = true;
                }
            }
        }
        Err(error) => {
            warn!("Could not read saved survivors progress: {error}");
            progress.load_error = true;
        }
    }
}

fn reward_points(
    elapsed: f32,
    kills: u32,
    bosses: u32,
    won: bool,
    map_index: usize,
    tier: DifficultyTier,
) -> (u32, u32, u32, u32, u32, u32, u32) {
    let base_points = 1u32;
    let time_points = (elapsed / 60.0) as u32;
    let kill_points = kills / 25;
    let boss_points = bosses.saturating_mul(2);
    let victory_points = if won { 5 } else { 0 };
    let subtotal = base_points
        .saturating_add(time_points)
        .saturating_add(kill_points)
        .saturating_add(boss_points)
        .saturating_add(victory_points);
    let mult = data::zone_reward_mult(map_index) * tier.reward_mult();
    let total = ((subtotal as f32) * mult).round().max(subtotal as f32) as u32;
    (
        base_points,
        time_points,
        kill_points,
        boss_points,
        victory_points,
        subtotal,
        total,
    )
}

fn run_char_xp(
    elapsed: f32,
    kills: u32,
    bosses: u32,
    won: bool,
    map_index: usize,
    tier: DifficultyTier,
) -> u32 {
    let base = 20u32
        .saturating_add((elapsed / 10.0) as u32)
        .saturating_add(kills / 5)
        .saturating_add(bosses.saturating_mul(35))
        .saturating_add(if won { 120 } else { 0 });
    let mult = data::zone_reward_mult(map_index) * tier.reward_mult();
    ((base as f32) * mult).round().max(base as f32) as u32
}

pub(super) fn award(mut progress: ResMut<Progress>, mut run: ResMut<Run>) {
    if run.reward.is_some() {
        return;
    }

    let elapsed = run.match_state.elapsed;
    let bosses = run.match_state.bosses;
    let won = run.match_state.won;
    let map_index = run.match_state.config.map_index;
    let map_name = run.match_state.config.map.name;
    let tier = run.match_state.config.tier;

    let local = run.local();
    let kills = local.kills;
    let hero_level = local.level;
    let damage_done = local.damage_done.round().max(0.0) as u64;
    let target_slot_id = local.slot_id.unwrap_or(progress.active_slot_id());

    let (
        base_points,
        time_points,
        kill_points,
        boss_points,
        victory_points,
        subtotal,
        total_points,
    ) = reward_points(elapsed, kills, bosses, won, map_index, tier);
    let char_xp_gained = run_char_xp(elapsed, kills, bosses, won, map_index, tier);

    let mut next = progress.save.clone();
    next.account.points = next.account.points.saturating_add(total_points);

    let Some(slot_pos) = next.slots.iter().position(|s| s.id == target_slot_id) else {
        run.reward = Some(Err("Could not locate character slot."));
        return;
    };

    let slot = &mut next.slots[slot_pos];
    let slot_name = slot.name.clone();
    let char_level_before = slot.level;

    // Grant character XP and apply level-ups.
    slot.xp = slot.xp.saturating_add(char_xp_gained);
    while slot.xp >= char_xp_to_next(slot.level) {
        slot.xp -= char_xp_to_next(slot.level);
        slot.level = slot.level.saturating_add(1);
    }
    let char_level_after = slot.level;

    // Update lifetime records.
    slot.records.runs = slot.records.runs.saturating_add(1);
    if won {
        slot.records.wins = slot.records.wins.saturating_add(1);
    }
    slot.records.kills = slot.records.kills.saturating_add(kills);
    slot.records.bosses = slot.records.bosses.saturating_add(bosses);
    slot.records.best_time_secs = slot.records.best_time_secs.max(elapsed as u32);
    slot.records.best_level = slot.records.best_level.max(hero_level);
    slot.records.highest_damage = slot.records.highest_damage.max(damage_done);

    // Record zone clear and detect newly unlocked zones / difficulty tiers.
    let mut newly_unlocked_zones = Vec::new();
    let mut newly_unlocked_tier = None;
    if won {
        let before_cleared = slot.cleared.clone();
        let prev_tier = before_cleared.get(map_name).copied().unwrap_or(0);
        if tier.rank() > prev_tier {
            slot.cleared.insert(map_name.to_owned(), tier.rank());
            let next_tier = match tier {
                DifficultyTier::Normal => Some(DifficultyTier::Heroic),
                DifficultyTier::Heroic => Some(DifficultyTier::Mythic),
                DifficultyTier::Mythic => None,
            };
            if let Some(nt) = next_tier {
                if !data::is_tier_unlocked(map_name, nt, &before_cleared)
                    && data::is_tier_unlocked(map_name, nt, &slot.cleared)
                {
                    newly_unlocked_tier = Some(nt);
                }
            }
        }
        for m in data::MAPS {
            let k = data::map_key(m);
            if !data::is_zone_unlocked(k, &before_cleared)
                && data::is_zone_unlocked(k, &slot.cleared)
            {
                newly_unlocked_zones.push(m.name);
            }
        }
    }

    let breakdown = RewardBreakdown {
        base_points,
        time_points,
        kill_points,
        boss_points,
        victory_points,
        subtotal,
        tier,
        zone_mult_pct: (data::zone_reward_mult(map_index) * 100.0).round() as u32,
        tier_mult_pct: (tier.reward_mult() * 100.0).round() as u32,
        total_points,
        char_xp_gained,
        char_level_before,
        char_level_after,
        newly_unlocked_zones,
        newly_unlocked_tier,
        slot_name,
    };

    run.reward = Some(progress.commit(next).map(|()| breakdown));
}

#[cfg(test)]
mod tests {
    use super::super::state::{MatchConfig, RunConfig};
    use super::*;
    use crate::local_state::test_env::{EnvGuard, ENV_LOCK};

    #[test]
    fn buying_rank_one_then_rank_two_sets_the_starting_rank() {
        let mut progress = Progress::default();
        assert!(progress.buy(0).is_err());
        assert!(progress.toggle(0).is_err());
        progress.save.account.points = 8;
        progress.buy(0).unwrap();
        assert_eq!(progress.points(), 5);
        assert!(progress.equipped(0));
        progress.buy(0).unwrap();
        assert_eq!(progress.points(), 0);
        assert_eq!(progress.starting_passives()[0].1, 2);
    }

    #[test]
    fn slots_and_ranks_are_capped() {
        let mut progress = Progress::default();
        progress.save.account.points = 100;
        for index in 0..=SLOTS {
            progress.buy(index).unwrap();
        }
        assert_eq!(progress.equipped_count(), SLOTS);
        assert!(progress.toggle(SLOTS).is_err());
        progress.toggle(0).unwrap();
        progress.toggle(SLOTS).unwrap();
        assert!(progress.equipped(SLOTS));
        for _ in 1..data::MAX_PASSIVE_LEVEL {
            progress.buy(SLOTS).unwrap();
        }
        let balance = progress.points();
        assert!(progress.buy(SLOTS).is_err());
        assert_eq!(progress.points(), balance);
    }

    #[test]
    fn respec_refunds_exact_spent_points_and_clears_all_loadouts() {
        let mut progress = Progress::default();
        progress.save.account.points = 20;
        // Rank 1 costs 3, Rank 2 costs 5, another Rank 1 costs 3 => 11 spent, 9 left.
        progress.buy(0).unwrap();
        progress.buy(0).unwrap();
        progress.buy(1).unwrap();
        assert_eq!(progress.points(), 9);
        assert_eq!(progress.spent_points(), 11);

        // Create second slot and equip passive 1 on it too.
        let slot2 = progress.create_slot("Thrall", 7, 2, 0, 0).unwrap();
        progress.toggle(1).unwrap();
        assert!(progress.equipped(1));

        let refunded = progress.respec().unwrap();
        assert_eq!(refunded, 11);
        assert_eq!(progress.points(), 20);
        assert_eq!(progress.spent_points(), 0);
        assert_eq!(progress.rank(0), 0);
        assert_eq!(progress.rank(1), 0);
        assert_eq!(progress.equipped_count(), 0);
        progress.select_slot(1).unwrap();
        assert_eq!(progress.equipped_count(), 0);
        assert_ne!(slot2, 1);
    }

    #[test]
    fn saves_balance_ranks_and_loadout_together() {
        let mut progress = Progress::default();
        progress.save.account.points = 10;
        progress.buy(1).unwrap();
        let saved: SaveFile = toml::from_str(&toml::to_string(&progress.save).unwrap()).unwrap();
        assert_eq!(saved.account.points, 7);
        assert_eq!(saved.account.ranks["fortitude"], 1);
        assert_eq!(saved.slots[0].equipped, ["fortitude"]);
        progress.load_error = true;
        assert!(progress.buy(1).is_err());
        assert_eq!(progress.points(), 7);
        assert_eq!(progress.rank(1), 1);
    }

    #[test]
    fn character_slot_crud_keeps_independent_loadouts_and_records() {
        let mut progress = Progress::default();
        progress.save.account.points = 20;
        progress.buy(0).unwrap();
        progress.buy(1).unwrap();
        assert_eq!(progress.active_slot().equipped, ["might", "fortitude"]);

        let s2 = progress.create_slot("Jaina", 8, 1, 1, 0).unwrap();
        assert_eq!(progress.active_slot().name, "Jaina");
        assert!(progress.active_slot().equipped.is_empty());
        progress.toggle(1).unwrap();
        assert_eq!(progress.active_slot().equipped, ["fortitude"]);

        progress.rename_slot(s2, "Archmage").unwrap();
        assert_eq!(progress.active_slot().name, "Archmage");

        progress.select_slot(1).unwrap();
        assert_eq!(progress.active_slot().equipped, ["might", "fortitude"]);

        progress.delete_slot(s2).unwrap();
        assert_eq!(progress.slot_count(), 1);
        assert!(progress.delete_slot(1).is_err());
    }

    #[test]
    fn starter_bonuses_scale_with_character_level_and_meta_ranks() {
        let mut progress = Progress::default();
        assert_eq!(
            progress.starter_bonus(),
            StarterBonus {
                starting_level: 1,
                initial_drafts: 0,
                starter_ability_bonus: 0,
            }
        );
        progress.save.slots[0].level = 4;
        assert_eq!(
            progress.starter_bonus(),
            StarterBonus {
                starting_level: 2,
                initial_drafts: 1,
                starter_ability_bonus: 0,
            }
        );
        progress.save.slots[0].level = 7;
        assert_eq!(
            progress.starter_bonus(),
            StarterBonus {
                starting_level: 3,
                initial_drafts: 2,
                starter_ability_bonus: 1,
            }
        );
    }

    #[test]
    fn a_completed_run_pays_once_and_unlocks_next_zone_and_heroic() {
        let mut app = App::new();
        let mut run = Run::from_config(
            RunConfig {
                match_config: MatchConfig {
                    map_index: 0,
                    map: &data::MAPS[0],
                    tier: DifficultyTier::Normal,
                },
                local: PlayerId(0),
                players: vec![PlayerSpawnSpec {
                    id: PlayerId(0),
                    slot_id: Some(1),
                    name: "Adventurer".into(),
                    ..PlayerSpawnSpec::default()
                }],
            },
            1,
        );
        run.match_state.elapsed = 120.0;
        run.match_state.bosses = 2;
        run.match_state.won = true;
        run.local_mut().kills = 51;

        app.init_resource::<Progress>()
            .insert_resource(run)
            .add_systems(Update, award);
        app.update();
        app.update();

        let progress = app.world().resource::<Progress>();
        assert_eq!(progress.points(), 14);
        assert!(progress.is_tier_unlocked(0, DifficultyTier::Heroic));
        assert!(progress.active_slot().level >= 2);

        let reward = app
            .world()
            .resource::<Run>()
            .reward
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(reward.total_points, 14);
        assert_eq!(reward.newly_unlocked_tier, Some(DifficultyTier::Heroic));
        assert!(reward.newly_unlocked_zones.contains(&"Westfall"));

        let (_, _, _, _, _, _, r0) = reward_points(59.9, 24, 0, false, 0, DifficultyTier::Normal);
        assert_eq!(r0, 1);
    }

    #[test]
    fn unsupported_save_versions_are_preserved_and_block_writes() {
        let _lock = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = std::env::temp_dir().join(format!("benilla-surv-version-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let _capture = EnvGuard::unset("WOW_CAPTURE");
        let _home = EnvGuard::set("BENILLA_HOME", tmp.to_str().unwrap());
        let path = tmp.join("survivors-saves.toml");
        let text = format!("version = {}\n", SAVE_VERSION + 1);
        std::fs::write(&path, &text).unwrap();
        let mut app = App::new();
        app.init_resource::<Progress>().add_systems(Update, load);
        app.update();
        let mut progress = app.world_mut().resource_mut::<Progress>();
        assert!(progress.load_error);
        assert!(progress.rename_slot(1, "Overwrite").is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn corrupt_legacy_progress_does_not_create_a_partial_migration() {
        let tmp = std::env::temp_dir().join(format!("benilla-surv-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let passives = tmp.join("survivors-passives.toml");
        let pick = tmp.join("survivors-pick.txt");
        std::fs::write(&passives, "points = [broken").unwrap();
        std::fs::write(&pick, "6 1 1 0").unwrap();
        assert!(migrate_legacy(Some(&passives), Some(&pick)).is_err());
        std::fs::write(&passives, "points = 12").unwrap();
        std::fs::write(&pick, "6 garbage 1 1 0").unwrap();
        assert!(migrate_legacy(Some(&passives), Some(&pick)).is_err());
        std::fs::remove_dir_all(tmp).unwrap();
    }

    #[test]
    fn migrates_legacy_pick_and_passives_into_unified_saves_toml() {
        let _l = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = std::env::temp_dir().join(format!("benilla-surv-mig-{}", std::process::id()));
        std::fs::remove_dir_all(&tmp).ok();
        std::fs::create_dir_all(&tmp).unwrap();
        let _c = EnvGuard::unset("WOW_CAPTURE");
        let _h = EnvGuard::set("BENILLA_HOME", tmp.to_str().unwrap());

        std::fs::write(
            tmp.join("survivors-passives.toml"),
            "points = 12\nequipped = [\"might\"]\n\n[ranks]\nmight = 2\nfortitude = 1\n",
        )
        .unwrap();
        // Class index 6 = Mage, race 1 = Human, sex 1 = Female, map 0 = The Barrens.
        std::fs::write(tmp.join("survivors-pick.txt"), "6 1 1 0").unwrap();

        let mut app = App::new();
        app.init_resource::<Progress>().add_systems(Update, load);
        app.update();

        let progress = app.world().resource::<Progress>();
        assert!(!progress.load_error);
        assert_eq!(progress.points(), 12);
        assert_eq!(progress.rank(0), 2);
        assert_eq!(progress.rank(1), 1);
        assert_eq!(progress.active_slot().class_id, data::CLASSES[6].id);
        assert_eq!(progress.active_slot().race, 1);
        assert_eq!(progress.active_slot().sex, 1);
        assert_eq!(progress.active_slot().last_zone, "The Barrens");
        assert!(tmp.join("survivors-saves.toml").is_file());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
