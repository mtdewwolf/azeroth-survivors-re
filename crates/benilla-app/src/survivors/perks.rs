//! Between-run passive ranks and loadout saved as a single transaction.
use std::{collections::BTreeMap, io::ErrorKind, path::PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{data, Run};

pub(super) const SLOTS: usize = data::MAX_PASSIVES;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Profile {
    points: u32,
    ranks: BTreeMap<String, u8>,
    equipped: Vec<String>,
}

#[derive(Resource, Default)]
pub(super) struct Progress {
    profile: Profile,
    path: Option<PathBuf>,
    pub(super) load_error: bool,
}

impl Progress {
    pub(super) fn points(&self) -> u32 {
        self.profile.points
    }

    pub(super) fn rank(&self, index: usize) -> u8 {
        self.profile
            .ranks
            .get(data::PASSIVES[index].key)
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn price(&self, index: usize) -> Option<u32> {
        let rank = self.rank(index);
        (rank < data::MAX_PASSIVE_LEVEL).then(|| 3 + 2 * u32::from(rank))
    }

    pub(super) fn equipped(&self, index: usize) -> bool {
        let key = data::PASSIVES[index].key;
        self.profile.equipped.iter().any(|item| item == key)
    }

    pub(super) fn equipped_count(&self) -> usize {
        self.profile.equipped.len()
    }

    pub(super) fn buy(&mut self, index: usize) -> Result<(), &'static str> {
        let price = self
            .price(index)
            .ok_or("This passive is at maximum rank.")?;
        let rank = self.rank(index);
        let mut next = self.profile.clone();
        next.points = next
            .points
            .checked_sub(price)
            .ok_or("Need more passive points.")?;
        let key = data::PASSIVES[index].key;
        next.ranks.insert(key.to_owned(), rank + 1);
        if rank == 0 && next.equipped.len() < SLOTS {
            next.equipped.push(key.to_owned());
        }
        self.commit(next)
    }

    pub(super) fn toggle(&mut self, index: usize) -> Result<(), &'static str> {
        if self.rank(index) == 0 {
            return Err("Unlock this passive first.");
        }
        let key = data::PASSIVES[index].key;
        let mut next = self.profile.clone();
        if let Some(slot) = next.equipped.iter().position(|item| item == key) {
            next.equipped.remove(slot);
        } else if next.equipped.len() < SLOTS {
            next.equipped.push(key.to_owned());
        } else {
            return Err("Unequip a passive to free a starting slot.");
        }
        self.commit(next)
    }

    pub(super) fn starting_passives(&self) -> Vec<(&'static data::PassiveDef, u8)> {
        self.profile
            .equipped
            .iter()
            .filter_map(|key| {
                let passive = data::PASSIVES.iter().find(|passive| passive.key == key)?;
                let rank = *self.profile.ranks.get(key)?;
                Some((passive, rank))
            })
            .collect()
    }

    fn commit(&mut self, next: Profile) -> Result<(), &'static str> {
        if self.load_error {
            return Err("Could not read saved passive progress.");
        }
        if let Some(path) = &self.path {
            let text = toml::to_string(&next).map_err(|_| "Could not save passive progress.")?;
            if let Err(error) = crate::local_state::write_atomic(path, &text) {
                warn!("Could not save passive progress: {error}");
                return Err("Could not save passive progress.");
            }
        }
        self.profile = next;
        Ok(())
    }
}

pub(super) fn load(mut progress: ResMut<Progress>) {
    progress.path = crate::local_state::home().map(|home| home.join("survivors-passives.toml"));
    let Some(path) = &progress.path else { return };
    let result = match crate::local_state::read_to_string(path) {
        Ok(text) => toml::from_str::<Profile>(&text).map_err(|error| error.to_string()),
        Err(error) if error.kind() == ErrorKind::NotFound => return,
        Err(error) => Err(error.to_string()),
    };
    match result {
        Ok(mut profile) => {
            profile.ranks.retain(|key, rank| {
                *rank = (*rank).min(data::MAX_PASSIVE_LEVEL);
                *rank > 0 && data::PASSIVES.iter().any(|passive| passive.key == key)
            });
            let mut equipped = Vec::new();
            for key in profile.equipped {
                if profile.ranks.contains_key(&key) && !equipped.contains(&key) {
                    equipped.push(key);
                }
                if equipped.len() == SLOTS {
                    break;
                }
            }
            profile.equipped = equipped;
            progress.profile = profile;
        }
        Err(error) => {
            warn!("Could not read saved passive progress: {error}");
            progress.load_error = true;
        }
    }
}

fn reward(elapsed: f32, kills: u32, bosses: u32, won: bool) -> u32 {
    1u32.saturating_add((elapsed / 60.0) as u32)
        .saturating_add(kills / 25)
        .saturating_add(bosses.saturating_mul(2))
        .saturating_add(if won { 5 } else { 0 })
}

pub(super) fn award(mut progress: ResMut<Progress>, mut run: ResMut<Run>) {
    if run.passive_reward.is_some() {
        return;
    }
    let mut next = progress.profile.clone();
    next.points = next
        .points
        .saturating_add(reward(run.elapsed, run.kills, run.bosses, run.won));
    let awarded = next.points - progress.points();
    run.passive_reward = Some(progress.commit(next).map(|()| awarded));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buying_rank_one_then_rank_two_sets_the_starting_rank() {
        let mut progress = Progress::default();
        assert!(progress.buy(0).is_err());
        assert!(progress.toggle(0).is_err());
        progress.profile.points = 8;
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
        progress.profile.points = 100;
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
    fn saves_balance_ranks_and_loadout_together() {
        let mut progress = Progress::default();
        progress.profile.points = 10;
        progress.buy(1).unwrap();
        let saved: Profile = toml::from_str(&toml::to_string(&progress.profile).unwrap()).unwrap();
        assert_eq!(saved.points, 7);
        assert_eq!(saved.ranks["fortitude"], 1);
        assert_eq!(saved.equipped, ["fortitude"]);
        progress.load_error = true;
        assert!(progress.buy(1).is_err());
        assert_eq!(progress.points(), 7);
        assert_eq!(progress.rank(1), 1);
    }

    #[test]
    fn a_completed_run_pays_once() {
        let mut app = App::new();
        app.init_resource::<Progress>()
            .insert_resource(Run {
                elapsed: 120.0,
                kills: 51,
                bosses: 2,
                won: true,
                ..Run::default()
            })
            .add_systems(Update, award);
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Progress>().points(), 14);
        assert_eq!(app.world().resource::<Run>().passive_reward, Some(Ok(14)));
        assert_eq!(reward(59.9, 24, 0, false), 1);
    }
}
