//! The hero: its body, input collection, movement over the terrain, top-down camera, and health.

use benilla_world::view::WorldCamera;
use benilla_world::world_point::WorldPoint;
use bevy::input::mouse::{AccumulatedMouseMotion, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

use super::data::{self, ClassDef};
use super::state::{PlayerHero, PlayerId, PlayerInput, PlayerInputs, PlayerState};
use super::units::{self, flat, wow_to_bevy, yaw_toward, Fx, Marks, UnitSpec};
use super::{Phase, Run, Selection};
use crate::creature_anim::{move_flags, MovementState};
use crate::net::{FieldChanged, Guid, ObjectStore};

#[derive(Component)]
pub(super) struct Hero {
    /// Vertical velocity in yards/second while airborne; 0 on the ground.
    pub(super) vy: f32,
    /// Height above the ground while airborne. Kept here, not in the transform: the creature
    /// ground clamp (`net::motion::ground_clamp_creatures`) re-seats every unit on the floor at
    /// the top of the frame, so the arc is re-applied over the ground each frame.
    pub(super) air: f32,
}

/// The body radius enemies stop at.
pub(super) const HERO_RADIUS: f32 = 0.7;

/// The hop's initial upward speed and the gravity that brings it back down — the reference
/// `CMovement` jump seed and gravity (`player/state.rs`'s `JUMP_SPEED` / `GRAVITY`).
const JUMP_SPEED: f32 = 7.955_547;
const GRAVITY: f32 = 19.291_105;

/// Radians of orbit per raw mouse unit while right-dragging.
const ORBIT_RATE: f32 = 0.008;
/// Elevation clamp (radians): ~10°..~83° above the horizon, so the camera never dives underground.
const PITCH_MIN: f32 = 0.18;
const PITCH_MAX: f32 = 1.45;
/// The locked view's tilt.
const FIXED_PITCH: f32 = 61.0 * std::f32::consts::PI / 180.0;
/// Wheel zoom range, yards from the hero's shoulders.
const ZOOM_MIN: f32 = 1.5;
const ZOOM_MAX: f32 = 70.0;

/// Whether the camera is locked; `C` toggles it.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum CamMode {
    /// Locked top-down (58°), laptop-friendly — the original feel.
    #[default]
    Fixed,
    /// Unlocked: a mouse drag swings and tilts it around the hero.
    Orbit,
}

#[derive(Resource)]
pub(super) struct CamRig {
    pub dist: f32,
    pub orbit: f32,
    /// Orbit azimuth (radians) around the hero; right-drag swings it during play.
    pub yaw: f32,
    /// Elevation above the horizon (radians), clamped so the camera never dives underground.
    pub pitch: f32,
    pub mode: CamMode,
}

impl Default for CamRig {
    fn default() -> Self {
        Self {
            dist: 30.0,
            orbit: 0.6,
            yaw: 0.0,
            pitch: FIXED_PITCH,
            mode: CamMode::Fixed,
        }
    }
}

/// Where a battleground's runs start.
pub(super) fn arena(map: &data::MapDef) -> Vec3 {
    wow_to_bevy(map.center)
}

/// The world is the battleground being stood in ([`Selection::stage`]): entering one on another
/// continent worldports, which raises the loading screen until the new ground is in.
pub(super) fn follow_map(
    selection: Res<Selection>,
    current: Option<ResMut<benilla_world::world_map::CurrentMap>>,
) {
    let Some(mut current) = current else {
        return;
    };
    let want = selection.stage_def().map;
    if current.0 != want {
        current.0 = want;
    }
}

/// The hero in the class trainer's look, weapons drawn.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_hero(
    commands: &mut Commands,
    player_id: PlayerId,
    class: &'static ClassDef,
    display: u32,
    race: u8,
    sex: u8,
    serial: u64,
    pos: Vec3,
) -> Entity {
    let bytes0 = u32::from(race)
        | u32::from(class.id) << 8
        | u32::from(sex) << 16
        | u32::from(class.power) << 24;
    let e = units::spawn_unit(
        commands,
        &UnitSpec {
            serial,
            display,
            scale: 1.0,
            level: 60,
            faction: units::FACTION_FRIENDLY,
            bytes0,
            weapons: class.weapons,
            sheath: class.sheath,
        },
        pos,
        0.0,
    );
    commands.entity(e).insert((
        Hero { vy: 0.0, air: 0.0 },
        PlayerHero(player_id),
        Marks::buffs(),
    ));
    e
}

/// Midday over the battleground, whatever the clock says.
pub(super) const MINUTE_OF_DAY: u32 = 780;

