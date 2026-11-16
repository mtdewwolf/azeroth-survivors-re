//! Match/world state vs. per-player state and input boundaries.
//!
//! Gameplay systems operate on [`MatchState`], [`PlayerState`], [`PlayerInputs`], and
//! [`PlayerHero`] rather than reading UI selection or persistence resources mid-frame.

use bevy::prelude::*;

use super::data::{self, AbilityDef, ClassDef, DifficultyTier, MapDef, PassiveDef, Stat};
use super::mobility::MobilityState;

pub(super) const BASE_HP: f32 = 120.0;
pub(super) const BASE_SPEED: f32 = 7.0;
pub(super) const BASE_MAGNET: f32 = 4.5;

/// Identifies one participant in a match (local player is `PlayerId(0)`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub(super) struct PlayerId(pub u8);

/// Attached to each spawned hero entity to bind it to its [`PlayerState`].
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct PlayerHero(pub PlayerId);

/// An ability the hero has, with its own cooldown.
pub(super) struct Owned {
    pub def: &'static AbilityDef,
    pub level: u8,
    pub cd: f32,
}

impl Owned {
    pub fn damage_mult(&self) -> f32 {
        1.0 + 0.3 * f32::from(self.level - 1)
    }

    pub fn cooldown(&self, stats: &Stats) -> f32 {
        let base = self.def.cooldown * 0.93f32.powi(i32::from(self.level) - 1);
        (base * (1.0 - stats.haste).max(0.35)).max(0.2)
    }

    pub fn count(&self) -> u32 {
        let extra = [3u8, 5, 7].iter().filter(|&&l| self.level >= l).count() as u32;
        self.def.count + extra
    }

    pub fn radius(&self, stats: &Stats) -> f32 {
        self.def.radius * (1.0 + 0.08 * f32::from(self.level - 1)) * stats.area
    }

    pub fn duration(&self) -> f32 {
        self.def.duration * (1.0 + 0.1 * f32::from(self.level - 1))
    }
}

/// The hero's derived numbers, from class base, in-run passives, and equipped meta-passives.
#[derive(Clone, Copy, Debug)]
pub(super) struct Stats {
    pub might: f32,
    pub max_hp: f32,
    pub speed: f32,
    pub regen: f32,
    pub haste: f32,
    pub area: f32,
    pub magnet: f32,
    pub growth: f32,
    pub armor: f32,
    pub crit: f32,
}

/// Immutable match parameters captured when a run begins.
#[derive(Clone, Copy)]
pub(super) struct MatchConfig {
    pub map_index: usize,
    pub map: &'static MapDef,
    pub tier: DifficultyTier,
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            map_index: 0,
            map: &data::MAPS[0],
            tier: DifficultyTier::Normal,
        }
    }
}

/// Everything needed to initialize one player's [`PlayerState`] at run start.
#[derive(Clone)]
pub(super) struct PlayerSpawnSpec {
    pub id: PlayerId,
    pub slot_id: Option<u32>,
    pub name: String,
    pub class: &'static ClassDef,
    pub race: u8,
    pub sex: u8,
    pub starting_level: u32,
    pub initial_drafts: u32,
    pub starter_ability_bonus: u8,
    pub meta_passives: Vec<(&'static PassiveDef, u8)>,
}

impl Default for PlayerSpawnSpec {
    fn default() -> Self {
        Self {
            id: PlayerId(0),
            slot_id: None,
            name: "Adventurer".into(),
            class: &data::CLASSES[0],
            race: 1,
            sex: 0,
            starting_level: 1,
            initial_drafts: 0,
            starter_ability_bonus: 0,
            meta_passives: Vec::new(),
        }
    }
}

/// Full specification handed from menu/lobby setup to initialize [`Run`].
#[derive(Clone, Default)]
pub(super) struct RunConfig {
    pub match_config: MatchConfig,
    pub local: PlayerId,
    pub players: Vec<PlayerSpawnSpec>,
}

/// Shared world/match state for a run.
pub(super) struct MatchState {
    pub config: MatchConfig,
    pub elapsed: f32,
    pub bosses: u32,
    pub rings: u32,
    pub won: bool,
    pub next_guid: u64,
    /// A centre-screen announcement and its seconds left.
    pub banner: Option<(String, f32)>,
}

impl Default for MatchState {
    fn default() -> Self {
        Self {
            config: MatchConfig::default(),
            elapsed: 0.0,
            bosses: 0,
            rings: 0,
            won: false,
            next_guid: 1,
            banner: None,
        }
    }
}

impl MatchState {
    pub fn guid(&mut self) -> u64 {
        self.next_guid += 1;
        self.next_guid
    }

    pub fn enemy_hp_mult(&self) -> f32 {
        data::hp_scale(self.elapsed) * self.config.tier.hp_mult()
    }

    pub fn enemy_damage_mult(&self) -> f32 {
        data::damage_scale(self.elapsed) * self.config.tier.damage_mult()
    }

    pub fn enemy_speed_mult(&self) -> f32 {
        self.config.tier.speed_mult()
    }

