extern crate cgmath;
extern crate rand;

pub mod geom;
pub mod map;
pub mod db;
pub mod unit;
pub mod dir;
pub mod partial_state;
pub mod game_state;
pub mod pathfinder;
pub mod misc;
pub mod types;

mod ai;
mod fov;
mod fow;
mod internal_state;
mod filter;

use rand::{thread_rng, Rng};
use std::{cmp, fmt};
use std::collections::{HashMap, HashSet, VecDeque};
use cgmath::{Vector2};
use types::{Size2};
use misc::{clamp};
use internal_state::{InternalState};
use game_state::{GameState, GameStateMut};
use partial_state::{PartialState};
use map::{Terrain, distance};
use pathfinder::{path_cost, tile_cost};
use unit::{Unit, UnitType, UnitTypeId, UnitClass};
use db::{Db};
use ai::{Ai};
use fow::{Fow};
use fov::{fov};
use dir::{Dir};

#[derive(Clone, Copy, Debug)]
pub struct Score{pub n: i32}

#[derive(Clone, Debug)]
pub struct MovePoints{pub n: i32}

#[derive(Clone, Debug)]
pub struct AttackPoints{pub n: i32}

#[derive(PartialOrd, PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct PlayerId{pub id: i32}

#[derive(PartialOrd, Ord, PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct UnitId{pub id: i32}

#[derive(PartialOrd, PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct SectorId{pub id: i32}

#[derive(PartialEq, Clone, Copy, Debug)]
pub struct MapPos{pub v: Vector2<i32>}

impl fmt::Display for MapPos {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MapPos({}, {})", self.v.x, self.v.y)
    }
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub enum SlotId {
    Id(u8),
    WholeTile,
    TwoTiles(Dir),
    // Air, // TODO: implement air units
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub struct ExactPos {
    pub map_pos: MapPos,
    pub slot_id: SlotId,
}

pub struct ExactPosIter<'a> {
    p: &'a ExactPos,
    i: u8,
}

impl ExactPos {
    pub fn map_pos_iter(&self) -> ExactPosIter {
        ExactPosIter {
            p: self,
            i: 0,
        }
    }
}

impl<'a> Iterator for ExactPosIter<'a> {
    type Item = MapPos;

    fn next(&mut self) -> Option<Self::Item> {
        let next_pos = match self.p.slot_id {
            SlotId::Id(_) | SlotId::WholeTile => {
                if self.i == 0 {
                    Some(self.p.map_pos.clone())
                } else {
                    None
                }
            }
            SlotId::TwoTiles(ref dir) => {
                if self.i == 0 {
                    Some(self.p.map_pos.clone())
                } else if self.i == 1 {
                    Some(Dir::get_neighbour_pos(&self.p.map_pos, dir))
                } else {
                    None
                }
            }
        };
        self.i += 1;
        next_pos
    }
}

// TODO: return iterator?
impl AsRef<MapPos> for ExactPos {
    fn as_ref(&self) -> &MapPos {
        &self.map_pos
    }
}

impl AsRef<MapPos> for MapPos {
    fn as_ref(&self) -> &MapPos {
        self
    }
}

#[derive(Clone, Debug)]
pub struct Sector {
    pub owner_id: Option<PlayerId>,
    pub positions: Vec<MapPos>,
}