pub(super) fn daylight(debug: Option<ResMut<benilla_world::dev_state::DebugState>>) {
    let Some(mut debug) = debug else { return };
    if debug.lighting.follow_server_time || debug.lighting.manual_minute != MINUTE_OF_DAY {
        debug.lighting.follow_server_time = false;
        debug.lighting.manual_minute = MINUTE_OF_DAY;
    }
}

/// The ground under the mouse cursor, `None` with the cursor off the window. The locked camera's
/// hero faces it, and the aimed mobility spells (Blink, Charge) go toward it.
#[derive(Resource, Default)]
pub(super) struct AimPoint(pub(super) Option<Vec3>);

/// Cast the cursor onto the terrain: the camera ray meets the plane at the local hero's height,
/// then the terrain height there, a few times over, which settles on hills within a frame.
pub(super) fn aim_cursor(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cams: Query<(&Camera, &Transform), With<WorldCamera>>,
    run: Res<Run>,
    heroes: Query<(&Transform, &PlayerHero), (With<Hero>, Without<WorldCamera>)>,
    world: WorldPoint,
    mut aim: ResMut<AimPoint>,
) {
    let local_tf = heroes
        .iter()
        .find(|(_, ph)| ph.0 == run.local)
        .or_else(|| heroes.iter().next())
        .map(|(tf, _)| tf);
    let point = (|| {
        let cursor = windows.single().ok()?.cursor_position()?;
        let (cam, tf) = cams.single().ok()?;
        let ray = cam
            .viewport_to_world(&GlobalTransform::from(*tf), cursor)
            .ok()?;
        let mut y = local_tf?.translation.y;
        let mut at = None;
        for _ in 0..4 {
            if ray.direction.y.abs() < 1e-4 {
                break;
            }
            let t = (y - ray.origin.y) / ray.direction.y;
            if t <= 0.0 {
                break;
            }
            let p = ray.origin + *ray.direction * t;
            at = Some(p);
            match world.terrain_height_under(p) {
                Some(h) if (h - y).abs() > 0.05 => y = h,
                _ => break,
            }
        }
        at
    })();
    if aim.0 != point {
        aim.0 = point;
    }
}

/// Capture local keyboard, mouse, touch, or autopilot commands into [`PlayerInputs`] for
/// `run.local`, isolating device polling from hero movement and mobility systems.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_local_input(
    keys: Res<ButtonInput<KeyCode>>,
    phase: Res<State<Phase>>,
    run: Res<Run>,
    rig: Res<CamRig>,
    aim: Res<AimPoint>,
    auto: Option<Res<super::auto::Auto>>,
    #[cfg(target_os = "android")] touch: Res<super::touch::TouchControls>,
    heroes: Query<(&Transform, &PlayerHero), With<Hero>>,
    mut inputs: ResMut<PlayerInputs>,
) {
    let local_id = run.local;
    if *phase.get() != Phase::Playing || run.local().dead_t.is_some() {
        inputs.set(local_id, PlayerInput::default());
        return;
    }

    let hero_pos = heroes
        .iter()
        .find(|(_, ph)| ph.0 == local_id)
        .map(|(tf, _)| tf.translation);

    let mut dir = Vec3::ZERO;
    if let Some(auto) = auto.as_deref() {
        dir = super::auto::steer(auto);
    } else {
        if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
            dir.z -= 1.0;
        }
        if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
            dir.z += 1.0;
        }
        if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
            dir.x -= 1.0;
        }
        if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
            dir.x += 1.0;
        }
        if dir != Vec3::ZERO {
            dir = dir.normalize();
        }
        #[cfg(target_os = "android")]
        if touch.direction != Vec2::ZERO {
            let clamped = touch.direction.clamp_length_max(1.0);
            dir = Vec3::new(clamped.x, 0.0, clamped.y);
        }
        if rig.mode == CamMode::Orbit && dir != Vec3::ZERO {
            let (sy, cy) = rig.yaw.sin_cos();
            dir = Vec3::new(dir.x * cy + dir.z * sy, 0.0, dir.z * cy - dir.x * sy);
        }
    }

    let face = if rig.mode == CamMode::Fixed && auto.is_none() {
        #[cfg(target_os = "android")]
        let suppress_face = touch.direction != Vec2::ZERO;
        #[cfg(not(target_os = "android"))]
        let suppress_face = false;

        if suppress_face {
            None
        } else {
            hero_pos.and_then(|pos| {
                aim.0
                    .map(|a| flat(a - pos))
                    .filter(|d| d.length() > 0.5)
                    .map(|d| d.normalize())
            })
        }
    } else {
        None
    };

    let jump = keys.just_pressed(KeyCode::Space);
    #[cfg(target_os = "android")]
    let jump = jump || touch.jump;

    let mobility = keys.just_pressed(super::mobility::KEY);
    #[cfg(target_os = "android")]
    let mobility = mobility || touch.mobility;

    #[cfg(target_os = "android")]
    let mobility_aim = if touch.mobility {
        hero_pos.and_then(|pos| {
            Vec3::new(touch.direction.x, 0.0, touch.direction.y)
                .try_normalize()
                .map(|d| pos + d * 100.0)
        })
    } else {
        aim.0
    };
    #[cfg(not(target_os = "android"))]
    let mobility_aim = aim.0;

    inputs.set(
        local_id,
        PlayerInput {
            move_dir: Vec2::new(dir.x, dir.z),
            look_dir: face,
            jump,
            mobility,
            mobility_aim,
        },
    );
}