    pub fn enemy_xp_mult(&self) -> f32 {
        self.config.tier.xp_mult()
    }
}

/// Per-player combat and build state inside a run.
pub(super) struct PlayerState {
    pub id: PlayerId,
    pub slot_id: Option<u32>,
    pub name: String,
    pub class: &'static ClassDef,
    pub race: u8,
    pub sex: u8,
    pub level: u32,
    pub xp: f32,
    pub hp: f32,
    pub kills: u32,
    pub damage_done: f64,
    pub abilities: Vec<Owned>,
    /// In-run passives drafted during this run (capped by `data::MAX_PASSIVES`).
    pub passives: Vec<(&'static PassiveDef, u8)>,
    /// Meta-passives equipped before the run began; do NOT consume in-run `MAX_PASSIVES` slots.
    pub meta_passives: Vec<(&'static PassiveDef, u8)>,
    pub shield: f32,
    pub shield_t: f32,
    pub immune_t: f32,
    pub thorns: f32,
    pub thorns_t: f32,
    pub thorns_spell: u32,
    pub heal_rate: f32,
    pub heal_t: f32,
    pub mobile: MobilityState,
    pub pending_levels: u32,
    /// Seconds since this hero fell, while the death plays out.
    pub dead_t: Option<f32>,
    /// Bumped whenever abilities, in-run passives, or meta-passives change.
    pub version: u32,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self::from_spawn(PlayerSpawnSpec::default())
    }
}

impl PlayerState {
    pub fn from_spawn(spec: PlayerSpawnSpec) -> Self {
        let start_level = (1 + spec.starter_ability_bonus).clamp(1, data::MAX_ABILITY_LEVEL);
        let abilities = vec![Owned {
            def: data::ability(spec.class.start),
            level: start_level,
            cd: 0.5,
        }];
        let mut p = Self {
            id: spec.id,
            slot_id: spec.slot_id,
            name: spec.name,
            class: spec.class,
            race: spec.race,
            sex: spec.sex,
            level: spec.starting_level.max(1),
            xp: 0.0,
            hp: BASE_HP,
            kills: 0,
            damage_done: 0.0,
            abilities,
            passives: Vec::new(),
            meta_passives: spec.meta_passives,
            shield: 0.0,
            shield_t: 0.0,
            immune_t: 0.0,
            thorns: 0.0,
            thorns_t: 0.0,
            thorns_spell: 0,
            heal_rate: 0.0,
            heal_t: 0.0,
            mobile: MobilityState::default(),
            pending_levels: spec.initial_drafts,
            dead_t: None,
            version: 1,
        };
        p.hp = p.stats().max_hp;
        p
    }

    pub fn stats(&self) -> Stats {
        let mut s = Stats {
            might: 1.0,
            max_hp: BASE_HP,
            speed: BASE_SPEED,
            regen: 0.0,
            haste: 0.0,
            area: 1.0,
            magnet: BASE_MAGNET,
            growth: 1.0,
            armor: 0.0,
            crit: 0.05,
        };
        // Tanky classes start tougher, cloth starts quicker.
        match self.class.id {
            1 | 2 => s.max_hp += 40.0,
            7 | 11 | 3 | 4 => s.max_hp += 15.0,
            _ => s.speed += 0.5,
        }
        for &(p, level) in self.passives.iter().chain(self.meta_passives.iter()) {
            let v = p.per_level * f32::from(level);
            match p.stat {
                Stat::Might => s.might += v,
                Stat::MaxHp => s.max_hp *= 1.0 + v,
                Stat::Speed => s.speed *= 1.0 + v,
                Stat::Regen => s.regen += v,
                Stat::Haste => s.haste += v,
                Stat::Area => s.area += v,
                Stat::Magnet => s.magnet *= 1.0 + v,
                Stat::Growth => s.growth += v,
                Stat::Armor => s.armor += v,
                Stat::Crit => s.crit += v,
            }
        }
        s
    }

    pub fn has_passive_key(&self, key: &str) -> bool {
        self.passives
            .iter()
            .chain(self.meta_passives.iter())
            .any(|(p, _)| p.key == key)
    }
}

/// Per-frame normalized input for one player, decoupled from local devices so remote inputs
/// can plug into [`PlayerInputs`] later.
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct PlayerInput {
    /// World XZ movement intent (`x` = east/west, `y` = world Z); magnitude in `[0, 1]`.
    pub move_dir: Vec2,
    /// Optional world-space look target direction from hero.
    pub look_dir: Option<Vec3>,
    pub jump: bool,
    pub mobility: bool,
    pub mobility_aim: Option<Vec3>,
}

/// Input buffer keyed by [`PlayerId`], written in `WorldStage::Input`.
#[derive(Resource, Default)]
pub(super) struct PlayerInputs {
    slots: Vec<(PlayerId, PlayerInput)>,
}

impl PlayerInputs {
    pub fn clear_all(&mut self) {
        self.slots.clear();
    }

    pub fn set(&mut self, id: PlayerId, input: PlayerInput) {
        if let Some((_, slot)) = self.slots.iter_mut().find(|(pid, _)| *pid == id) {
            *slot = input;
        } else {
            self.slots.push((id, input));
        }
    }

