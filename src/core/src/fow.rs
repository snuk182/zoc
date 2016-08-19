use std::default::{Default};
use types::{Size2};
use internal_state::{InternalState};
use game_state::{GameState};
use map::{Map, Terrain, distance};
use fov::{fov};
use db::{Db};
use unit::{Unit, UnitType, UnitClass};
use ::{CoreEvent, PlayerId, MapPos, ExactPos, ObjectClass};

#[derive(Clone, PartialEq, PartialOrd, Debug)]
pub enum TileVisibility {
    No,
    // Bad,
    Normal,
    Excellent,
}

impl Default for TileVisibility {
    fn default() -> Self { TileVisibility::No }
}

fn fov_unit(
    db: &Db,
    state: &InternalState,
    fow: &mut Map<TileVisibility>,
    unit: &Unit,
) {
    fov_unit_in_pos(db, state, fow, unit, &unit.pos.map_pos);
}

fn fov_unit_in_pos(
    db: &Db,
    state: &InternalState,
    fow: &mut Map<TileVisibility>,
    unit: &Unit,
    origin: &MapPos,
) {
    let unit_type = db.unit_type(&unit.type_id);
    let range = unit_type.los_range;
    fov(
        state,
        origin,
        range,
        &mut |pos| {
            let vis = calc_visibility(state, unit_type, *origin, *pos);
            if vis > *fow.tile_mut(pos) {
                *fow.tile_mut(pos) = vis;
            }
        },
    );
}

fn calc_visibility<S: GameState>(
    state: &S,
    unit_type: &UnitType,
    origin: MapPos,
    pos: MapPos,
) -> TileVisibility {
    let distance = distance(&origin, &pos);
    if distance > unit_type.los_range {
        return TileVisibility::No;
    }
    if distance <= unit_type.cover_los_range {
        return TileVisibility::Excellent;
    }
    let mut vis = match *state.map().tile(&pos) {
        Terrain::City | Terrain::Trees => TileVisibility::Normal,
        Terrain::Plain | Terrain::Water => TileVisibility::Excellent,
    };
    for object in state.objects_at(&pos) {
        match object.class {
            ObjectClass::Building | ObjectClass::Smoke => {
                vis = TileVisibility::Normal;
            }
            ObjectClass::Road => {},
        }
    }
    vis
}

/// Fog of War
pub struct Fow {
    map: Map<TileVisibility>,
    player_id: PlayerId,
}

impl Fow {
    pub fn new(map_size: &Size2, player_id: &PlayerId) -> Fow {
        Fow {
            map: Map::new(map_size),
            player_id: player_id.clone(),
        }
    }

    pub fn is_tile_visible(&self, pos: &MapPos) -> bool {
        // TODO: кстати, можно убрать тип поверхности "город"
        // и так же как и с дымом работать с ним
        match *self.map.tile(pos) {
            TileVisibility::Excellent |
            TileVisibility::Normal => true,
            TileVisibility::No => false,
        }
    }

    fn check_terrain_visibility(&self, unit_type: &UnitType, pos: &MapPos) -> bool {
        match *self.map.tile(pos) {
            TileVisibility::Excellent => true,
            TileVisibility::Normal => match unit_type.class {
                UnitClass::Infantry => false,
                UnitClass::Vehicle => true,
            },
            TileVisibility::No => false,
        }
    }

    pub fn is_visible(
        &self,
        db: &Db,
        state: &InternalState,
        unit: &Unit,
        pos: &ExactPos,
    ) -> bool {
        for other_unit in state.units().values() {
            if let Some(ref passenger_id) = other_unit.passenger_id {
                if *passenger_id == unit.id && other_unit.pos == *pos {
                    return false;
                }
            }
        }
        let unit_type = db.unit_type(&unit.type_id);
        self.check_terrain_visibility(unit_type, &pos.map_pos)
    }

    fn clear(&mut self) {
        for pos in self.map.get_iter() {
            *self.map.tile_mut(&pos) = TileVisibility::No;
        }
    }

    fn reset(&mut self, db: &Db, state: &InternalState) {
        self.clear();
        for unit in state.units().values() {
            if unit.player_id == self.player_id {
                fov_unit(db, state, &mut self.map, unit);
            }
        }
    }

    pub fn apply_event(
        &mut self,
        db: &Db,
        state: &InternalState,
        event: &CoreEvent,
    ) {
        match *event {
            CoreEvent::Move{ref unit_id, ref to, ..} => {
                let unit = state.unit(unit_id);
                if unit.player_id == self.player_id {
                    fov_unit_in_pos(
                        db, state, &mut self.map, unit, &to.map_pos);
                }
            },
            CoreEvent::EndTurn{ref new_id, ..} => {
                if self.player_id == *new_id {
                    self.reset(db, state);
                }
            },
            CoreEvent::CreateUnit{ref unit_info} => {
                let unit = state.unit(&unit_info.unit_id);
                if self.player_id == unit_info.player_id {
                    fov_unit(db, state, &mut self.map, unit);
                }
            },
            CoreEvent::AttackUnit{ref attack_info} => {
                if let Some(ref attacker_id) = attack_info.attacker_id {
                    if !attack_info.is_ambush {
                        let pos = &state.unit(attacker_id).pos;
                        // TODO: do not give away all units in this tile!
                        *self.map.tile_mut(pos) = TileVisibility::Excellent;
                    }
                }
            },
            CoreEvent::UnloadUnit{ref unit_info, ..} => {
                if self.player_id == unit_info.player_id {
                    let unit = state.unit(&unit_info.unit_id);
                    let pos = &unit_info.pos.map_pos;
                    fov_unit_in_pos(db, state, &mut self.map, unit, pos);
                }
            },
            CoreEvent::ShowUnit{..} |
            CoreEvent::HideUnit{..} |
            CoreEvent::LoadUnit{..} |
            CoreEvent::SetReactionFireMode{..} |
            CoreEvent::SectorOwnerChanged{..} |
            CoreEvent::Smoke{..} |
            CoreEvent::VictoryPoint{..} => {},
        }
    }
}