/// Drive every hero from [`PlayerInputs`] over the terrain within the battleground's leash,
/// and keep the streaming focus on the local hero.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_hero(
    time: Res<Time>,
    phase: Res<State<Phase>>,
    run: Res<Run>,
    inputs: Res<PlayerInputs>,
    selection: Res<Selection>,
    world: WorldPoint,
    mut player: ResMut<crate::player::Player>,
    mut heroes: Query<(&mut Transform, &mut MovementState, &mut Hero, &PlayerHero)>,
) {
    let dt = time.delta_secs();
    let in_menu = *phase.get() == Phase::Menu;
    let map = if in_menu {
        selection.stage_def()
    } else {
        run.match_state.config.map
    };
    let center = arena(map);
    let mut local_focus = None;

    for (mut tf, mut mv, mut hm, player_hero) in &mut heroes {
        let state = run.player(player_hero.0).unwrap_or_else(|| run.local());
        let playing = *phase.get() == Phase::Playing && state.dead_t.is_none();
        let input = if playing {
            inputs.get(player_hero.0)
        } else {
            PlayerInput::default()
        };

        let mut pos = tf.translation;
        let face = if playing { input.look_dir } else { None };
        if let Some(look) = face {
            let target = Quat::from_rotation_y(yaw_toward(look));
            tf.rotation = tf.rotation.slerp(target, (dt * 20.0).min(1.0));
        }

        let move_len = input.move_dir.length();
        if move_len > 1e-4 && dt > 0.0 {
            let dir = Vec3::new(
                input.move_dir.x / move_len,
                0.0,
                input.move_dir.y / move_len,
            );
            let speed = state.stats().speed * state.haste_mult() * move_len.min(1.0);
            pos += dir * speed * dt;
            mv.speed = speed;
            mv.flags = match face {
                Some(look) => gait(look, dir),
                None => {
                    let target = Quat::from_rotation_y(yaw_toward(dir));
                    tf.rotation = tf.rotation.slerp(target, (dt * 20.0).min(1.0));
                    move_flags::FORWARD
                }
            };
        } else if mv.speed != 0.0 || mv.flags != 0 {
            mv.speed = 0.0;
            mv.flags = 0;
        }

        let off = flat(pos - center);
        if off.length() > map.leash * 2.0 {
            pos = center;
        } else if off.length() > map.leash {
            let held = center + off.normalize() * map.leash;
            pos.x = held.x;
            pos.z = held.z;
        }
        let ground = world.terrain_height_under(pos).unwrap_or(pos.y);

        if playing && input.jump && hm.vy == 0.0 && hm.air == 0.0 {
            hm.vy = JUMP_SPEED;
        }
        if playing && (hm.vy != 0.0 || hm.air > 0.0) {
            hm.vy -= GRAVITY * dt;
            hm.air += hm.vy * dt;
            if hm.air <= 0.0 {
                hm.air = 0.0;
                hm.vy = 0.0;
            }
        } else {
            hm.air = 0.0;
            hm.vy = 0.0;
        }
        pos.y = ground + hm.air;
        mv.vertical_speed = hm.vy;
        if hm.vy != 0.0 {
            mv.flags |= move_flags::FALLING;
        }
        if tf.translation != pos {
            tf.translation = pos;
        }
        if player_hero.0 == run.local || local_focus.is_none() {
            local_focus = Some(pos);
        }
    }

    let focus = local_focus.unwrap_or_else(|| {
        let mut p = center;
        if let Some(h) = world.terrain_height_under(p) {
            p.y = h;
        }
        p
    });
    // The terrain streams and the loading screen waits on the ground under this.
    player.active = true;
    player.detached = false;
    player.settling = false;
    player.world_stale = false;
    player.pos = focus;
}