    pub fn get(&self, id: PlayerId) -> PlayerInput {
        self.slots
            .iter()
            .find(|(pid, _)| *pid == id)
            .map_or_default(|(_, input)| *input)
    }
}

/// Post-run passive-point & character-progression breakdown shown on `OverReward`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RewardBreakdown {
    pub base_points: u32,
    pub time_points: u32,
    pub kill_points: u32,
    pub boss_points: u32,
    pub victory_points: u32,
    pub subtotal: u32,
    pub tier: DifficultyTier,
    pub zone_mult_pct: u32,
    pub tier_mult_pct: u32,
    pub total_points: u32,
    pub char_xp_gained: u32,
    pub char_level_before: u32,
    pub char_level_after: u32,
    pub newly_unlocked_zones: Vec<&'static str>,
    pub newly_unlocked_tier: Option<DifficultyTier>,
    pub slot_name: String,
}

pub(super) type RewardOutcome = Result<RewardBreakdown, &'static str>;

/// Top-level active run state: shared match state + per-player roster + reward summary.
#[derive(Resource)]
pub(super) struct Run {
    pub match_state: MatchState,
    pub players: Vec<PlayerState>,
    pub local: PlayerId,
    pub reward: Option<RewardOutcome>,
}

impl Default for Run {
    fn default() -> Self {
        Self {
            match_state: MatchState::default(),
            players: vec![PlayerState::default()],
            local: PlayerId(0),
            reward: None,
        }
    }
}

impl Run {
    pub fn from_config(config: RunConfig, next_guid: u64) -> Self {
        let mut players: Vec<PlayerState> = config
            .players
            .into_iter()
            .map(PlayerState::from_spawn)
            .collect();
        if players.is_empty() {
            players.push(PlayerState::default());
        }
        Self {
            match_state: MatchState {
                config: config.match_config,
                next_guid,
                ..MatchState::default()
            },
            players,
            local: config.local,
            reward: None,
        }
    }

    pub fn guid(&mut self) -> u64 {
        self.match_state.guid()
    }

    pub fn player(&self, id: PlayerId) -> Option<&PlayerState> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn player_mut(&mut self, id: PlayerId) -> Option<&mut PlayerState> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    pub fn local(&self) -> &PlayerState {
        self.player(self.local).unwrap_or_else(|| &self.players[0])
    }

    pub fn local_mut(&mut self) -> &mut PlayerState {
        let id = self.local;
        if let Some(pos) = self.players.iter().position(|p| p.id == id) {
            &mut self.players[pos]
        } else {
            &mut self.players[0]
        }
    }

    pub fn any_alive(&self) -> bool {
        self.players.iter().any(|p| p.dead_t.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_passives_apply_stats_without_consuming_in_run_slots() {
        let spec = PlayerSpawnSpec {
            meta_passives: vec![
                (&data::PASSIVES[0], 2),
                (&data::PASSIVES[1], 1),
                (&data::PASSIVES[2], 1),
                (&data::PASSIVES[3], 1),
                (&data::PASSIVES[4], 1),
                (&data::PASSIVES[5], 1),
            ],
            ..PlayerSpawnSpec::default()
        };
        let mut p = PlayerState::from_spawn(spec);
        assert_eq!(p.meta_passives.len(), 6);
        assert_eq!(p.passives.len(), 0);

        let stats_meta_only = p.stats();
        assert!((stats_meta_only.might - 1.20).abs() < 1e-4);

        // In-run passives have all 6 slots free and stack additively on top of meta-passives.
        p.passives.push((&data::PASSIVES[0], 3));
        assert_eq!(p.passives.len(), 1);
        let stats_combined = p.stats();
        assert!((stats_combined.might - 1.50).abs() < 1e-4);
    }

    #[test]
    fn run_from_config_preserves_guid_and_tracks_multiple_players() {
        let cfg = RunConfig {
            match_config: MatchConfig {
                map_index: 0,
                map: &data::MAPS[0],
                tier: DifficultyTier::Heroic,
            },
            local: PlayerId(0),
            players: vec![
                PlayerSpawnSpec {
                    id: PlayerId(0),
                    name: "Thrall".into(),
                    starting_level: 3,
                    initial_drafts: 2,
                    starter_ability_bonus: 1,
                    ..PlayerSpawnSpec::default()
                },
                PlayerSpawnSpec {
                    id: PlayerId(1),
                    name: "Jaina".into(),
                    class: &data::CLASSES[6],
                    ..PlayerSpawnSpec::default()
                },
            ],
        };
        let mut run = Run::from_config(cfg, 42);
        assert_eq!(run.guid(), 43);
        assert_eq!(run.players.len(), 2);
        assert_eq!(run.local().name, "Thrall");
        assert_eq!(run.local().level, 3);
        assert_eq!(run.local().pending_levels, 2);
        assert_eq!(run.local().abilities[0].level, 2);
        assert_eq!(run.player(PlayerId(1)).unwrap().name, "Jaina");
        assert!(run.any_alive());
    }
}