impl Sector {
    pub fn center(&self) -> MapPos {
        let mut pos = Vector2{x: 0.0, y: 0.0};
        for sector_pos in &self.positions {
            pos.x += sector_pos.v.x as f32;
            pos.y += sector_pos.v.y as f32;
        }
        pos /= self.positions.len() as f32;
        let pos = MapPos{v: Vector2{
            x: (pos.x + 0.5) as i32,
            y: (pos.y + 0.5) as i32,
        }};
        assert!(self.positions.contains(&pos));
        pos
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum ObjectClass {
    Building,
    Road,
    Smoke,
}

#[derive(Debug, PartialOrd, Ord, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ObjectId {
    pub id: i32,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub pos: ExactPos,
    pub class: ObjectClass,
    pub timer: Option<i32>,
}

#[derive(PartialEq, Clone, Debug)]
pub enum PlayerClass {
    Human,
    Ai,
}

pub struct Player {
    pub id: PlayerId,
    pub class: PlayerClass,
}

#[derive(Clone, PartialEq, Debug)]
pub enum FireMode {
    Active,
    Reactive,
}

#[derive(Clone, PartialEq, Debug)]
pub enum ReactionFireMode {
    Normal,
    HoldFire,
}

#[derive(Clone, PartialEq, Debug)]
pub enum MoveMode {
    Fast,
    Hunt,
}

#[derive(PartialEq, Clone, Debug)]
pub enum Command {
    Move{unit_id: UnitId, path: Vec<ExactPos>, mode: MoveMode},
    EndTurn,
    CreateUnit{pos: ExactPos, type_id: UnitTypeId},
    AttackUnit{attacker_id: UnitId, defender_id: UnitId},
    LoadUnit{transporter_id: UnitId, passenger_id: UnitId},
    UnloadUnit{transporter_id: UnitId, passenger_id: UnitId, pos: ExactPos},
    SetReactionFireMode{unit_id: UnitId, mode: ReactionFireMode},
    Smoke{unit_id: UnitId, pos: MapPos},
}

#[derive(Clone, Debug)]
pub struct UnitInfo {
    pub unit_id: UnitId,
    pub pos: ExactPos,
    pub type_id: UnitTypeId,
    pub player_id: PlayerId,
    pub passenger_id: Option<UnitId>,
}

#[derive(Clone, Debug)]
pub struct AttackInfo {
    pub attacker_id: Option<UnitId>,
    pub defender_id: UnitId,
    pub mode: FireMode,
    pub killed: i32,
    pub suppression: i32,
    pub remove_move_points: bool,
    pub is_ambush: bool,
    pub is_inderect: bool,
}

#[derive(Clone, Debug)]
pub enum CoreEvent {
    Move {
        unit_id: UnitId,
        from: ExactPos,
        to: ExactPos,
        mode: MoveMode,
        cost: MovePoints,
    },
    EndTurn {
        old_id: PlayerId,
        new_id: PlayerId,
    },
    CreateUnit {
        unit_info: UnitInfo,
    },
    AttackUnit {
        attack_info: AttackInfo,
    },
    ShowUnit {
        unit_info: UnitInfo,
    },
    HideUnit {
        unit_id: UnitId,
    },
    LoadUnit {
        transporter_id: Option<UnitId>,
        passenger_id: UnitId,
        from: ExactPos,
        to: ExactPos,
    },
    UnloadUnit {
        unit_info: UnitInfo,
        transporter_id: Option<UnitId>,
        from: ExactPos,
        to: ExactPos,
    },
    SetReactionFireMode {
        unit_id: UnitId,
        mode: ReactionFireMode,
    },
    SectorOwnerChanged {
        sector_id: SectorId,
        new_owner_id: Option<PlayerId>,
    },
    VictoryPoint {
        player_id: PlayerId,
        pos: MapPos,
        count: i32,
    },
    // TODO: CreateObject
    Smoke {
        id: ObjectId,
        pos: MapPos,
        unit_id: Option<UnitId>,
    },
    // TODO: RemoveObject
    RemoveSmoke {
        id: ObjectId,
    },
}

pub const MAX_GROUND_SLOTS_COUNT: usize = 3;

pub fn move_cost_modifier(mode: &MoveMode) -> i32 {
    match *mode {
        MoveMode::Fast => 1,
        MoveMode::Hunt => 2,
    }
}

// TODO: simplify/optimize
pub fn find_next_player_unit_id<S: GameState>(
    state: &S,
    player_id: &PlayerId,
    unit_id: &UnitId,
) -> UnitId {
    let mut i = state.units().iter().cycle().filter(
        |&(_, unit)| unit.player_id == *player_id);
    while let Some((id, _)) = i.next() {
        if *id == *unit_id {
            let (id, _) = i.next().unwrap();
            return id.clone();
        }
    }
    unreachable!()
}

// TODO: simplify/optimize
pub fn find_prev_player_unit_id<S: GameState>(
    state: &S,
    player_id: &PlayerId,
    unit_id: &UnitId,
) -> UnitId {
    let mut i = state.units().iter().cycle().filter(
        |&(_, unit)| unit.player_id == *player_id).peekable();
    while let Some((id, _)) = i.next() {
        let &(next_id, _) = i.peek().unwrap();
        if *next_id == *unit_id {
            return id.clone();
        }
    }
    unreachable!()
}

pub fn get_unit_ids_at(db: &Db, state: &PartialState, pos: &MapPos) -> Vec<UnitId> {
    let units_at = state.units_at(pos);
    let mut hidden_ids = HashSet::new();
    for unit in &units_at {
        if db.unit_type(&unit.type_id).is_transporter {
            if let Some(ref passenger_id) = unit.passenger_id {
                hidden_ids.insert(passenger_id.clone());
            }
        }
    }
    let mut ids = Vec::new();
    for unit in &units_at {
        if !hidden_ids.contains(&unit.id) {
            ids.push(unit.id.clone())
        }
    }
    ids
}

pub fn unit_to_info(unit: &Unit) -> UnitInfo {
    UnitInfo {
        unit_id: unit.id.clone(),
        pos: unit.pos.clone(),
        type_id: unit.type_id.clone(),
        player_id: unit.player_id.clone(),
        passenger_id: unit.passenger_id.clone(),
    }
}

struct PlayerInfo {
    events: VecDeque<CoreEvent>,
    fow: Fow,
    visible_enemies: HashSet<UnitId>,
}

pub fn print_unit_info(db: &Db, unit: &Unit) {
    let unit_type = db.unit_type(&unit.type_id);
    let weapon_type = db.weapon_type(&unit_type.weapon_type_id);
    println!("unit:");
    println!("  player_id: {}", unit.player_id.id);
    println!("  move_points: {}", unit.move_points.n);
    println!("  attack_points: {}", unit.attack_points.n);
    if let Some(ref reactive_attack_points) = unit.reactive_attack_points {
        println!("  reactive_attack_points: {}", reactive_attack_points.n);
    } else {
        println!("  reactive_attack_points: ?");
    }
    println!("  count: {}", unit.count);
    println!("  morale: {}", unit.morale);
    println!("type:");
    println!("  name: {}", unit_type.name);
    match unit_type.class {
        UnitClass::Infantry => println!("  class: Infantry"),
        UnitClass::Vehicle => println!("  class: Vehicle"),
    }
    println!("  count: {}", unit_type.count);
    println!("  size: {}", unit_type.size);
    println!("  armor: {}", unit_type.armor);
    println!("  toughness: {}", unit_type.toughness);
    println!("  weapon_skill: {}", unit_type.weapon_skill);
    println!("  mp: {}", unit_type.move_points.n);
    println!("  ap: {}", unit_type.attack_points.n);
    println!("  reactive_ap: {}", unit_type.reactive_attack_points.n);
    println!("  los_range: {}", unit_type.los_range);
    println!("  cover_los_range: {}", unit_type.cover_los_range);
    println!("weapon:");
    println!("  name: {}", weapon_type.name);
    println!("  damage: {}", weapon_type.damage);
    println!("  ap: {}", weapon_type.ap);
    println!("  accuracy: {}", weapon_type.accuracy);
    println!("  max_distance: {}", weapon_type.max_distance);
}

pub fn print_terrain_info<S: GameState>(state: &S, pos: &MapPos) {
    match *state.map().tile(pos) {
        Terrain::City => println!("City"),
        Terrain::Trees => println!("Trees"),
        Terrain::Plain => println!("Plain"),
        Terrain::Water => println!("Water"),
    }
}

#[derive(Debug)]
pub enum CommandError {
    TileIsOccupied,
    NotEnoughMovePoints,
    NotEnoughAttackPoints,
    NotEnoughReactiveAttackPoints,
    BadMorale,
    OutOfRange,
    TooClose,
    NoLos,
    BadTransporterClass,
    BadPassengerClass,
    TransporterIsNotEmpty,
    TransporterIsEmpty,
    TransporterIsTooFarAway,
    PassengerHasNotEnoughMovePoints,
    UnloadDistanceIsTooBig,
    DestinationTileIsNotEmpty,
    BadUnitId,
    BadTransporterId,
    BadPassengerId,
    BadAttackerId,
    BadDefenderId,
    BadPath,
    BadUnitType,
}

impl CommandError {
    fn to_str(&self) -> &str {
        match *self {
            CommandError::TileIsOccupied => "Tile is occupied",
            CommandError::NotEnoughMovePoints => "Not enough move points",
            CommandError::NotEnoughAttackPoints => "No attack points",
            CommandError::NotEnoughReactiveAttackPoints => "No reactive attack points",
            CommandError::BadMorale => "Can`t attack when suppresset",
            CommandError::OutOfRange => "Out of range",
            CommandError::TooClose => "Too close",
            CommandError::NoLos => "No Line of Sight",
            CommandError::BadTransporterClass => "Bad transporter class",
            CommandError::BadPassengerClass => "Bad passenger class",
            CommandError::TransporterIsNotEmpty => "Transporter is not empty",
            CommandError::TransporterIsEmpty => "Transporter is empty",
            CommandError::TransporterIsTooFarAway => "Transporter is too far away",
            CommandError::PassengerHasNotEnoughMovePoints => "Passenger has not enough move points",
            CommandError::UnloadDistanceIsTooBig => "Unload pos it too far away",
            CommandError::DestinationTileIsNotEmpty => "Destination tile is not empty",
            CommandError::BadUnitId => "Bad unit id",
            CommandError::BadTransporterId => "Bad transporter id",
            CommandError::BadPassengerId => "Bad passenger id",
            CommandError::BadAttackerId => "Bad attacker id",
            CommandError::BadDefenderId => "Bad defender id",
            CommandError::BadPath => "Bad path",
            CommandError::BadUnitType => "Bad unit type",
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(self.to_str())
    }
}

impl std::error::Error for CommandError {
    fn description(&self) -> &str {
        self.to_str()
    }
}

fn check_attack<S: GameState>(
    db: &Db,
    state: &S,
    attacker: &Unit,
    defender: &Unit,
    fire_mode: &FireMode,
) -> Result<(), CommandError> {
    let reactive_attack_points = attacker
        .reactive_attack_points.as_ref().unwrap().clone();
    match *fire_mode {
        FireMode::Active => if attacker.attack_points.n <= 0 {
            return Err(CommandError::NotEnoughAttackPoints);
        },
        FireMode::Reactive => if reactive_attack_points.n <= 0 {
            return Err(CommandError::NotEnoughReactiveAttackPoints);
        },
    }
    let minimal_ok_morale = 50;
    if attacker.morale < minimal_ok_morale {
        return Err(CommandError::BadMorale);
    }
    let attacker_type = db.unit_type(&attacker.type_id);
    let weapon_type = db.weapon_type(&attacker_type.weapon_type_id);
    if distance(&attacker.pos.map_pos, &defender.pos.map_pos) > weapon_type.max_distance {
        return Err(CommandError::OutOfRange);
    }
    if distance(&attacker.pos.map_pos, &defender.pos.map_pos) < weapon_type.min_distance {
        return Err(CommandError::TooClose);
    }
    let is_los_ok = los(
        state,
        attacker_type,
        &attacker.pos.map_pos,
        &defender.pos.map_pos,
    );
    if !weapon_type.is_inderect && !is_los_ok {
        return Err(CommandError::NoLos);
    }
    Ok(())
}

pub fn check_command<S: GameState>(
    db: &Db,
    state: &S,
    command: &Command,
) -> Result<(), CommandError> {
    match *command {
        Command::EndTurn => Ok(()),
        Command::CreateUnit{ref pos, ref type_id} => {
            if is_exact_pos_free(db, state, type_id, pos) {
                Ok(())
            } else {
                Err(CommandError::TileIsOccupied)
            }
        },
        Command::Move{ref unit_id, ref path, ref mode} => {
            if path.len() < 2 {
                return Err(CommandError::BadPath);
            }
            if state.units().get(unit_id).is_none() {
                return Err(CommandError::BadUnitId);
            }
            let unit = state.unit(unit_id);
            for window in path.windows(2) {
                let pos = &window[1];
                if !is_exact_pos_free(db, state, &unit.type_id, pos) {
                    return Err(CommandError::BadPath);
                }
            }
            let cost = path_cost(db, state, unit, path).n
                * move_cost_modifier(mode);
            if cost > unit.move_points.n {
                return Err(CommandError::NotEnoughMovePoints);
            }
            Ok(())
        },
        Command::AttackUnit{ref attacker_id, ref defender_id} => {
            if state.units().get(attacker_id).is_none() {
                return Err(CommandError::BadAttackerId);
            }
            if state.units().get(defender_id).is_none() {
                return Err(CommandError::BadDefenderId);
            }
            let attacker = state.unit(attacker_id);
            let defender = state.unit(defender_id);
            check_attack(db, state, attacker, defender, &FireMode::Active)
        },
        Command::LoadUnit{ref transporter_id, ref passenger_id} => {
            if state.units().get(transporter_id).is_none() {
                return Err(CommandError::BadTransporterId);
            }
            if state.units().get(passenger_id).is_none() {
                return Err(CommandError::BadPassengerId);
            }
            let passenger = state.unit(passenger_id);
            let transporter = state.unit(transporter_id);
            if !db.unit_type(&transporter.type_id).is_transporter {
                return Err(CommandError::BadTransporterClass);
            }
            match db.unit_type(&passenger.type_id).class {
                UnitClass::Infantry => {},
                _ => {
                    return Err(CommandError::BadPassengerClass);
                }
            }
            if transporter.passenger_id.is_some() {
                return Err(CommandError::TransporterIsNotEmpty);
            }
            if distance(&transporter.pos.map_pos, &passenger.pos.map_pos) > 1 {
                return Err(CommandError::TransporterIsTooFarAway);
            }
            // TODO: 0 -> real move cost of transport tile for passenger
            if passenger.move_points.n == 0 {
                return Err(CommandError::PassengerHasNotEnoughMovePoints);
            }
            Ok(())
        },
        Command::UnloadUnit{ref transporter_id, ref passenger_id, ref pos} => {
            if state.units().get(transporter_id).is_none() {
                return Err(CommandError::BadTransporterId);
            }
            let passenger = match state.units().get(passenger_id) {
                Some(passenger) => passenger,
                None => return Err(CommandError::BadPassengerId),
            };
            let transporter = state.unit(transporter_id);
            if !db.unit_type(&transporter.type_id).is_transporter {
                return Err(CommandError::BadTransporterClass);
            }
            if distance(&transporter.pos.map_pos, &pos.map_pos) > 1 {
                return Err(CommandError::UnloadDistanceIsTooBig);
            }
            if transporter.passenger_id.is_none() {
                return Err(CommandError::TransporterIsEmpty);
            }
            if !is_exact_pos_free(db, state, &passenger.type_id, pos) {
                return Err(CommandError::DestinationTileIsNotEmpty);
            }
            // TODO: check that tile is walkable for passenger
            Ok(())
        },
        Command::SetReactionFireMode{ref unit_id, ..} => {
            if state.units().get(unit_id).is_none() {
                Err(CommandError::BadUnitId)
            } else {
                Ok(())
            }
        },
        Command::Smoke{unit_id, pos} => {
            let unit = match state.units().get(&unit_id) {
                Some(unit) => unit,
                None => return Err(CommandError::BadUnitId),
            };
            let unit_type = db.unit_type(&unit.type_id);
            let weapon_type = db.weapon_type(&unit_type.weapon_type_id);
            if !weapon_type.smoke.is_some() {
                return Err(CommandError::BadUnitType);
            }
            if distance(&unit.pos.map_pos, &pos) > weapon_type.max_distance {
                return Err(CommandError::OutOfRange);
            }
            if unit.attack_points.n != unit_type.attack_points.n {
                return Err(CommandError::NotEnoughAttackPoints);
            }
            Ok(())
        },
    }
}

fn check_sectors(state: &InternalState) -> Vec<CoreEvent> {
    let mut events = Vec::new();
    for (sector_id, sector) in state.sectors() {
        let mut claimers = HashSet::new();
        for pos in &sector.positions {
            for unit in state.units_at(pos) {
                claimers.insert(unit.player_id.clone());
            }
        }
        let owner_id = if claimers.len() != 1 {
            None
        } else {
            Some(claimers.into_iter().next().unwrap())
        };
        if sector.owner_id != owner_id {
            events.push(CoreEvent::SectorOwnerChanged {
                sector_id: sector_id.clone(),
                new_owner_id: owner_id,
            });
        }
    }
    events
}

#[derive(PartialEq, Clone, Debug)]
enum ReactionFireResult {
    Attacked,
    Killed,
    None,
}

#[derive(PartialEq, Eq, Debug)]
pub enum GameType {
    Hotseat,
    SingleVsAi,
}

impl Default for GameType {
    fn default() -> GameType {
        GameType::Hotseat
    }
}

#[derive(Default, Debug)]
pub struct Options {
    pub game_type: GameType,
}

pub struct Core {
    state: InternalState,
    players: Vec<Player>,
    current_player_id: PlayerId,
    db: Db,
    ai: Ai,
    players_info: HashMap<PlayerId, PlayerInfo>,
    next_unit_id: UnitId,
}

fn get_players_list(game_type: &GameType) -> Vec<Player> {
    vec!(
        Player {
            id: PlayerId{id: 0},
            class: PlayerClass::Human,
        },
        Player {
            id: PlayerId{id: 1},
            class: match *game_type {
                GameType::SingleVsAi => PlayerClass::Ai,
                GameType::Hotseat => PlayerClass::Human,
            },
        },
    )
}

fn get_player_info_lists(map_size: &Size2) -> HashMap<PlayerId, PlayerInfo> {
    let mut map = HashMap::new();
    map.insert(PlayerId{id: 0}, PlayerInfo {
        fow: Fow::new(map_size, &PlayerId{id: 0}),
        events: VecDeque::new(),
        visible_enemies: HashSet::new(),
    });
    map.insert(PlayerId{id: 1}, PlayerInfo {
        fow: Fow::new(map_size, &PlayerId{id: 1}),
        events: VecDeque::new(),
        visible_enemies: HashSet::new(),
    });
    map
}

fn los<S: GameState>(
    state: &S,
    unit_type: &UnitType,
    from: &MapPos,
    to: &MapPos,
) -> bool {
    // TODO: profile and optimize!
    let mut v = false;
    let range = unit_type.los_range;
    fov(state, from, range, &mut |p| if *p == *to { v = true });
    v
}

pub fn get_free_slot_for_building<S: GameState>(
    state: &S,
    pos: &MapPos,
) -> Option<SlotId> {
    let objects_at = state.objects_at(pos);
    let mut slots = [false, false, false];
    for object in &objects_at {
        if let SlotId::Id(slot_id) = object.pos.slot_id {
            slots[slot_id as usize] = true;
        } else {
            return None;
        }
    }
    for (i, slot) in slots.iter().enumerate().take(MAX_GROUND_SLOTS_COUNT) {
        if !slot {
            return Some(SlotId::Id(i as u8));
        }
    }
    None
}

pub fn get_free_exact_pos<S: GameState>(
    db: &Db,
    state: &S,
    type_id: &UnitTypeId,
    pos: &MapPos,
) -> Option<ExactPos> {
    let slot_id = match get_free_slot_id(db, state, type_id, pos) {
        Some(id) => id,
        None => return None,
    };
    Some(ExactPos{map_pos: pos.clone(), slot_id: slot_id})
}

pub fn get_free_slot_id<S: GameState>(
    db: &Db,
    state: &S,
    type_id: &UnitTypeId,
    pos: &MapPos,
) -> Option<SlotId> {
    let objects_at = state.objects_at(pos);
    let units_at = state.units_at(pos);
    let unit_type = db.unit_type(type_id);
    if unit_type.is_big {
        for object in &objects_at {
            match object.class {
                ObjectClass::Building => return None,
                ObjectClass::Smoke | ObjectClass::Road => {},
            }
        }
        if units_at.is_empty() {
            return Some(SlotId::WholeTile);
        } else {
            return None;
        }
    }
    let mut slots = [false, false, false];
    for unit in &units_at {
        match unit.pos.slot_id {
            SlotId::Id(slot_id) => slots[slot_id as usize] = true,
            SlotId::WholeTile | SlotId::TwoTiles(_) => return None,
        }
    }
    if unit_type.class == UnitClass::Vehicle {
        for object in &objects_at {
            match object.pos.slot_id {
                SlotId::Id(slot_id) => {
                    slots[slot_id as usize] = true;
                },
                SlotId::WholeTile => {
                    match object.class {
                        ObjectClass::Building => return None,
                        ObjectClass::Smoke | ObjectClass::Road => {},
                    }
                }
                SlotId::TwoTiles(_) => {},
            }
        }
    }
    for (i, slot) in slots.iter().enumerate().take(MAX_GROUND_SLOTS_COUNT) {
        if !slot {
            return Some(SlotId::Id(i as u8));
        }
    }
    None
}

// TODO: join logic with get_free_slot_id
pub fn is_exact_pos_free<S: GameState>(
    db: &Db,
    state: &S,
    type_id: &UnitTypeId,
    pos: &ExactPos,
) -> bool {
    let units_at = state.units_at(&pos.map_pos);
    if db.unit_type(type_id).is_big {
        return units_at.is_empty();
    }
    for unit in &units_at {
        match &unit.pos.slot_id {
            slot_id @ &SlotId::Id(_) => {
                if *slot_id == pos.slot_id {
                    return false;
                }
            }
            &SlotId::WholeTile | &SlotId::TwoTiles(_) => return false,
        }
    }
    true
}

impl Core {
    pub fn new(options: &Options) -> Core {
        let map_size = Size2{w: 10, h: 12}; // TODO: read from config file
        let mut core = Core {
            state: InternalState::new(&map_size),
            players: get_players_list(&options.game_type),
            current_player_id: PlayerId{id: 0},
            db: Db::new(),
            ai: Ai::new(&PlayerId{id:1}, &map_size),
            players_info: get_player_info_lists(&map_size),
            next_unit_id: UnitId{id: 0},
        };
        core.get_units();
        core
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    // TODO: Move to scenario.json
    fn get_units(&mut self) {
        for &(player_id, (x, y), type_name) in &[
            (0, (0, 1), "medium_tank"),
            (0, (0, 4), "mammoth_tank"),
            (0, (0, 5), "heavy_tank"),
            (0, (0, 5), "medium_tank"),
            (0, (1, 3), "truck"),
            (0, (1, 3), "mortar"),
            (0, (1, 4), "jeep"),
            (0, (2, 2), "soldier"),
            (0, (2, 2), "scout"),
            (0, (2, 4), "smg"),
            (0, (2, 4), "smg"),
            (1, (9, 1), "medium_tank"),
            (1, (9, 2), "soldier"),
            (1, (9, 2), "soldier"),
            (1, (9, 4), "soldier"),
            (1, (9, 5), "light_tank"),
            (1, (9, 5), "light_tank"),
            (1, (9, 6), "light_spg"),
            (1, (8, 2), "field_gun"),
            (1, (8, 4), "field_gun"),
            (1, (5, 10), "field_gun"),
            (1, (5, 10), "soldier"),
        ] {
            let pos = MapPos{v: Vector2{x: x, y: y}};
            let unit_type_id = self.db.unit_type_id(type_name);
            self.add_unit(&pos, &unit_type_id, &PlayerId{id: player_id});
        }
    }

    fn get_new_unit_id(&mut self) -> UnitId {
        let new_unit_id = self.next_unit_id;
        self.next_unit_id.id += 1;
        new_unit_id
    }

    fn get_new_object_id(&mut self) -> ObjectId {
        let mut next_id = match self.state.objects().keys().max() {
            Some(id) => *id,
            None => ObjectId{id: 0},
        };
        next_id.id += 1;
        next_id
    }

    fn add_unit(&mut self, pos: &MapPos, type_id: &UnitTypeId, player_id: &PlayerId) {
        let new_unit_id = self.get_new_unit_id();
        let pos = get_free_exact_pos(&self.db, &self.state, type_id, pos).unwrap();
        let event = CoreEvent::CreateUnit {
            unit_info: UnitInfo {
                unit_id: new_unit_id,
                pos: pos,
                type_id: type_id.clone(),
                player_id: player_id.clone(),
                passenger_id: None,
            },
        };
        self.do_core_event(&event);
    }

    pub fn map_size(&self) -> &Size2 {
        self.state.map().size()
    }

    fn get_killed_count(&self, attacker: &Unit, defender: &Unit) -> i32 {
        let hit = self.attack_test(attacker, defender);
        if !hit {
            return 0;
        }
        let defender_type = self.db.unit_type(&defender.type_id);
        match defender_type.class {
            UnitClass::Infantry => {
                clamp(thread_rng().gen_range(1, 5), 1, defender.count)
            },
            UnitClass::Vehicle => 1,
        }
    }

    fn cover_bonus(&self, defender: &Unit) -> i32 {
        let defender_type = self.db.unit_type(&defender.type_id);
        if defender_type.class == UnitClass::Infantry {
            match *self.state.map().tile(&defender.pos) {
                Terrain::Plain | Terrain::Water => 0,
                Terrain::Trees => 2,
                Terrain::City => 3,
            }
        } else {
            0
        }
    }

    // TODO: i32 -> HitChance
    pub fn hit_chance(&self, attacker: &Unit, defender: &Unit) -> i32 {
        let attacker_type = self.db.unit_type(&attacker.type_id);
        let defender_type = self.db.unit_type(&defender.type_id);
        let weapon_type = self.db.weapon_type(&attacker_type.weapon_type_id);
        let cover_bonus = self.cover_bonus(defender);
        let hit_test_v = -7 - cover_bonus + defender_type.size
            + weapon_type.accuracy + attacker_type.weapon_skill;
        let pierce_test_v = 10 + -defender_type.armor + weapon_type.ap;
        let wound_test_v = 5 -defender_type.toughness + weapon_type.damage;
        let hit_test_v = clamp(hit_test_v, 0, 10);
        let pierce_test_v = clamp(pierce_test_v, 0, 10);
        let wound_test_v = clamp(wound_test_v, 0, 10);
        let k = (hit_test_v * pierce_test_v * wound_test_v) / 10;
        assert!(0 <= k);
        assert!(100 > k);
        k
    }

    fn attack_test(&self, attacker: &Unit, defender: &Unit) -> bool {
        let k = self.hit_chance(attacker, defender);
        let r = thread_rng().gen_range(0, 100);
        r < k
    }

    pub fn player(&self) -> &Player {
        &self.players[self.player_id().id as usize]
    }

    pub fn player_id(&self) -> &PlayerId {
        &self.current_player_id
    }

    pub fn get_event(&mut self) -> Option<CoreEvent> {
        let mut i = self.players_info.get_mut(&self.current_player_id)
            .expect("core: Can`t get current player`s info");
        i.events.pop_front()
    }

    fn command_attack_unit_to_event(
        &self,
        attacker_id: &UnitId,
        defender_id: &UnitId,
        fire_mode: &FireMode,
    ) -> Option<CoreEvent> {
        let attacker = self.state.unit(attacker_id);
        let defender = self.state.unit(defender_id);
        let check_attack_result = check_attack(
            &self.db,
            &self.state,
            attacker,
            defender,
            fire_mode,
        );
        if check_attack_result.is_err() {
            return None;
        }
        let attacker_type = self.db.unit_type(&attacker.type_id);
        let weapon_type = self.db.weapon_type(&attacker_type.weapon_type_id);
        let killed = cmp::min(
            defender.count, self.get_killed_count(attacker, defender));
        let fow = &self.players_info[&defender.player_id].fow;
        let is_visible = fow.is_visible(
            &self.db, &self.state, attacker, &attacker.pos);
        let ambush_chance = 70;
        let is_ambush = !is_visible
            && thread_rng().gen_range(1, 100) <= ambush_chance;
        let base_suppression = 10;
        let per_death_suppression = 20;
        let attack_info = AttackInfo {
            attacker_id: Some(attacker_id.clone()),
            defender_id: defender_id.clone(),
            killed: killed,
            mode: fire_mode.clone(),
            suppression: base_suppression + per_death_suppression * killed,
            remove_move_points: false,
            is_ambush: is_ambush,
            is_inderect: weapon_type.is_inderect,
        };
        Some(CoreEvent::AttackUnit{attack_info: attack_info})
    }

    fn can_unit_make_reaction_attack(
        &self,
        defender: &Unit,
        attacker: &Unit,
    ) -> bool {
        assert!(attacker.player_id != defender.player_id);
        if attacker.reaction_fire_mode == ReactionFireMode::HoldFire {
            return false;
        }
        // TODO: move to `check_attack`
        let fow = &self.players_info[&attacker.player_id].fow;
        if !fow.is_visible(&self.db, &self.state, defender, &defender.pos) {
            return false;
        }
        let check_attack_result = check_attack(
            &self.db,
            &self.state,
            attacker,
            defender,
            &FireMode::Reactive,
        );
        check_attack_result.is_ok()
    }

    fn reaction_fire_internal(&mut self, unit_id: &UnitId, stop_on_attack: bool) -> ReactionFireResult {
        let unit_ids: Vec<_> = self.state.units().keys().cloned().collect();
        let mut result = ReactionFireResult::None;
        for enemy_unit_id in unit_ids {
            let event = {
                let enemy_unit = self.state.unit(&enemy_unit_id);
                let unit = self.state.unit(unit_id);
                if enemy_unit.player_id == unit.player_id {
                    continue;
                }
                if !self.can_unit_make_reaction_attack(unit, enemy_unit) {
                    continue;
                }
                let event = self.command_attack_unit_to_event(
                    &enemy_unit.id, unit_id, &FireMode::Reactive);
                if let Some(CoreEvent::AttackUnit{mut attack_info}) = event {
                    let hit_chance = self.hit_chance(enemy_unit, unit);
                    if hit_chance > 15 && stop_on_attack {
                        attack_info.remove_move_points = true;
                    }
                    CoreEvent::AttackUnit{attack_info: attack_info}
                } else {
                    continue;
                }
            };
            self.do_core_event(&event);
            result = ReactionFireResult::Attacked;
            if self.state.units().get(unit_id).is_none() {
                return ReactionFireResult::Killed;
            }
        }
        result
    }

    fn reaction_fire(&mut self, unit_id: &UnitId) {
        self.reaction_fire_internal(unit_id, false);
    }

    pub fn next_player_id(&self, id: &PlayerId) -> PlayerId {
        let old_id = id.id;
        let max_id = self.players.len() as i32;
        PlayerId{id: if old_id + 1 == max_id {
            0
        } else {
            old_id + 1
        }}
    }

    fn simulation_step(&mut self, command: Command) {
        if let Err(err) = check_command(&self.db, &self.state, &command) {
            println!("Bad command: {:?}", err);
            return;
        }
        match command {
            Command::EndTurn => {
                let old_id = self.current_player_id.clone();
                let new_id = self.next_player_id(&old_id);
                // TODO: extruct func
                let mut end_turn_events = Vec::new();
                for (_, sector) in self.state.sectors() {
                    if let Some(player_id) = sector.owner_id.clone() {
                        if player_id != new_id {
                            continue;
                        }
                        end_turn_events.push(CoreEvent::VictoryPoint {
                            player_id: player_id.clone(),
                            pos: sector.center(),
                            count: 1,
                        });
                    }
                }
                for (object_id, object) in self.state.objects() {
                    if let Some(timer) = object.timer {
                        if timer == 0 {
                            end_turn_events.push(CoreEvent::RemoveSmoke {
                                id: *object_id,
                            });
                        }
                    }
                }
                for event in end_turn_events {
                    self.do_core_event(&event);
                }
                self.do_core_event(&CoreEvent::EndTurn {
                    old_id: old_id,
                    new_id: new_id,
                });
            },
            Command::CreateUnit{pos, type_id} => {
                let event = CoreEvent::CreateUnit {
                    unit_info: UnitInfo {
                        unit_id: self.get_new_unit_id(),
                        pos: pos,
                        type_id: type_id,
                        player_id: self.current_player_id.clone(),
                        passenger_id: None,
                    },
                };
                self.do_core_event(&event);
            },
            Command::Move{unit_id, path, mode} => {
                let player_id = self.state.unit(&unit_id).player_id.clone();
                for window in path.windows(2) {
                    let from = &window[0];
                    let to = &window[1];
                    let event = {
                        let unit = self.state.unit(&unit_id);
                        let cost = MovePoints {
                            n: tile_cost(&self.db, &self.state, unit, from, to).n
                                * move_cost_modifier(&mode)
                        };
                        CoreEvent::Move {
                            unit_id: unit_id.clone(),
                            from: from.clone(),
                            to: to.clone(),
                            mode: mode.clone(),
                            cost: cost,
                        }
                    };
                    let pre_visible_enemies = self.players_info[&player_id]
                        .visible_enemies.clone();
                    self.do_core_event(&event);
                    let reaction_fire_result = self.reaction_fire_internal(
                        &unit_id, mode == MoveMode::Fast);
                    if reaction_fire_result != ReactionFireResult::None {
                        break;
                    }
                    let i = &self.players_info[&player_id];
                    if pre_visible_enemies != i.visible_enemies {
                        break;
                    }
                }
            },
            Command::AttackUnit{ref attacker_id, ref defender_id} => {
                if let Some(ref event) = self.command_attack_unit_to_event(
                    attacker_id, defender_id, &FireMode::Active)
                {
                    self.do_core_event(event);
                    self.reaction_fire(attacker_id);
                }
            },
            Command::LoadUnit{transporter_id, passenger_id} => {
                let from = self.state.unit(&passenger_id).pos.clone();
                let to = self.state.unit(&transporter_id).pos.clone();
                self.do_core_event(&CoreEvent::LoadUnit {
                    transporter_id: Some(transporter_id),
                    passenger_id: passenger_id,
                    from: from,
                    to: to,
                });
            },
            Command::UnloadUnit{transporter_id, passenger_id, pos} => {
                let event = {
                    let passenger = self.state.unit(&passenger_id);
                    let from = self.state.unit(&transporter_id).pos.clone();
                    CoreEvent::UnloadUnit {
                        transporter_id: Some(transporter_id),
                        unit_info: UnitInfo {
                            pos: pos.clone(),
                            .. unit_to_info(passenger)
                        },
                        from: from,
                        to: pos.clone(),
                    }
                };
                self.do_core_event(&event);
                self.reaction_fire(&passenger_id);
            },
            Command::SetReactionFireMode{unit_id, mode} => {
                self.do_core_event(&CoreEvent::SetReactionFireMode {
                    unit_id: unit_id,
                    mode: mode,
                });
            },
            Command::Smoke{unit_id, pos} => {
                let id = self.get_new_object_id();
                self.do_core_event(&CoreEvent::Smoke {
                    id: id,
                    unit_id: Some(unit_id),
                    pos: pos,
                });
                let mut dir = Dir::from_int(thread_rng().gen_range(0, 5));
                let additional_smoke_count = {
                    let unit = self.state.unit(&unit_id);
                    let unit_type = self.db.unit_type(&unit.type_id);
                    let weapon_type = self.db.weapon_type(&unit_type.weapon_type_id);
                    weapon_type.smoke.unwrap()
                };
                assert!(additional_smoke_count <= 3);
                for _ in 0..additional_smoke_count {
                    let mut dir_index = dir.to_int() + thread_rng().gen_range(1, 3);
                    if dir_index > 5 {
                        dir_index -= 6;
                    }
                    dir = Dir::from_int(dir_index);
                    let id = self.get_new_object_id();
                    self.do_core_event(&CoreEvent::Smoke {
                        id: id,
                        unit_id: Some(unit_id),
                        pos: Dir::get_neighbour_pos(&pos, &dir),
                    });
                }
                self.reaction_fire(&unit_id);
            },
        };
        let sector_events = check_sectors(&self.state);
        for event in sector_events {
            self.do_core_event(&event);
        }
    }

    pub fn do_command(&mut self, command: Command) {
        self.simulation_step(command);
    }

    fn do_ai(&mut self) {
        loop {
            while let Some(event) = self.get_event() {
                self.ai.apply_event(&self.db, &event);
            }
            let command = self.ai.get_command(&self.db);
            self.do_command(command.clone());
            if command == Command::EndTurn {
                return;
            }
        }
    }

    fn handle_end_turn_event(&mut self, old_id: &PlayerId, new_id: &PlayerId) {
        for player in &self.players {
            if player.id == *new_id {
                if self.current_player_id == *old_id {
                    self.current_player_id = player.id.clone();
                }
                break;
            }
        }
        if self.player().class == PlayerClass::Ai
            && *new_id == *self.player_id()
        {
            self.do_ai();
        }
    }

    fn do_core_event(&mut self, event: &CoreEvent) {
        self.state.apply_event(&self.db, event);
        for player in &self.players {
            let (filtered_events, active_unit_ids) = filter::filter_events(
                &self.db,
                &self.state,
                &player.id,
                &self.players_info[&player.id].fow,
                event,
            );
            let mut i = self.players_info.get_mut(&player.id)
                .expect("core: Can`t get player`s info");
            for event in filtered_events {
                i.fow.apply_event(&self.db, &self.state, &event);
                i.events.push_back(event);
                let new_visible_enemies = filter::get_visible_enemies(
                    &self.db,
                    &self.state,
                    &i.fow,
                    &player.id,
                );
                let show_hide_events = filter::show_or_hide_passive_enemies(
                    self.state.units(),
                    &active_unit_ids,
                    &i.visible_enemies,
                    &new_visible_enemies,
                );
                i.events.extend(show_hide_events);
                i.visible_enemies = new_visible_enemies;
            }
        }
        if let CoreEvent::EndTurn{ref old_id, ref new_id} = *event {
            self.handle_end_turn_event(old_id, new_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use cgmath::{Vector2};
    use ::{Sector, MapPos};

    #[test]
    fn test_center_1() {
        let real = Sector {
            positions: vec![
                MapPos{v: Vector2{x: 5, y: 0}},
                MapPos{v: Vector2{x: 6, y: 0}},
                MapPos{v: Vector2{x: 5, y: 1}},
                MapPos{v: Vector2{x: 6, y: 1}},
                MapPos{v: Vector2{x: 7, y: 1}},
                MapPos{v: Vector2{x: 5, y: 2}},
                MapPos{v: Vector2{x: 6, y: 2}},
            ],
            owner_id: None,
        }.center();
        let expected = MapPos{v: Vector2{x: 6, y: 1}};
        assert_eq!(expected, real);
    }

    #[test]
    fn test_center_2() {
        let real = Sector {
            positions: vec![
                MapPos{v: Vector2{x: 6, y: 0}},
                MapPos{v: Vector2{x: 6, y: 1}},
                MapPos{v: Vector2{x: 6, y: 2}},
            ],
            owner_id: None,
        }.center();
        let expected = MapPos{v: Vector2{x: 6, y: 1}};
        assert_eq!(expected, real);
    }
}