/// Let go of the mouse buttons when the page loses focus or the pointer leaves it.
pub(super) fn drop_stray_buttons(
    mut focus: MessageReader<bevy::window::WindowFocused>,
    mut left: MessageReader<bevy::window::CursorLeft>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    let lost = focus.read().any(|f| !f.focused);
    if lost | (left.read().count() > 0) {
        mouse.release_all();
    }
}

/// A slow orbit at the menu; in play, top-down until `C` unlocks it to swing and tilt.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_camera(
    time: Res<Time<Real>>,
    phase: Res<State<Phase>>,
    run: Res<Run>,
    mut rig: ResMut<CamRig>,
    mut wheel: MessageReader<MouseWheel>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    #[cfg(target_os = "android")] touch: Res<super::touch::TouchControls>,
    keys: Res<ButtonInput<KeyCode>>,
    player: Res<crate::player::Player>,
    world: WorldPoint,
    heroes: Query<(&Transform, &PlayerHero), (With<Hero>, Without<WorldCamera>)>,
    mut cams: Query<&mut Transform, With<WorldCamera>>,
) {
    if keys.just_pressed(KeyCode::KeyC) {
        rig.mode = match rig.mode {
            CamMode::Fixed => CamMode::Orbit,
            CamMode::Orbit => {
                rig.yaw = 0.0;
                rig.pitch = FIXED_PITCH;
                CamMode::Fixed
            }
        };
    }
    let focus = heroes
        .iter()
        .find(|(_, ph)| ph.0 == run.local)
        .or_else(|| heroes.iter().next())
        .map_or(player.pos, |(t, _)| t.translation);
    for w in wheel.read() {
        let notches = match w.unit {
            MouseScrollUnit::Line => w.y,
            MouseScrollUnit::Pixel => w.y * 0.02,
        };
        rig.dist = (rig.dist * (1.0 - 0.12 * notches).clamp(0.5, 1.5)).clamp(ZOOM_MIN, ZOOM_MAX);
    }
    #[cfg(target_os = "android")]
    if touch.zoom != 0.0 {
        rig.dist = (rig.dist * (1.0 - 0.12 * touch.zoom).clamp(0.5, 1.5)).clamp(ZOOM_MIN, ZOOM_MAX);
    }
    let (eye, look) = if *phase.get() == Phase::Menu {
        rig.orbit += time.delta_secs() * 0.2;
        let d = 7.0;
        (
            focus + Vec3::new(rig.orbit.sin() * d, 2.4, rig.orbit.cos() * d),
            focus + Vec3::Y * 1.1,
        )
    } else {
        if rig.mode == CamMode::Orbit && mouse.any_pressed([MouseButton::Left, MouseButton::Right])
        {
            rig.yaw -= motion.delta.x * ORBIT_RATE;
            rig.pitch = (rig.pitch + motion.delta.y * ORBIT_RATE).clamp(PITCH_MIN, PITCH_MAX);
        }
        let (sy, cy) = rig.yaw.sin_cos();
        let (sp, cp) = rig.pitch.sin_cos();
        let look = focus + Vec3::Y * 1.4;
        let mut eye = look + Vec3::new(cp * sy, sp, cp * cy) * rig.dist;
        if let Some(h) = world.terrain_height_under(eye) {
            eye.y = eye.y.max(h + 0.6);
        }
        (eye, look)
    };
    let placed = Transform::from_translation(eye).looking_at(look, Vec3::Y);
    for mut cam in &mut cams {
        *cam = placed;
    }
}

/// Damage a hero takes from one blow, after immunity, armour and the shield; returns what got through.
pub(super) fn hurt_hero(player: &mut PlayerState, raw: f32) -> f32 {
    if player.immune_t > 0.0 || player.dead_t.is_some() {
        return 0.0;
    }
    let mut dmg = (raw - player.stats().armor).max(1.0);
    if player.shield_t > 0.0 && player.shield > 0.0 {
        let absorbed = dmg.min(player.shield);
        player.shield -= absorbed;
        dmg -= absorbed;
    }
    player.hp -= dmg;
    dmg
}

/// The clock, regeneration, the timed buffs and their visuals, death and victory.
pub(super) fn hero_vitals(
    time: Res<Time>,
    mut run: ResMut<Run>,
    mut next: ResMut<NextState<Phase>>,
    mut heroes: Query<(Entity, &Guid, &mut ObjectStore, &mut Marks, &PlayerHero), With<Hero>>,
    mut fields: MessageWriter<FieldChanged>,
    mut fx: Fx,
) {
    let dt = time.delta_secs();
    if !run.any_alive() {
        let mut all_finished = !run.players.is_empty();
        for player in &mut run.players {
            if let Some(t) = player.dead_t.as_mut() {
                *t += dt;
                if *t <= 3.0 {
                    all_finished = false;
                }
            } else {
                all_finished = false;
            }
        }
        if all_finished {
            next.set(Phase::GameOver);
        }
        return;
    }

    run.match_state.elapsed += dt;
    let won_now = run.match_state.elapsed >= data::VICTORY_SECS;
    if won_now {
        run.match_state.won = true;
    }

    for player in &mut run.players {
        let Some((entity, guid, mut store, mut marks, _)) =
            heroes.iter_mut().find(|(_, _, _, _, ph)| ph.0 == player.id)
        else {
            continue;
        };
        if let Some(t) = player.dead_t.as_mut() {
            *t += dt;
            continue;
        }

        let stats = player.stats();
        for t in [
            &mut player.shield_t,
            &mut player.immune_t,
            &mut player.thorns_t,
        ] {
            *t = (*t - dt).max(0.0);
        }
        let mut heal = stats.regen * dt;
        if player.heal_t > 0.0 {
            heal += player.heal_rate * dt.min(player.heal_t);
            player.heal_t = (player.heal_t - dt).max(0.0);
        }
        player.hp = (player.hp + heal).min(stats.max_hp);

        let mut shown: Vec<u32> = Vec::new();
        if player.shield_t > 0.0 && player.shield > 0.0 {
            shown.push(17);
        }
        if player.immune_t > 0.0 {
            shown.push(642);
        }
        if player.thorns_t > 0.0 {
            shown.push(player.thorns_spell);
        }
        if player.heal_t > 0.0 {
            shown.push(774);
        }
        for owned in &player.abilities {
            if owned.def.key == "lightning_shield" {
                shown.push(owned.def.spell);
            }
        }
        if player.has_passive_key("renew") {
            shown.push(139);
        }
        marks.set(&shown);
        marks.flush(entity, guid.0, &mut store, &mut fields);

        if player.hp <= 0.0 {
            player.hp = 0.0;
            player.dead_t = Some(0.0);
            units::kill_unit(&mut store);
            marks.clear();
            marks.flush(entity, guid.0, &mut store, &mut fields);
        } else if won_now {
            fx.cast(entity, 642, &[entity], None);
        }
    }

    if won_now {
        next.set(Phase::GameOver);
    }
}

/// The movement flags for running `dir` while facing `look`: forward, backpedal or strafe.
fn gait(look: Vec3, dir: Vec3) -> u32 {
    let ahead = look.dot(dir);
    let left = look.cross(dir).y;
    let quarter = std::f32::consts::FRAC_1_SQRT_2;
    if ahead >= quarter {
        move_flags::FORWARD
    } else if ahead <= -quarter {
        move_flags::BACKWARD
    } else if left > 0.0 {
        move_flags::STRAFE_LEFT
    } else {
        move_flags::STRAFE_RIGHT
    }
}

pub(super) fn reach(hero: Vec3, other: Vec3) -> f32 {
    flat(other - hero).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::window::{CursorLeft, WindowFocused};

    fn held_left() -> World {
        let mut world = World::new();
        world.init_resource::<Messages<WindowFocused>>();
        world.init_resource::<Messages<CursorLeft>>();
        let mut mouse = ButtonInput::<MouseButton>::default();
        mouse.press(MouseButton::Left);
        world.insert_resource(mouse);
        world
    }

    fn left_held(world: &World) -> bool {
        world
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left)
    }

    #[test]
    fn a_held_button_is_dropped_when_the_page_loses_focus() {
        let mut world = held_left();
        world.run_system_once(drop_stray_buttons).unwrap();
        assert!(left_held(&world), "nothing happened: the drag goes on");
        world.write_message(WindowFocused {
            window: Entity::PLACEHOLDER,
            focused: true,
        });
        world.run_system_once(drop_stray_buttons).unwrap();
        assert!(left_held(&world), "gaining focus is no release");
        world.write_message(WindowFocused {
            window: Entity::PLACEHOLDER,
            focused: false,
        });
        world.run_system_once(drop_stray_buttons).unwrap();
        assert!(!left_held(&world));
    }

    #[test]
    fn a_held_button_is_dropped_when_the_pointer_leaves_the_page() {
        let mut world = held_left();
        world.write_message(CursorLeft {
            window: Entity::PLACEHOLDER,
        });
        world.run_system_once(drop_stray_buttons).unwrap();
        assert!(!left_held(&world));
    }
}
