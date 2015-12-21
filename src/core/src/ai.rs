// See LICENSE file for copyright and license details.

use common::types::{Size2};
use game_state::{GameState, GameStateMut};
use partial_state::{PartialState};
use map::{distance};
use pathfinder::{Pathfinder, path_cost, truncate_path};
use dir::{Dir};
use unit::{Unit};
use db::{Db};
use ::{CoreEvent, Command, MoveMode, PlayerId, MapPos, check_command};

pub struct Ai {
    id: PlayerId,
    state: PartialState,
    pathfinder: Pathfinder,
}

impl Ai {
    pub fn new(id: &PlayerId, map_size: &Size2) -> Ai {
        Ai {
            id: id.clone(),
            state: PartialState::new(map_size, id),
            pathfinder: Pathfinder::new(map_size),
        }
    }

    pub fn apply_event(&mut self, db: &Db, event: &CoreEvent) {
        self.state.apply_event(db, event);
    }

    // TODO: move fill_map here
    fn get_best_pos(&self, db: &Db, unit: &Unit) -> Option<MapPos> {
        let mut best_pos = None;
        let mut best_cost = None;
        for (_, enemy) in self.state.units() {
            if enemy.player_id == self.id {
                continue;
            }
            for i in 0 .. 6 {
                let dir = Dir::from_int(i);
                let destination = Dir::get_neighbour_pos(&enemy.pos, &dir);
                if !self.state.map().is_inboard(&destination) {
                    continue;
                }
                if self.state.is_tile_occupied(&destination) {
                    continue;
                }
                let path = match self.pathfinder.get_path(&destination) {
                    Some(path) => path,
                    None => continue,
                };
                // TODO: ZInt -> MovePoints
                let cost = path_cost(db, &self.state, unit, &path).n;
                if let Some(ref mut best_cost) = best_cost {
                    if *best_cost > cost {
                        *best_cost = cost;
                        best_pos = Some(destination.clone());
                    }
                } else {
                    best_cost = Some(cost);
                    best_pos = Some(destination.clone());
                }
            }
        }
        best_pos
    }

    fn is_close_to_enemies(&self, db: &Db, unit: &Unit) -> bool {
        for (_, target) in self.state.units() {
            if target.player_id == self.id {
                continue;
            }
            let attacker_type = db.unit_type(&unit.type_id);
            let weapon_type = db.weapon_type(&attacker_type.weapon_type_id);
            if distance(&unit.pos, &target.pos) <= weapon_type.max_distance {
                return true;
            }
        }
        false
    }

    pub fn try_get_attack_command(&self, db: &Db) -> Option<Command> {
        for (_, unit) in self.state.units() {
            if unit.player_id != self.id {
                continue;
            }
            // println!("id: {}, ap: {}", unit.id.id, unit.attack_points);
            if unit.attack_points <= 0 {
                continue;
            }
            for (_, target) in self.state.units() {
                if target.player_id == self.id {
                    continue;
                }
                let command = Command::AttackUnit {
                    attacker_id: unit.id.clone(),
                    defender_id: target.id.clone(),
                };
                if let Ok(()) = check_command(db, &self.state, &command) {
                    return Some(command);
                }
            }
        }
        None
    }

    pub fn try_get_move_command(&mut self, db: &Db) -> Option<Command> {
        for (_, unit) in self.state.units() {
            if unit.player_id != self.id {
                continue;
            }
            if self.is_close_to_enemies(db, unit) {
                continue;
            }
            self.pathfinder.fill_map(db, &self.state, unit);
            // TODO: if no enemy is visible then move to random invisible tile
            let destination = match self.get_best_pos(db, unit) {
                Some(destination) => destination,
                None => continue,
            };
            let path = match self.pathfinder.get_path(&destination) {
                Some(path) => path,
                None => continue,
            };
            if unit.move_points.n == 0 {
                continue;
            }
            let path = truncate_path(db, &self.state, &path, unit);
            return Some(Command::Move {
                unit_id: unit.id.clone(),
                path: path,
                mode: MoveMode::Fast,
            });
        }
        None
    }

    pub fn get_command(&mut self, db: &Db) -> Command {
        if let Some(cmd) = self.try_get_attack_command(db) {
            cmd
        } else if let Some(cmd) = self.try_get_move_command(db) {
            cmd
        } else {
            Command::EndTurn
        }
    }
}

// vim: set tabstop=4 shiftwidth=4 softtabstop=4 expandtab:
